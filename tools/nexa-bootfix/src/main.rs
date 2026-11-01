//! nexa-bootfix: find why Windows won't start on this PC, fix what can be
//! fixed safely, and save a report (and a screenshot) to a USB drive.
//!
//!   doas nexa-bootfix              step by step, in a terminal
//!   nexa-bootfix check [--json]    what's wrong (read-only)
//!   nexa-bootfix fix --yes [--to DIR]  back up, then apply the safe fixes
//!   nexa-bootfix drives            USB drives the report can be saved to
//!   nexa-bootfix screenshot FILE   the desktop as a PNG (the step-by-step run saves one)
//!
//! It looks at: the internal disk's partitions (GPT), the EFI boot
//! partition (Windows Boot Manager, its signature, BCD, the fallback
//! loader), the Windows drive (BitLocker, Windows' own copy of the boot
//! manager), the firmware's boot entries, and the Secure Boot keys.
//!
//! When the internal disk's partition list is empty (partitions deleted in
//! the Windows installer, or diskpart clean), it looks for the partitions
//! still on it by their boot sectors, opens them read-only and offers to
//! copy each user's folders to the USB drive.
//!
//! Fixes it may make, after copying the whole boot partition and the boot
//! entries to the USB drive: put back a missing or damaged Windows Boot
//! Manager from Windows' own copy (same version, same signature), put back
//! a missing fallback loader, and add a Windows Boot Manager boot entry
//! that points at the right partition. It never writes Secure Boot keys,
//! never edits BCD, never formats anything; those steps are explained
//! instead (bcdboot in the Windows installer, the BIOS key menu).

mod desktop;
mod disk;
mod efi;
mod lost;
mod mbr;
mod pe;
#[path = "../../nexa-doctor/src/secureboot.rs"]
mod secureboot;

use serde::Serialize;
use serde_json::json;
use std::ffi::CString;
use std::fs;
use std::io::{BufRead, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn env_path(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).unwrap_or_else(|| default.into())
}
fn sys_root() -> PathBuf { env_path("NEXA_BOOTFIX_SYS", "/sys") }
fn dev_root() -> PathBuf { env_path("NEXA_BOOTFIX_DEV", "/dev") }
fn proc_root() -> PathBuf { env_path("NEXA_BOOTFIX_PROC", "/proc") }
fn work_dir() -> PathBuf { env_path("NEXA_BOOTFIX_WORK", "/run/nexa-bootfix") }
fn efivars() -> PathBuf { sys_root().join("firmware/efi/efivars") }

// ---------- mounting ----------

fn mount(dev: &Path, at: &Path, fs_type: &str, readonly: bool) -> Result<(), String> {
    mount_with(dev, at, fs_type, readonly, None)
}

/// The same, with filesystem options (e.g. who owns the files).
fn mount_with(dev: &Path, at: &Path, fs_type: &str, readonly: bool, options: Option<&str>) -> Result<(), String> {
    fs::create_dir_all(at).map_err(|e| e.to_string())?;
    let c = |s: &str| CString::new(s).unwrap();
    let (src, tgt) = (c(&dev.to_string_lossy()), c(&at.to_string_lossy()));
    let flags = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC | if readonly { libc::MS_RDONLY } else { 0 };
    let types: &[&str] = match fs_type {
        "ntfs" => &["ntfs3"],
        "vfat" => &["vfat"],
        "exfat" => &["exfat"],
        other => return Err(format!("can't open a {other} filesystem")),
    };
    let data = options.map(c);
    let data_ptr = data.as_ref().map_or(std::ptr::null(), |d| d.as_ptr() as *const libc::c_void);
    let mut last = String::new();
    for t in types {
        // SAFETY: plain mount(2) with NUL-terminated strings.
        let r = unsafe { libc::mount(src.as_ptr(), tgt.as_ptr(), c(t).as_ptr(), flags, data_ptr) };
        if r == 0 {
            return Ok(());
        }
        last = std::io::Error::last_os_error().to_string();
    }
    Err(last)
}

fn umount(at: &Path) {
    let p = CString::new(at.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: umount2(2). Still in use (a window open on it): let go of it
    // now, the kernel finishes when the last file is closed.
    unsafe {
        if libc::umount2(p.as_ptr(), 0) != 0 {
            libc::umount2(p.as_ptr(), libc::MNT_DETACH);
        }
    }
    let _ = fs::remove_dir(at);
}

/// Where a device is mounted already (e.g. a USB drive under /media).
fn mounted_at(dev: &str) -> Option<PathBuf> {
    fs::read_to_string(proc_root().join("mounts")).ok()?.lines().find_map(|l| {
        let w: Vec<&str> = l.split_whitespace().collect();
        (w.len() > 2 && w[0] == format!("/dev/{dev}")).then(|| PathBuf::from(w[1].replace("\\040", " ")))
    })
}

/// A path inside a FAT/NTFS tree, ignoring upper/lower case.
fn find_ci(root: &Path, parts: &[&str]) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for part in parts {
        let want = part.to_ascii_lowercase();
        cur = fs::read_dir(&cur).ok()?.flatten().find(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == want)?.path();
    }
    Some(cur)
}

// ---------- what's there ----------

#[derive(Debug, Serialize, Clone, Default)]
struct Esp {
    dev: String,
    disk: String,
    guid: String,
    number: u32,
    first_lba: u64,
    sectors: u64,
    readable: bool,
    error: Option<String>,
    /// Folders under \EFI (Microsoft, Boot, nexaos, ...)
    efi_dirs: Vec<String>,
    has_microsoft_boot: bool,
    bootmgr: Option<pe::Image>,
    bootmgr_ca: String,
    /// BCD: (bytes, looks like a registry hive)
    bcd: Option<(u64, bool)>,
    fallback: Option<pe::Image>,
    free_mb: Option<u64>,
}

#[derive(Debug, Serialize, Clone, Default)]
struct WinDrive {
    dev: String,
    fs: String,
    size_mb: u64,
    readable: bool,
    error: Option<String>,
    has_windows: bool,
    winload: bool,
    /// \Windows\Boot\EFI\bootmgfw.efi: Windows' own copy.
    bootmgr_copy: Option<pe::Image>,
    bootmgr_copy_ca: String,
    /// Same bytes as the boot partition's copy?
    same_as_esp: Option<bool>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
struct Finding {
    /// "fault" (stops Windows), "warning", "info"
    level: String,
    title: String,
    detail: String,
    /// A fix this tool can make.
    fix: Option<String>,
}

#[derive(Debug, Serialize)]
struct Report {
    time: String,
    uefi: bool,
    disks: Vec<disk::Disk>,
    esps: Vec<Esp>,
    windows: Vec<WinDrive>,
    entries: Vec<efi::Entry>,
    boot_order: Vec<String>,
    boot_current: Option<String>,
    keys: secureboot::Keys,
    /// Internal disks with an empty partition list: what's still on them.
    lost: Vec<lost::Scan>,
    /// Windows Legacy BIOS boot partitions on MBR disks.
    legacy: Vec<Legacy>,
    findings: Vec<Finding>,
}

fn find(level: &str, title: impl Into<String>, detail: impl Into<String>, fix: Option<&str>) -> Finding {
    Finding { level: level.into(), title: title.into(), detail: detail.into(), fix: fix.map(String::from) }
}

fn free_mb(path: &Path) -> Option<u64> {
    let p = CString::new(path.to_string_lossy().as_bytes()).ok()?;
    // SAFETY: statvfs fills a plain struct.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(p.as_ptr(), &mut st) } == 0).then(|| st.f_bavail as u64 * st.f_frsize as u64 / 1_048_576)
}

fn look_at_esp(d: &disk::Disk, p: &disk::Partition) -> Esp {
    let mut e = Esp {
        dev: p.dev.clone(), disk: d.dev.clone(), guid: p.guid.clone(), number: p.number,
        first_lba: p.first_lba, sectors: p.last_lba + 1 - p.first_lba, ..Default::default()
    };
    let at = work_dir().join(format!("esp-{}", p.dev));
    let (root, ours) = match mounted_at(&p.dev) {
        Some(m) => (m, false),
        None => match mount(&dev_root().join(&p.dev), &at, "vfat", true) {
            Ok(()) => (at.clone(), true),
            Err(err) => {
                e.error = Some(format!("can't open it: {err}"));
                return e;
            }
        },
    };
    e.readable = true;
    e.free_mb = free_mb(&root);
    if let Some(efi) = find_ci(&root, &["EFI"]) {
        e.efi_dirs = fs::read_dir(&efi).into_iter().flatten().flatten().map(|x| x.file_name().to_string_lossy().into_owned()).collect();
        e.efi_dirs.sort();
    }
    e.has_microsoft_boot = find_ci(&root, &["EFI", "Microsoft", "Boot"]).is_some();
    if let Some(f) = find_ci(&root, &["EFI", "Microsoft", "Boot", "bootmgfw.efi"]) {
        let img = pe::inspect(&fs::read(&f).unwrap_or_default());
        e.bootmgr_ca = pe::microsoft_ca(&img).into();
        e.bootmgr = Some(img);
    }
    if let Some(f) = find_ci(&root, &["EFI", "Microsoft", "Boot", "BCD"]) {
        let b = fs::read(&f).unwrap_or_default();
        e.bcd = Some((b.len() as u64, b.starts_with(b"regf")));
    }
    if let Some(f) = find_ci(&root, &["EFI", "Boot", "bootx64.efi"]) {
        e.fallback = Some(pe::inspect(&fs::read(&f).unwrap_or_default()));
    }
    if ours {
        umount(&at);
    }
    e
}

fn look_at_windows(p: &disk::Partition, esp_bootmgr: Option<&[u8]>) -> WinDrive {
    let mut w = WinDrive { dev: p.dev.clone(), fs: p.fs.clone(), size_mb: p.size_mb, ..Default::default() };
    if p.fs == "bitlocker" {
        w.error = Some("locked by BitLocker".into());
        return w;
    }
    if p.fs != "ntfs" {
        return w;
    }
    let at = work_dir().join(format!("win-{}", p.dev));
    let (root, ours) = match mounted_at(&p.dev) {
        Some(m) => (m, false),
        None => match mount(&dev_root().join(&p.dev), &at, "ntfs", true) {
            Ok(()) => (at.clone(), true),
            Err(err) => {
                w.error = Some(format!("can't open it: {err}"));
                return w;
            }
        },
    };
    w.readable = true;
    w.has_windows = find_ci(&root, &["Windows", "System32"]).is_some();
    w.winload = find_ci(&root, &["Windows", "System32", "winload.efi"]).is_some();
    if let Some(f) = find_ci(&root, &["Windows", "Boot", "EFI", "bootmgfw.efi"]) {
        let b = fs::read(&f).unwrap_or_default();
        let img = pe::inspect(&b);
        w.bootmgr_copy_ca = pe::microsoft_ca(&img).into();
        w.bootmgr_copy = Some(img);
        w.same_as_esp = esp_bootmgr.map(|e| e == b.as_slice());
    }
    if ours {
        umount(&at);
    }
    w
}

fn esp_bootmgr_bytes(e: &Esp) -> Option<Vec<u8>> {
    let at = work_dir().join(format!("esp-{}", e.dev));
    let (root, ours) = match mounted_at(&e.dev) {
        Some(m) => (m, false),
        None => (at.clone(), mount(&dev_root().join(&e.dev), &at, "vfat", true).ok().map(|_| true)?),
    };
    let b = find_ci(&root, &["EFI", "Microsoft", "Boot", "bootmgfw.efi"]).and_then(|f| fs::read(f).ok());
    if ours {
        umount(&at);
    }
    b
}

fn check() -> Report {
    let _ = fs::create_dir_all(work_dir());
    let disks = disk::disks(&sys_root(), &dev_root(), &proc_root());
    let internal: Vec<&disk::Disk> = disks.iter().filter(|d| !d.boot_stick && !d.removable).collect();
    let mut esps = Vec::new();
    let mut windows = Vec::new();
    for d in &internal {
        for p in d.partitions.iter().filter(|p| p.type_guid == disk::ESP) {
            esps.push(look_at_esp(d, p));
        }
    }
    let esp_bytes = esps.iter().find(|e| e.bootmgr.is_some()).and_then(esp_bootmgr_bytes);
    for d in &internal {
        for p in d.partitions.iter().filter(|p| p.type_guid == disk::MS_DATA && (p.fs == "ntfs" || p.fs == "bitlocker")) {
            windows.push(look_at_windows(p, esp_bytes.as_deref()));
        }
    }
    let dir = efivars();
    let uefi = sys_root().join("firmware/efi").exists();
    let entries = if uefi { efi::entries(&dir) } else { vec![] };
    let keys = secureboot::read(&sys_root());
    let lost = internal.iter().filter(|d| d.partitions.is_empty()).map(|d| scan_disk(d)).collect();
    let legacy = find_legacy(&internal);
    let mut r = Report {
        time: now().1,
        uefi,
        boot_order: efi::boot_order(&dir).iter().map(|n| format!("{n:04X}")).collect(),
        boot_current: efi::boot_current(&dir).map(|n| format!("{n:04X}")),
        disks, esps, windows, entries, keys, lost, legacy,
        findings: vec![],
    };
    r.findings = findings(&r);
    r
}

// ---------- Windows in Legacy BIOS mode ----------

#[derive(Debug, Serialize, Clone, Default)]
struct Legacy {
    dev: String,
    disk: String,
    /// Marked active in the MBR (the BIOS starts that one).
    active: bool,
    disk_signature: u32,
    /// The disk ID Windows' BCD expects.
    bcd_signature: Option<u32>,
}

/// NTFS partitions with bootmgr and \Boot\BCD on internal MBR disks.
fn find_legacy(internal: &[&disk::Disk]) -> Vec<Legacy> {
    let mut out = Vec::new();
    for d in internal.iter().filter(|d| !d.gpt && !d.partitions.is_empty()) {
        let Some((sig, entries)) = disk::read_file(&dev_root().join(&d.dev), 0, 512).as_deref().and_then(mbr::parse) else { continue };
        for p in d.partitions.iter().filter(|p| p.fs == "ntfs") {
            let at = work_dir().join(format!("legacy-{}", p.dev));
            let (root, undo) = match mounted_at(&p.dev) {
                Some(m) => (m, false),
                None => match mount(&dev_root().join(&p.dev), &at, "ntfs", true) {
                    Ok(()) => (at.clone(), true),
                    Err(_) => continue,
                },
            };
            if find_ci(&root, &["bootmgr"]).is_some() {
                if let Some(bcd) = find_ci(&root, &["Boot", "BCD"]) {
                    out.push(Legacy {
                        dev: p.dev.clone(), disk: d.dev.clone(), disk_signature: sig,
                        active: entries.iter().any(|e| e.0 && e.2 as u64 == p.first_lba),
                        bcd_signature: fs::read(&bcd).ok().and_then(|b| mbr::bcd_disk(&b)).map(|x| x.0),
                    });
                }
            }
            if undo {
                umount(&at);
            }
        }
    }
    out
}

const LEGACY_BIOS: &str = "Set the BIOS to start it: ThinkPad: F1 > Startup > UEFI/Legacy Boot: Both, UEFI/Legacy Boot Priority: \
Legacy First (or Legacy Only), CSM Support: Yes. Secure Boot must be off for that (Security > Secure Boot: Disabled). Save, restart, \
and choose the internal disk (e.g. \"NVMe0\") in the F12 boot menu.";

fn legacy_findings(r: &Report) -> Vec<Finding> {
    let mut out = Vec::new();
    for l in &r.legacy {
        if !l.active {
            out.push(find("fault", format!("Windows' boot partition {} isn't marked active", l.dev),
                format!("The BIOS starts the active partition. In the Windows installer's command prompt (Shift+F10): diskpart, \
select disk 0, select partition 1, active, exit. {LEGACY_BIOS}"), None));
        } else if l.bcd_signature.map_or(false, |s| s != l.disk_signature) {
            out.push(find("fault", format!("{}'s disk ID doesn't match Windows' boot settings", l.disk),
                format!("The disk ID is {:08X}; Windows' BCD expects {:08X}. In the Windows installer's command prompt (Shift+F10): \
bcdboot C:\\Windows /s <the boot partition's letter>: /f BIOS. {LEGACY_BIOS}", l.disk_signature, l.bcd_signature.unwrap_or(0)), None));
        } else {
            out.push(find("warning", "Windows on this PC starts in Legacy BIOS mode",
                format!("Its boot files are on {} (marked active, disk ID {:08X}, as Windows expects). {LEGACY_BIOS}", l.dev, l.disk_signature), None));
        }
    }
    out
}

// ---------- lost partitions ----------

/// Look for the partitions still on a disk whose partition list is empty,
/// and at what's inside them. Read-only.
fn scan_disk(d: &disk::Disk) -> lost::Scan {
    let path = dev_root().join(&d.dev);
    let bytes = fs::read_to_string(sys_root().join("block").join(&d.dev).join("size")).ok()
        .and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0) * 512;
    let mut s = lost::Scan { disk: d.dev.clone(), size: bytes, sector: d.sector, ..Default::default() };
    let Ok(file) = fs::File::open(&path) else { return s };
    let mut read = |at: u64, len: usize| {
        let mut b = vec![0u8; len];
        file.read_exact_at(&mut b, at).ok().map(|_| b)
    };
    let name = d.dev.clone();
    let (mut found, checked, with_data) = lost::scan(&mut read, bytes, &mut |p| eprint!("\rLooking for lost partitions on {name}: {p}%   "));
    eprintln!();
    // The backup partition list, in the disk's last sector, may still have them.
    let last_lba = (bytes / d.sector).saturating_sub(1);
    for (_, _, _, _, first, last) in disk::parse_gpt_at(&mut read, d.sector, last_lba).unwrap_or_default() {
        let start = first * d.sector;
        if last < first || last >= last_lba {
            continue;
        }
        if let Some(f) = found.iter_mut().find(|f| f.start == start) {
            f.from = "scan and backup partition table".into();
            continue;
        }
        let head = read(start, 4096).unwrap_or_default();
        let (fs_type, label) = lost::boot_sector(&head).map(|(f, _, l)| (f, l)).unwrap_or(("", String::new()));
        found.push(lost::Found { start, size: (last + 1 - first) * d.sector, fs: fs_type.into(), label,
            from: "backup partition table".into(), ..Default::default() });
    }
    found.sort_by_key(|f| f.start);
    for f in &mut found {
        look_inside(&path, d.sector, f);
    }
    s.checked = checked;
    s.with_data = with_data;
    s.found = found;
    s
}

/// Mount a found partition read-only (through a read-only loop device).
fn open_lost(disk: &Path, sector: u64, f: &lost::Found, at: &Path, options: Option<&str>) -> Result<(), String> {
    let (dev, handle) = lost::attach(&dev_root(), disk, f.start, f.size, sector)?;
    let r = mount_with(&dev, at, &f.fs, true, options);
    // The mount holds the loop device now; it goes away when unmounted.
    drop(handle);
    r
}

fn look_inside(disk: &Path, sector: u64, f: &mut lost::Found) {
    match f.fs.as_str() {
        "bitlocker" => {
            f.what = "locked by BitLocker".into();
            return;
        }
        "ntfs" | "vfat" | "exfat" => {}
        _ => {
            f.what = "unknown".into();
            return;
        }
    }
    let at = work_dir().join(format!("lost-{}", f.start / lost::STEP));
    if let Err(e) = open_lost(disk, sector, f, &at, None) {
        f.what = format!("can't be opened: {e}");
        return;
    }
    let has = |p: &[&str]| find_ci(&at, p).is_some();
    // Windows' Legacy BIOS boot files: bootmgr, and BCD saying where Windows is.
    if f.fs == "ntfs" && has(&["bootmgr"]) {
        if let Some(bcd) = find_ci(&at, &["Boot", "BCD"]) {
            f.legacy_boot = true;
            if let Some((sig, offs)) = fs::read(&bcd).ok().and_then(|b| mbr::bcd_disk(&b)) {
                f.bcd_signature = Some(sig);
                f.bcd_offsets = offs;
            }
        }
    }
    f.what = if f.fs == "vfat" {
        if has(&["EFI", "Microsoft"]) { "EFI boot partition (Windows)" } else if has(&["EFI"]) { "EFI boot partition" } else { "FAT32 drive" }
    } else if has(&["Windows", "System32"]) {
        "Windows (C:)"
    } else if f.legacy_boot {
        "Windows boot (Legacy BIOS)"
    } else if has(&["Recovery", "WindowsRE"]) || (has(&["Recovery"]) && f.size < 4 << 30) {
        "Windows recovery"
    } else {
        "data drive"
    }.into();
    if f.what == "Windows (C:)" {
        f.users = lost::users(&at);
    }
    umount(&at);
}

/// A found partition with someone's files on it.
fn has_files(f: &lost::Found) -> bool {
    f.what == "Windows (C:)" || f.what == "data drive"
}

fn lost_finding(s: &lost::Scan, boots_elsewhere: bool) -> Finding {
    let disk = &s.disk;
    let list = s.found.iter().map(|f| format!("{} ({})", f.what, lost::human(f.size))).collect::<Vec<_>>().join(", ");
    if s.found.iter().any(has_files) {
        let users: Vec<&str> = s.found.iter().flat_map(|f| f.users.iter().map(String::as_str)).collect();
        let restore = mbr::plan(&s.found, s.size, s.sector.max(512));
        let legacy = s.found.iter().any(|f| f.legacy_boot);
        let tail = match (&restore, legacy) {
            (Ok(p), _) => format!(" This Windows was installed in Legacy BIOS mode. After your files are safe, NexaOS can put the \
partition list back as it was (disk ID {:08X} and the partition starts, from Windows' own boot settings), so Windows starts again \
without reinstalling: only the disk's first sector and the empty partition list at its start and end are written, after they're \
saved to the USB drive (and it can be undone).", p.signature),
            (Err(e), true) => format!(" This Windows was installed in Legacy BIOS mode, but NexaOS won't put its partition list back: {e}."),
            _ => String::new(),
        };
        return find("fault", format!("The partition list on {disk} is empty, but Windows' partitions are still on it"),
            format!("They were deleted (in the Windows installer's \"Where do you want to install Windows?\" screen, or with diskpart clean), \
so the PC finds nothing to start. Still on the disk: {list}.{} Your files are still there: NexaOS can copy them to a USB drive, \
reading the internal disk without changing anything on it. Until your files are safe, don't install Windows or anything else \
on this disk and don't create partitions on it.{tail}", if users.is_empty() { String::new() } else { format!(" User folders: {}.", users.join(", ")) }),
            restore.is_ok().then_some("restore-mbr"));
    }
    if s.found.iter().any(|f| f.fs == "bitlocker") {
        return find("fault", format!("The partition list on {disk} is empty; Windows is still on it, locked by BitLocker"),
            format!("Still on the disk: {list}. NexaOS can't open BitLocker drives yet, so it can't copy the files. Don't install \
anything on this disk. Getting the files back needs your BitLocker recovery key (aka.ms/myrecoverykey, from another device) \
and a data-recovery program."), None);
    }
    let level = if boots_elsewhere { "info" } else { "fault" };
    if s.with_data * 100 < s.checked.max(1) {
        return find(level, format!("{disk} is blank"),
            "Its partition list is empty and it reads back blank everywhere NexaOS looked: SSDs erase deleted space by themselves, \
so files that were on it can't be brought back from this disk (files kept in OneDrive or another cloud are still there). \
To use the PC again, install Windows from the installer USB and choose the empty space.", None);
    }
    find(level, format!("The partition list on {disk} is empty"),
        format!("No Windows partition was found where they usually start{}, but {}% of the disk still holds data. \
Don't install anything on it: a recovery program (PhotoRec, free) may still find files.",
            if list.is_empty() { String::new() } else { format!(" (only: {list})") }, s.data_percent()), None)
}

const BCDBOOT: &str = "Rebuild them from the Windows installer USB: on its first screen press Shift+F10, run diskpart, \
\"list vol\" to find the Windows drive letter (big NTFS, e.g. C) and the FAT32 boot partition (about 100 MB); \
\"sel vol N\", \"assign letter=S\", \"exit\"; then: bcdboot C:\\Windows /s S: /f UEFI";

/// What's wrong, worst first.
fn findings(r: &Report) -> Vec<Finding> {
    let mut out = Vec::new();
    if !r.uefi {
        out.push(find("warning", "NexaOS started in legacy (BIOS) mode",
            "Boot entries and Secure Boot can't be checked. Restart and choose the stick's UEFI entry in the F12 menu.", None));
    }
    let good_copy = r.windows.iter().any(|w| w.bootmgr_copy.as_ref().map_or(false, |i| i.valid && i.problem.is_none()));
    for s in &r.lost {
        out.push(lost_finding(s, !r.esps.is_empty()));
    }
    out.extend(legacy_findings(r));
    if r.esps.is_empty() && r.lost.is_empty() && r.legacy.is_empty() {
        out.push(find("fault", "No EFI boot partition on the internal disk",
            format!("Windows needs one to start (a small FAT32 partition). {BCDBOOT}"), None));
    }
    let sb_on = r.keys.secure_boot == Some(true) && r.keys.setup_mode != Some(true);
    for e in &r.esps {
        let name = format!("the boot partition ({})", e.dev);
        if !e.readable {
            out.push(find("fault", format!("Can't read {name}"),
                format!("{}. Its FAT filesystem may be damaged: in the Windows installer's command prompt, chkdsk S: /f (after giving it the letter S as below). {BCDBOOT}",
                    e.error.clone().unwrap_or_default()), None));
            continue;
        }
        if !e.has_microsoft_boot {
            out.push(find("fault", format!("Windows' boot folder is missing from {name}"),
                format!("\\EFI\\Microsoft\\Boot isn't there, so there is nothing for the firmware to start. {BCDBOOT}"), None));
            continue;
        }
        match &e.bootmgr {
            None => out.push(find("fault", "Windows Boot Manager (bootmgfw.efi) is missing",
                if good_copy { "NexaOS can put back Windows' own copy of it." .to_string() } else { format!("Windows' own copy can't be read here. {BCDBOOT}") },
                good_copy.then_some("restore-bootmgr"))),
            Some(i) if i.problem.is_some() => out.push(find("fault", format!("Windows Boot Manager is damaged: {}", i.problem.clone().unwrap()),
                if good_copy { "NexaOS can put back Windows' own copy of it.".to_string() } else { format!("Windows' own copy can't be read here. {BCDBOOT}") },
                good_copy.then_some("restore-bootmgr"))),
            Some(i) => {
                if sb_on {
                    let refused = match e.bootmgr_ca.as_str() {
                        "2023" => !r.keys.trusts_windows_2023,
                        "2011" => !r.keys.db.iter().any(|c| c.contains("Microsoft Windows Production PCA 2011")),
                        _ => !i.signed,
                    };
                    if refused {
                        out.push(find("fault", format!("Secure Boot refuses Windows Boot Manager (signed with Microsoft's {} key)",
                                if e.bootmgr_ca.is_empty() { "unknown".to_string() } else { e.bootmgr_ca.clone() }),
                            "The PC's trusted list (db) doesn't have that key. Turn Secure Boot off in the BIOS, start Windows, install all updates, \
then turn Secure Boot on again. If the 2023 key is the one missing, a BIOS update from the PC maker also adds it.", None));
                    }
                }
            }
        }
        match e.bcd {
            None => out.push(find("fault", "Windows' boot settings (BCD) are missing", BCDBOOT, None)),
            Some((_, false)) => out.push(find("fault", "Windows' boot settings (BCD) are damaged", BCDBOOT, None)),
            _ => {}
        }
        match &e.fallback {
            None if e.bootmgr.as_ref().map_or(good_copy, |i| i.problem.is_none()) || good_copy =>
                out.push(find("warning", "The disk's fallback loader (\\EFI\\Boot\\bootx64.efi) is missing",
                    "Firmware entries named after the disk (like \"NVMe0 ...\") start this file. NexaOS can put a copy of Windows Boot Manager there.",
                    Some("restore-fallback"))),
            Some(i) if i.problem.is_some() =>
                out.push(find("warning", format!("The disk's fallback loader is damaged: {}", i.problem.clone().unwrap()),
                    "Firmware entries named after the disk start this file. NexaOS can replace it with Windows Boot Manager.",
                    (good_copy || e.bootmgr.as_ref().map_or(false, |b| b.problem.is_none())).then_some("restore-fallback"))),
            _ => {}
        }
        if e.free_mb.map_or(false, |f| f < 10) {
            out.push(find("warning", format!("The boot partition is almost full ({} MB free)", e.free_mb.unwrap()),
                "Windows updates that change the boot files can fail. Old fonts and language folders under \\EFI\\Microsoft\\Boot can be removed from Windows (Disk Management can't do it; ask for help).", None));
        }
    }
    // Boot entries.
    if r.uefi && !r.esps.is_empty() {
        let has_bootmgr = r.esps.iter().any(|e| e.has_microsoft_boot);
        let wins: Vec<&efi::Entry> = r.entries.iter().filter(|e| e.is_windows()).collect();
        let known = |g: &Option<String>| g.as_ref().map_or(false, |g| r.esps.iter().any(|e| &e.guid == g));
        if has_bootmgr && wins.is_empty() {
            out.push(find("fault", "The firmware has no \"Windows Boot Manager\" entry",
                "NexaOS can add one pointing at the boot partition.", Some("boot-entry")));
        } else if has_bootmgr && !wins.iter().any(|e| known(&e.partition_guid) && e.active) {
            let e = wins[0];
            let why = if !e.active { "is switched off".to_string() }
                else if e.partition_guid.is_none() { "doesn't say which partition to start".to_string() }
                else { format!("points to a partition that isn't on this disk ({})", e.partition_guid.clone().unwrap()) };
            out.push(find("fault", format!("The \"{}\" entry {why}", e.description),
                "So choosing it just returns to the menu. NexaOS can make a new entry pointing at the right partition.", Some("boot-entry")));
        }
    }
    // Secure Boot keys.
    for p in &r.keys.problems {
        out.push(find("warning", p.title.clone(), p.detail.clone(), None));
    }
    for w in &r.windows {
        if w.fs == "bitlocker" {
            out.push(find("info", format!("The Windows drive ({}) is encrypted with BitLocker", w.dev),
                "NexaOS can't look inside it, so it can't use Windows' own boot files. Repairs in the Windows installer need the recovery key: aka.ms/myrecoverykey.", None));
        } else if let Some(err) = &w.error {
            out.push(find("warning", format!("The Windows drive ({}) can't be opened", w.dev),
                format!("{err}. If Windows was shut down with Fast Startup or hibernated, that is normal; otherwise run chkdsk C: /f from the Windows installer."), None));
        } else if w.has_windows && !w.winload {
            out.push(find("fault", "Windows' loader (winload.efi) is missing from the Windows drive",
                "Windows itself is damaged. In the Windows installer: Repair your computer > Troubleshoot > Startup Repair, or sfc /scannow /offbootdir=C:\\ /offwindir=C:\\Windows from Shift+F10.", None));
        }
        let esp_ok = r.esps.iter().any(|e| e.bootmgr.as_ref().map_or(false, |i| i.problem.is_none()));
        if w.same_as_esp == Some(false) && esp_ok && w.bootmgr_copy.as_ref().map_or(false, |i| i.problem.is_none()) {
            out.push(find("info", "The boot partition's Windows Boot Manager differs from Windows' own copy",
                format!("Often normal after updates. Windows' copy is signed with the {} key, the boot partition's with the {} key.",
                    if w.bootmgr_copy_ca.is_empty() { "unknown" } else { &w.bootmgr_copy_ca },
                    r.esps.iter().find(|e| e.bootmgr.is_some()).map_or("unknown", |e| if e.bootmgr_ca.is_empty() { "unknown" } else { &e.bootmgr_ca })), None));
        }
    }
    if !out.iter().any(|f| f.level == "fault") && !r.esps.is_empty() {
        out.push(find("info", "No fault found in Windows' boot files or boot entries",
            "If Windows still returns to the menu: try with Secure Boot off; in the BIOS check that \"Boot mode\" is UEFI only and CSM is off; \
update the BIOS from the PC maker; then in the Windows installer (Shift+F10) run bcdboot as below to rebuild the boot files. ".to_string() + BCDBOOT, None));
    }
    let rank = |l: &str| match l { "fault" => 0, "warning" => 1, _ => 2 };
    out.sort_by_key(|f| rank(&f.level));
    out
}

// ---------- the report ----------

/// (compact for names, readable) in UTC.
fn now() -> (String, String) {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, secs) = (t.div_euclid(86400), t.rem_euclid(86400));
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let (h, mi, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    (format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}"), format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC"))
}

fn image_text(i: &Option<pe::Image>) -> String {
    let ca = i.as_ref().map_or("", pe::microsoft_ca);
    match i {
        None => "MISSING".into(),
        Some(i) if i.problem.is_some() => format!("DAMAGED: {} ({} bytes)", i.problem.clone().unwrap(), i.size),
        Some(i) => format!("ok, {} bytes, {}{}", i.size, i.machine,
            if !i.signed { ", NOT signed".to_string() } else if ca.is_empty() { format!(", signed by: {}", i.names.join(" / ")) } else { format!(", signed with Microsoft's {ca} key") }),
    }
}

fn text(r: &Report) -> String {
    let mut s = format!("NexaOS boot check for Windows - {}\n\n", r.time);
    let faults = r.findings.iter().filter(|f| f.level == "fault").count();
    s += &format!("RESULT: {}\n", if faults == 0 { "no fault found".to_string() } else { format!("{faults} fault(s) that can stop Windows starting") });
    for (i, f) in r.findings.iter().enumerate() {
        let tag = match f.level.as_str() { "fault" => "FAULT", "warning" => "WARNING", _ => "NOTE" };
        s += &format!("\n{}. [{tag}] {}\n   {}\n", i + 1, f.title, f.detail);
        if let Some(fix) = &f.fix {
            s += &format!("   -> NexaOS can fix this ({fix})\n");
        }
    }
    s += "\n--- Disks\n";
    for d in &r.disks {
        s += &format!("{} {} {:.1} GB{}{}{}\n", d.dev, d.model, d.size_gb, if d.boot_stick { " (the NexaOS stick)" } else { "" },
            if d.removable && !d.boot_stick { " (removable)" } else { "" }, if d.gpt { "" } else { " (MBR)" });
        for p in &d.partitions {
            s += &format!("  {:<12} {:>7} MB  {:<26} {:<9} {}\n", p.dev, p.size_mb, p.kind, if p.fs.is_empty() { "-" } else { &p.fs }, p.name);
        }
    }
    for l in &r.legacy {
        s += &format!("\n--- Windows Legacy BIOS boot on {} (disk {}): active: {}, disk ID {:08X}, Windows expects {}\n", l.dev, l.disk,
            if l.active { "yes" } else { "NO" }, l.disk_signature, l.bcd_signature.map_or("?".into(), |x| format!("{x:08X}")));
    }
    for l in &r.lost {
        s += &format!("\n--- Lost partitions on {} (looked at {} places, {}% of them hold data)\n", l.disk, l.checked, l.data_percent());
        if l.found.is_empty() {
            s += "none found\n";
        }
        for f in &l.found {
            s += &format!("at {} MB: {}, {}{}: {}{}{}\n", f.start >> 20, lost::human(f.size), f.fs,
                if f.label.is_empty() { String::new() } else { format!(" \"{}\"", f.label) }, f.what,
                if f.users.is_empty() { String::new() } else { format!(" (users: {})", f.users.join(", ")) },
                if f.from == "scan" { String::new() } else { format!(" [from the {}]", f.from) });
        }
    }
    for e in &r.esps {
        s += &format!("\n--- Boot partition {} (GUID {})\n", e.dev, e.guid);
        if !e.readable {
            s += &format!("can't read it: {}\n", e.error.clone().unwrap_or_default());
            continue;
        }
        s += &format!("\\EFI folders: {}\n", if e.efi_dirs.is_empty() { "none".into() } else { e.efi_dirs.join(", ") });
        s += &format!("Windows Boot Manager: {}\n", image_text(&e.bootmgr));
        s += &format!("BCD: {}\n", match e.bcd { None => "MISSING".into(), Some((n, true)) => format!("ok, {n} bytes"), Some((n, false)) => format!("DAMAGED ({n} bytes)") });
        s += &format!("Fallback \\EFI\\Boot\\bootx64.efi: {}\n", image_text(&e.fallback));
        if let Some(f) = e.free_mb {
            s += &format!("Free space: {f} MB\n");
        }
    }
    for w in &r.windows {
        s += &format!("\n--- Windows drive {} ({}, {} MB)\n", w.dev, w.fs, w.size_mb);
        if let Some(err) = &w.error {
            s += &format!("{err}\n");
            continue;
        }
        s += &format!("Windows folder: {}, winload.efi: {}\n", if w.has_windows { "yes" } else { "no" }, if w.winload { "yes" } else { "no" });
        s += &format!("Windows' own boot manager: {}\n", image_text(&w.bootmgr_copy));
        if let Some(same) = w.same_as_esp {
            s += &format!("Same as the boot partition's: {}\n", if same { "yes" } else { "no" });
        }
    }
    s += &format!("\n--- Firmware boot entries (order: {}; started now: {})\n",
        if r.boot_order.is_empty() { "-".into() } else { r.boot_order.join(", ") }, r.boot_current.clone().unwrap_or("-".into()));
    for e in &r.entries {
        let part = e.partition_guid.as_ref().map(|g| {
            let found = r.esps.iter().any(|x| &x.guid == g);
            format!(" partition {g}{}", if found { "" } else { " (NOT ON THIS DISK)" })
        }).unwrap_or_default();
        s += &format!("Boot{} {}{}{}  {}{}\n", e.id, e.description, if e.active { "" } else { " (off)" }, if e.in_boot_order { "" } else { " (not in order)" },
            e.file.clone().unwrap_or_default(), part);
    }
    s += "\n--- Secure Boot keys\n";
    s += &secureboot::text(&r.keys);
    s
}

// ---------- saving ----------

#[derive(Debug, Serialize, Clone)]
struct Drive {
    dev: String,
    fs: String,
    label: String,
    size_mb: u64,
    mounted: Option<String>,
}

/// USB drives (not the NexaOS stick) with a filesystem the report can go on.
fn drives() -> Vec<Drive> {
    let disks = disk::disks(&sys_root(), &dev_root(), &proc_root());
    let mut out = Vec::new();
    for d in disks.iter().filter(|d| d.removable && !d.boot_stick) {
        let b = sys_root().join("block").join(&d.dev);
        let mut parts: Vec<String> = fs::read_dir(&b).into_iter().flatten().flatten()
            .filter(|e| e.path().join("partition").exists()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        parts.sort();
        if parts.is_empty() {
            parts.push(d.dev.clone()); // a drive formatted without a partition table
        }
        for p in parts {
            let head = fs::File::open(dev_root().join(&p)).ok().and_then(|mut f| {
                let mut v = vec![0u8; 4096];
                std::io::Read::read_exact(&mut f, &mut v).ok().map(|_| v)
            }).unwrap_or_default();
            let fs_type = disk::fs_of(&head);
            if !matches!(fs_type, "vfat" | "exfat" | "ntfs") {
                continue;
            }
            let size: u64 = fs::read_to_string(b.join(&p).join("size")).or_else(|_| fs::read_to_string(b.join("size")))
                .ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            let label = match fs_type {
                "vfat" => String::from_utf8_lossy(head.get(if head.get(0x52..0x57) == Some(b"FAT32") { 0x47..0x52 } else { 0x2b..0x36 }).unwrap_or(&[])).trim().to_string(),
                _ => String::new(),
            };
            out.push(Drive { mounted: mounted_at(&p).map(|m| m.to_string_lossy().into_owned()), dev: p, fs: fs_type.into(), label, size_mb: size * 512 / 1_048_576 });
        }
    }
    out
}

/// Open a drive for writing: (folder to write in, mount point to undo).
fn open_drive(d: &Drive) -> Result<(PathBuf, Option<PathBuf>), String> {
    if let Some(m) = &d.mounted {
        return Ok((PathBuf::from(m), None));
    }
    let at = work_dir().join("drive");
    // Owned by the person at the desktop, so Files can copy onto it too.
    let opts = desktop::current().map(|u| desktop::owner_options(&d.fs, &u.user));
    mount_with(&dev_root().join(&d.dev), &at, &d.fs, false, opts.as_deref()).map_err(|e| format!("can't open {}: {e}", d.dev))?;
    Ok((at.clone(), Some(at)))
}

fn sync() {
    // SAFETY: sync(2).
    unsafe { libc::sync() };
}

/// Copy a folder tree.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<u64> {
    fs::create_dir_all(to)?;
    let mut n = 0;
    for e in fs::read_dir(from)? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            n += copy_tree(&e.path(), &dst)?;
        } else {
            n += fs::copy(e.path(), &dst)?;
        }
    }
    Ok(n)
}

// ---------- fixing ----------

/// Write a file and make sure it reached the disk.
fn write_synced(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(data)?;
    f.sync_all()
}

/// Back up, then apply every fix the report offers. Returns what was done.
fn fix(r: &Report, backup_to: &Path) -> Result<Vec<String>, String> {
    let wanted: Vec<&str> = r.findings.iter().filter_map(|f| f.fix.as_deref()).collect();
    if wanted.is_empty() {
        return Ok(vec![]);
    }
    let mut done = Vec::new();
    if wanted.contains(&"restore-mbr") {
        done.extend(restore_mbr(r, backup_to)?);
        if r.esps.is_empty() {
            return Ok(done);
        }
    }
    let esp = r.esps.iter().find(|e| e.readable && e.has_microsoft_boot).or_else(|| r.esps.iter().find(|e| e.readable))
        .ok_or("no readable boot partition")?;
    // 1. Back up the whole boot partition and the boot entries.
    let backup = backup_to.join("backup");
    let at = work_dir().join("esp-fix");
    let already = mounted_at(&esp.dev);
    if already.is_some() {
        return Err(format!("{} is mounted elsewhere; unmount it first", esp.dev));
    }
    mount(&dev_root().join(&esp.dev), &at, "vfat", true).map_err(|e| format!("can't open the boot partition: {e}"))?;
    let copied = copy_tree(&at, &backup.join("boot-partition"));
    umount(&at);
    let copied = copied.map_err(|e| format!("backup failed, nothing was changed: {e}"))?;
    let vars = backup.join("efivars");
    fs::create_dir_all(&vars).map_err(|e| e.to_string())?;
    for e in fs::read_dir(efivars()).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with("Boot") {
            let _ = fs::copy(e.path(), vars.join(&n));
        }
    }
    sync();
    done.push(format!("Backed up the boot partition ({} KB) and the boot entries to {}", copied / 1024, backup.display()));
    // 2. Windows' own boot manager, if a fix needs it.
    let source: Option<Vec<u8>> = r.windows.iter().filter(|w| w.bootmgr_copy.as_ref().map_or(false, |i| i.problem.is_none())).find_map(|w| {
        let wat = work_dir().join(format!("win-{}", w.dev));
        mount(&dev_root().join(&w.dev), &wat, "ntfs", true).ok()?;
        let b = find_ci(&wat, &["Windows", "Boot", "EFI", "bootmgfw.efi"]).and_then(|f| fs::read(f).ok());
        umount(&wat);
        b.filter(|b| pe::inspect(b).problem.is_none())
    });
    // 3. The file fixes, on the boot partition opened for writing.
    if wanted.iter().any(|w| *w == "restore-bootmgr" || *w == "restore-fallback") {
        mount(&dev_root().join(&esp.dev), &at, "vfat", false).map_err(|e| format!("can't open the boot partition for writing: {e}"))?;
        let result = (|| -> Result<(), String> {
            let ms_boot = find_ci(&at, &["EFI", "Microsoft", "Boot"]);
            if wanted.contains(&"restore-bootmgr") {
                let src = source.as_ref().ok_or("Windows' own copy of the boot manager couldn't be read")?;
                let dir = ms_boot.clone().ok_or("\\EFI\\Microsoft\\Boot is missing")?;
                let target = find_ci(&dir, &["bootmgfw.efi"]).unwrap_or_else(|| dir.join("bootmgfw.efi"));
                write_synced(&target, src).map_err(|e| e.to_string())?;
                done.push("Put back Windows Boot Manager (\\EFI\\Microsoft\\Boot\\bootmgfw.efi) from Windows' own copy".into());
            }
            if wanted.contains(&"restore-fallback") {
                let src = match &source {
                    Some(s) => s.clone(),
                    None => ms_boot.as_ref().and_then(|d| find_ci(d, &["bootmgfw.efi"])).and_then(|f| fs::read(f).ok())
                        .filter(|b| pe::inspect(b).problem.is_none()).ok_or("no good copy of Windows Boot Manager")?,
                };
                let efi = find_ci(&at, &["EFI"]).ok_or("\\EFI is missing")?;
                let bootdir = find_ci(&efi, &["Boot"]).unwrap_or_else(|| efi.join("Boot"));
                fs::create_dir_all(&bootdir).map_err(|e| e.to_string())?;
                let target = find_ci(&bootdir, &["bootx64.efi"]).unwrap_or_else(|| bootdir.join("bootx64.efi"));
                write_synced(&target, &src).map_err(|e| e.to_string())?;
                done.push("Put Windows Boot Manager at the disk's fallback (\\EFI\\Boot\\bootx64.efi)".into());
            }
            Ok(())
        })();
        sync();
        umount(&at);
        result?;
    }
    // 4. The boot entry.
    if wanted.contains(&"boot-entry") {
        let g = disk::guid_bytes(&esp.guid).ok_or("bad partition GUID")?;
        let opt = efi::build_option("Windows Boot Manager", esp.number, esp.first_lba, esp.sectors, &g, efi::WINDOWS_PATH);
        // Reuse the broken Windows entry's number, else a free one.
        let id = r.entries.iter().find(|e| e.is_windows()).and_then(|e| u16::from_str_radix(&e.id, 16).ok())
            .unwrap_or_else(|| efi::free_id(&efivars()));
        efi::put_first(&efivars(), id, &opt).map_err(|e| format!("couldn't write the boot entry: {e}"))?;
        done.push(format!("Wrote boot entry Boot{id:04X} \"Windows Boot Manager\" -> {} {}, first in the boot order", esp.dev, efi::WINDOWS_PATH));
    }
    Ok(done)
}

// ---------- the screenshot ----------

/// grim on the desktop: the caller's own Wayland socket if it really exists
/// (root's login sets a default that may not), else the desktop of the
/// person who ran doas, else any desktop on this PC. Root may use their
/// sockets.
fn screenshot(to: &Path) -> Result<(), String> {
    let tries = desktop::wayland_sockets();
    let mut last = "no desktop running (no Wayland display)".to_string();
    for (dir, display) in tries {
        let out = Command::new("grim").arg(to).env("XDG_RUNTIME_DIR", &dir).env("WAYLAND_DISPLAY", &display)
            .stdin(Stdio::null()).stdout(Stdio::null()).output().map_err(|e| format!("grim: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        last = format!("grim ({}/{display}): {}", dir.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Err(last)
}

// ---------- putting a deleted MBR partition list back ----------

const BLKRRPART: u64 = 0x125f;

/// Ask the kernel to read a disk's partition list again.
fn reread_partitions(f: &fs::File) -> bool {
    use std::os::unix::io::AsRawFd;
    // SAFETY: BLKRRPART takes no argument.
    unsafe { libc::ioctl(f.as_raw_fd(), BLKRRPART as _) == 0 }
}

fn sys_disk_bytes(dev: &str) -> u64 {
    fs::read_to_string(sys_root().join("block").join(dev).join("size")).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0) * 512
}

/// Write back the MBR partition list of a Windows installed in Legacy BIOS
/// mode (see mbr::plan for when it's allowed). The disk's first and last
/// MiB are saved to `backup_to`/backup and read back before anything is
/// written; then only the first sector and the empty GPT (start and end)
/// are written.
fn restore_mbr(r: &Report, backup_to: &Path) -> Result<Vec<String>, String> {
    let (scan, plan) = r.lost.iter().find_map(|l| mbr::plan(&l.found, l.size, l.sector.max(512)).ok().map(|p| (l, p)))
        .ok_or("there's no partition list NexaOS can safely put back")?;
    let sector = scan.sector.max(512);
    let size = scan.size;
    const MIB: u64 = 1 << 20;
    if sys_disk_bytes(&scan.disk) != size || size < 4 * MIB {
        return Err(format!("{} isn't the disk that was checked; nothing written", scan.disk));
    }
    let path = dev_root().join(&scan.disk);
    let disk = fs::OpenOptions::new().read(true).write(true).open(&path).map_err(|e| format!("can't open {}: {e}", scan.disk))?;
    let read = |at: u64, len: u64| -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len as usize];
        disk.read_exact_at(&mut b, at).map_err(|e| format!("can't read {}: {e}", scan.disk))?;
        Ok(b)
    };
    let tail_at = size - MIB;
    let (head, tail) = (read(0, MIB)?, read(tail_at, MIB)?);
    // Still as checked: no partition list (a protective MBR at most), and
    // every partition's boot sector where the scan saw it.
    if let Some((_, parts)) = mbr::parse(&head[..512]) {
        if parts.iter().any(|p| p.1 != 0xee) {
            return Err(format!("{} has a partition list again; nothing written", scan.disk));
        }
    }
    let mut from_head = |at: u64, len: usize| head.get(at as usize..at as usize + len).map(|b| b.to_vec());
    if disk::parse_gpt(&mut from_head, sector).map_or(false, |e| !e.is_empty()) {
        return Err(format!("{} has a partition list again; nothing written", scan.disk));
    }
    for p in &plan.parts {
        if lost::boot_sector(&read(p.start_lba as u64 * sector, 512)?).is_none() {
            return Err(format!("the partition at {} MB has changed since it was checked; nothing written", p.start_lba as u64 * sector >> 20));
        }
    }
    // 1. Back up, and read the backup back.
    let dir = backup_to.join("backup");
    fs::create_dir_all(&dir).map_err(|e| format!("can't make the backup folder: {e}"))?;
    let mbr0 = mbr::build(&plan);
    let mut ranges: Vec<(u64, Vec<u8>)> = vec![(0, mbr0.to_vec())];
    if head[sector as usize..sector as usize + 8] == *b"EFI PART" {
        ranges.push((sector, vec![0u8; 33 * sector as usize])); // the empty GPT at the start
    }
    let last_end = plan.parts.iter().map(|p| (p.start_lba as u64 + p.sectors as u64) * sector).max().unwrap_or(0);
    let gpt_end = size - 33 * sector;
    if tail[(size - sector - tail_at) as usize..][..8] == *b"EFI PART" && gpt_end >= last_end {
        ranges.push((gpt_end, vec![0u8; 33 * sector as usize])); // its copy at the end
    }
    let info = format!("disk={}\nmodel={}\nsize={size}\nend_at={tail_at}\n{}",
        scan.disk, r.disks.iter().find(|d| d.dev == scan.disk).map_or("", |d| d.model.as_str()),
        ranges.iter().map(|(at, b)| format!("range={at}:{}\n", b.len())).collect::<String>());
    for (name, data) in [("disk-start.bin", &head[..]), ("disk-end.bin", &tail[..]), ("disk.txt", info.as_bytes())] {
        write_synced(&dir.join(name), data).map_err(|e| format!("can't save the backup ({name}): {e}; nothing written"))?;
        if fs::read(dir.join(name)).ok().as_deref() != Some(data) {
            return Err(format!("the backup ({name}) didn't read back the same; nothing written"));
        }
    }
    // 2. Write.
    for (at, data) in &ranges {
        disk.write_all_at(data, *at).map_err(|e| format!("writing {}: {e} (undo: doas nexa-bootfix undo-partitions {})", scan.disk, backup_to.display()))?;
    }
    disk.sync_all().map_err(|e| format!("writing {}: {e}", scan.disk))?;
    // 3. Check what's on the disk now.
    if read(0, 512)? != mbr0.to_vec() {
        return Err(format!("the new first sector didn't read back the same (undo: doas nexa-bootfix undo-partitions {})", backup_to.display()));
    }
    let seen = reread_partitions(&disk);
    let parts = plan.parts.iter().enumerate().map(|(i, p)| format!("{} {}{}", i + 1, p.what, if p.active { " (active)" } else { "" })).collect::<Vec<_>>().join(", ");
    Ok(vec![
        format!("Saved the disk's first and last MiB to {}", dir.display()),
        format!("Put back the partition list on {} (MBR, disk ID {:08X}): {parts}{}", scan.disk, plan.signature,
            if seen { "" } else { " (NexaOS sees it after a restart)" }),
        format!("To undo it: doas nexa-bootfix undo-partitions {} --yes", backup_to.display()),
    ])
}

/// Put back what restore_mbr replaced, from its backup folder.
fn undo_partitions(folder: &Path) -> Result<String, String> {
    let dir = if folder.join("backup/disk.txt").exists() { folder.join("backup") } else { folder.to_path_buf() };
    let info = fs::read_to_string(dir.join("disk.txt")).map_err(|e| format!("no backup in {}: {e}", dir.display()))?;
    let get = |k: &str| info.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(str::to_string);
    let dev = get("disk").ok_or("disk.txt: no disk")?;
    let size: u64 = get("size").and_then(|v| v.parse().ok()).ok_or("disk.txt: no size")?;
    let end_at: u64 = get("end_at").and_then(|v| v.parse().ok()).ok_or("disk.txt: no end_at")?;
    if sys_disk_bytes(&dev) != size {
        return Err(format!("{dev} isn't the disk this backup came from (its size differs); nothing written"));
    }
    let (head, tail) = (fs::read(dir.join("disk-start.bin")).map_err(|e| e.to_string())?, fs::read(dir.join("disk-end.bin")).map_err(|e| e.to_string())?);
    let disk = fs::OpenOptions::new().read(true).write(true).open(dev_root().join(&dev)).map_err(|e| format!("can't open {dev}: {e}"))?;
    let mut n = 0;
    for r in info.lines().filter_map(|l| l.strip_prefix("range=")) {
        let (at, len) = r.split_once(':').and_then(|(a, l)| Some((a.parse::<u64>().ok()?, l.parse::<usize>().ok()?))).ok_or("disk.txt: bad range")?;
        let src = if at + len as u64 <= head.len() as u64 { &head[at as usize..at as usize + len] }
            else if at >= end_at && (at - end_at) as usize + len <= tail.len() { &tail[(at - end_at) as usize..(at - end_at) as usize + len] }
            else { return Err("disk.txt: a range outside the backup".into()) };
        disk.write_all_at(src, at).map_err(|e| format!("writing {dev}: {e}"))?;
        n += 1;
    }
    disk.sync_all().map_err(|e| e.to_string())?;
    reread_partitions(&disk);
    Ok(format!("Put back {n} part(s) of {dev} as they were before."))
}

// ---------- the step-by-step run ----------

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    // No more input (the window closed, or a pipe ended): treat as "skip".
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => "s".into(),
        Ok(_) => line.trim().to_lowercase(),
    }
}

/// Offer each user's folders on the lost partitions, copy the chosen ones
/// to `folder`/Files. Returns the copy log.
fn rescue_files(r: &Report, folder: &Path, fat: bool) -> String {
    println!("\n=== Copy your files ===");
    println!("They're still on the internal disk. NexaOS only reads it: nothing there changes.\n");
    let sources: Vec<(&lost::Scan, usize, &lost::Found)> = r.lost.iter()
        .flat_map(|l| l.found.iter().enumerate().filter(|(_, f)| has_files(f)).map(move |(i, f)| (l, i, f))).collect();
    // Read-only, but shown as the desktop user's, so Files can open it.
    let desk = desktop::current();
    let mut opened = Vec::new();
    let mut mounted: Vec<(PathBuf, usize, &lost::Found)> = Vec::new();
    for (l, i, f) in &sources {
        let sector = r.disks.iter().find(|d| d.dev == l.disk).map_or(512, |d| d.sector);
        let at = work_dir().join(if f.what == "Windows (C:)" { format!("Windows-C-{}-{i}", l.disk) } else { format!("drive-{}-{i}", l.disk) });
        let opts = desk.as_ref().map(|d| desktop::owner_options(&f.fs, &d.user));
        match open_lost(&dev_root().join(&l.disk), sector, f, &at, opts.as_deref()) {
            Ok(()) => {
                mounted.push((at.clone(), *i, f));
                opened.push(at);
            }
            Err(e) => println!("Can't open {}: {e}", f.what),
        }
    }
    let close = |opened: &[PathBuf]| opened.iter().for_each(|m| umount(m));
    if mounted.is_empty() {
        return "The partitions with files couldn't be opened.\n".into();
    }
    if let Some(d) = &desk {
        println!("How do you want to choose what to copy?");
        println!("  1  Open your Windows drive in Files and pick the files yourself");
        println!("  2  Pick whole folders here, by number");
        let a = loop {
            let a = ask("Type 1 or 2 and Enter: ");
            if a == "1" || a == "2" || a == "s" {
                break a;
            }
        };
        if a == "1" {
            let drive_root = folder.parent().unwrap_or(folder);
            match pick_in_files(d, &mounted, drive_root, folder) {
                Ok(log) => {
                    close(&opened);
                    return log;
                }
                Err(e) => println!("Files didn't open ({e}). Pick whole folders instead:\n"),
            }
        }
    }
    let mut list: Vec<lost::Item> = Vec::new();
    for (at, i, f) in &mounted {
        println!("Reading {} ({})...", f.what, lost::human(f.size));
        for mut it in lost::items(at, f.what == "Windows (C:)") {
            if it.label == "the whole drive" {
                it.label = format!("{} ({}) - all of it", f.what, lost::human(f.size));
            }
            if sources.len() > 1 {
                it.dest = Path::new(&format!("partition-{}", i + 1)).join(&it.dest);
            }
            list.push(it);
        }
    }
    if list.is_empty() {
        println!("No files found in the user folders.");
        close(&opened);
        return "No files found in the user folders.\n".into();
    }
    println!();
    for (i, it) in list.iter().enumerate() {
        println!("  {:>2}. {:<44} {:>9}  ({} file{})", i + 1, it.label, lost::human(it.bytes), it.files, if it.files == 1 { "" } else { "s" });
    }
    let free = free_mb(folder).unwrap_or(0) << 20;
    println!("\nFree on the USB drive: {}{}", lost::human(free), if fat { " (FAT32: files of 4 GB or more can't go on it)" } else { "" });
    let chosen: Vec<usize> = loop {
        let a = ask("Type the numbers to copy (e.g. 1 3 4) or all, then Enter. (Enter alone: don't copy.) ");
        if a.is_empty() || ["s", "exit", "q", "quit", "n", "no"].contains(&a.as_str()) {
            break vec![];
        }
        let mut pick: Vec<usize> = if a == "all" {
            (0..list.len()).collect()
        } else {
            a.split(|c: char| c == ' ' || c == ',').filter_map(|w| w.parse::<usize>().ok()).filter(|n| (1..=list.len()).contains(n)).map(|n| n - 1).collect()
        };
        pick.sort();
        pick.dedup();
        if pick.is_empty() {
            println!("Type numbers from the list, or all.");
            continue;
        }
        let total: u64 = pick.iter().map(|i| list[*i].bytes).sum();
        if total + total / 50 + (16 << 20) > free {
            println!("That's {}, more than the {} free on the USB drive. Pick fewer, or use a bigger drive (exFAT or NTFS for files over 4 GB).",
                lost::human(total), lost::human(free));
            continue;
        }
        break pick;
    };
    if chosen.is_empty() {
        close(&opened);
        println!("No files copied.");
        return "No files copied (none chosen).\n".into();
    }
    let total: u64 = chosen.iter().map(|i| list[*i].bytes).sum();
    let to = folder.join("Files");
    let mut done = lost::Copied::default();
    let mut log = String::new();
    for i in &chosen {
        let it = &list[*i];
        println!("Copying {}...", it.label);
        let mut next = 0u64;
        lost::copy_into(&it.path, &to.join(&it.dest), fat, &mut done, &mut |c| {
            if c.bytes >= next {
                next = c.bytes + (64 << 20);
                print!("\r  {} of {} ({} files)   ", lost::human(c.bytes), lost::human(total), c.files);
                let _ = std::io::stdout().flush();
            }
        });
        println!();
        log += &format!("{} -> Files/{}\n", it.label, it.dest.display());
    }
    sync();
    close(&opened);
    println!("\nCOPIED: {} files, {} to Files on the USB drive.", done.files, lost::human(done.bytes));
    log += &format!("\nCopied {} files, {}.\n", done.files, lost::human(done.bytes));
    if !done.failed.is_empty() {
        println!("NOT COPIED: {} file(s) (the list is in files-copied.txt):", done.failed.len());
        for f in done.failed.iter().take(8) {
            println!("  {f}");
        }
        log += &format!("\nNot copied ({}):\n{}\n", done.failed.len(), done.failed.join("\n"));
    }
    log
}

/// Open the lost partitions in Files as the desktop user and let them copy
/// what they want to the USB drive; then tell what landed on the drive.
fn pick_in_files(d: &desktop::Desktop, mounted: &[(PathBuf, usize, &lost::Found)], drive_root: &Path, report: &Path) -> Result<String, String> {
    // Start in the user's own folder when there's just one.
    let (first, _, f) = &mounted[0];
    let start = if f.what == "Windows (C:)" {
        let users = lost::users(first);
        match (find_ci(first, &["Users"]), users.as_slice()) {
            (Some(u), [one]) => u.join(one),
            (Some(u), _) => u,
            _ => first.clone(),
        }
    } else {
        first.clone()
    };
    let before = desktop::file_set(drive_root, report);
    let mut child = desktop::launch(d, "/usr/local/bin/nexa-files", &[&start]).map_err(|e| e.to_string())?;
    println!("\nFiles is open on your Windows drive. It's read-only: nothing on it can change.");
    println!("  1. Select what you want (Ctrl+click picks several files or folders).");
    println!("  2. Copy (Ctrl+C), click the USB drive in Files' side bar, then Paste (Ctrl+V).");
    println!("  3. Wait until the copying has finished, then close Files.");
    for (at, _, f) in mounted.iter().skip(1) {
        println!("  (Also read-only: {} at {})", f.what, at.display());
    }
    println!("The USB drive is {}.", drive_root.display());
    loop {
        let a = ask("\nWhen you're done, close Files and press Enter here. ");
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        if a == "s" {
            break; // no more input: go on, Files keeps what it already copied
        }
        println!("Files is still open. Let any copy finish, close Files, then press Enter.");
    }
    sync();
    let new = desktop::added(&before, &desktop::file_set(drive_root, report));
    let bytes: u64 = new.iter().map(|(_, n)| n).sum();
    println!("\nCOPIED: {} files, {} are new on the USB drive.", new.len(), lost::human(bytes));
    let mut log = format!("Picked in Files. New on the USB drive: {} files, {}.\n\n", new.len(), lost::human(bytes));
    for (p, n) in &new {
        log += &format!("{}  ({})\n", p.display(), lost::human(*n));
    }
    Ok(log)
}

fn interactive() -> i32 {
    println!("NexaOS boot check for Windows\n=============================\n");
    println!("This looks at the internal disk's boot files, the firmware's boot entries and the");
    println!("Secure Boot keys. Nothing is changed unless you say yes.\n");
    let before = check();
    println!("{}", text(&before));
    // Where to save: a second USB drive.
    let mut drive = None;
    loop {
        let ds = drives();
        if let Some(d) = ds.first() {
            println!("Saving to the USB drive {} ({}{}, {} MB).", d.dev, d.fs, if d.label.is_empty() { String::new() } else { format!(", \"{}\"", d.label) }, d.size_mb);
            drive = Some(d.clone());
            break;
        }
        let a = ask("\nPlug in a second USB drive (FAT32, exFAT or NTFS) to save the report and a backup,\nthen press Enter. (Type s and Enter to go on without saving.) ");
        if a == "s" {
            break;
        }
    }
    let opened = drive.as_ref().map(open_drive);
    let (folder, undo) = match opened {
        Some(Ok((root, undo))) => {
            let f = root.join(format!("NexaOS-bootfix-{}", now().0));
            match fs::create_dir_all(&f) {
                Ok(()) => (Some(f), undo),
                Err(e) => {
                    println!("Can't write to the drive: {e}");
                    (None, undo)
                }
            }
        }
        Some(Err(e)) => {
            println!("{e}");
            (None, None)
        }
        None => (None, None),
    };
    let save = |name: &str, data: &[u8]| {
        if let Some(f) = &folder {
            let _ = write_synced(&f.join(name), data);
        }
    };
    save("report-before.txt", text(&before).as_bytes());
    save("report-before.json", serde_json::to_string_pretty(&before).unwrap_or_default().as_bytes());
    if before.lost.iter().any(|l| l.found.iter().any(has_files)) {
        match (&folder, &drive) {
            (Some(f), Some(d)) => {
                let log = rescue_files(&before, f, d.fs == "vfat");
                save("files-copied.txt", log.as_bytes());
            }
            _ => println!("\nYour files can be copied to a USB drive: run Windows Boot Repair again with one plugged in."),
        }
    }
    let fixes: Vec<&Finding> = before.findings.iter().filter(|f| f.fix.is_some()).collect();
    let mut changed = false;
    if !fixes.is_empty() {
        println!("\nNexaOS can fix:");
        for f in &fixes {
            println!("  - {}", f.title);
        }
        let backup_root = folder.clone().unwrap_or_else(|| work_dir().join(format!("backup-{}", now().0)));
        if folder.is_none() {
            println!("(No USB drive: the backup goes to memory and is lost when the PC turns off.)");
        }
        let restore = fixes.iter().any(|f| f.fix.as_deref() == Some("restore-mbr"));
        if restore {
            println!("\nPutting the partition list back WRITES TO THE INTERNAL DISK: its first sector and the empty");
            println!("partition list at its start and end (your files and Windows itself aren't touched). Do it only");
            println!("after your files are copied to a USB drive and you've opened them on another computer.");
            println!("The disk's first and last MiB are saved to the USB drive first; it can be undone.");
        }
        let a = if restore && folder.is_none() {
            println!("A USB drive is needed for the backup: nothing changed. Run Windows Boot Repair again with one.");
            String::new()
        } else {
            ask(if restore { "\nAre your files safe, and do you want to put the partition list back? Type yes and Enter: " }
                else { "\nThe whole boot partition and the boot entries are backed up first.\nApply these fixes? Type yes and Enter: " })
        };
        if a == "yes" || a == "y" {
            match fix(&before, &backup_root) {
                Ok(done) => {
                    changed = true;
                    println!();
                    for d in &done {
                        println!("DONE: {d}");
                    }
                    save("fixes-done.txt", (done.join("\n") + "\n").as_bytes());
                }
                Err(e) => {
                    println!("\nNOT FIXED: {e}");
                    save("fixes-done.txt", format!("failed: {e}\n").as_bytes());
                }
            }
        } else {
            println!("Nothing changed.");
        }
    }
    let after = if changed {
        let a = check();
        println!("\n=== After the fixes ===\n{}", text(&a));
        save("report-after.txt", text(&a).as_bytes());
        save("report-after.json", serde_json::to_string_pretty(&a).unwrap_or_default().as_bytes());
        a
    } else {
        before
    };
    let faults: Vec<&Finding> = after.findings.iter().filter(|f| f.level == "fault").collect();
    println!("\n=============================");
    if faults.is_empty() {
        println!("No fault left in Windows' boot files. Restart and choose Windows Boot Manager.");
    } else {
        println!("Still to fix (see the steps above):");
        for f in &faults {
            println!("  - {}", f.title);
        }
    }
    if before_lost_files(&after) {
        println!("\nAfter your files are copied, check them on another computer before installing anything on this disk.");
    }
    if let Some(f) = &folder {
        std::thread::sleep(std::time::Duration::from_millis(400));
        match screenshot(&f.join("screenshot.png")) {
            Ok(()) => println!("Screenshot saved."),
            Err(e) => println!("(No screenshot: {e})"),
        }
        sync();
        println!("Report saved on the USB drive in {}", f.file_name().unwrap_or_default().to_string_lossy());
    }
    if let Some(u) = undo {
        sync();
        umount(&u);
        println!("The USB drive can be removed now.");
    }
    ask("\nPress Enter to close. ");
    0
}

fn before_lost_files(r: &Report) -> bool {
    r.lost.iter().any(|l| l.found.iter().any(has_files))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let is_root = unsafe { libc::geteuid() } == 0;
    let need_root = || {
        if !is_root && std::env::var_os("NEXA_BOOTFIX_SYS").is_none() {
            eprintln!("nexa-bootfix needs root: doas nexa-bootfix");
            std::process::exit(1);
        }
    };
    match args.first().map(String::as_str) {
        None => {
            need_root();
            std::process::exit(interactive());
        }
        Some("check") => {
            need_root();
            let r = check();
            if args.iter().any(|a| a == "--json") {
                println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            } else {
                print!("{}", text(&r));
            }
        }
        Some("fix") => {
            need_root();
            if !args.iter().any(|a| a == "--yes") {
                eprintln!("nexa-bootfix fix changes the boot partition and boot entries (after a backup); add --yes");
                std::process::exit(2);
            }
            let to = args.iter().position(|a| a == "--to").and_then(|i| args.get(i + 1)).map(PathBuf::from)
                .unwrap_or_else(|| work_dir().join(format!("backup-{}", now().0)));
            let before = check();
            let _ = fs::create_dir_all(&to);
            let _ = write_synced(&to.join("report-before.txt"), text(&before).as_bytes());
            match fix(&before, &to) {
                Ok(done) => {
                    for d in &done {
                        println!("DONE: {d}");
                    }
                    if done.is_empty() {
                        println!("Nothing to fix here.");
                    }
                    let after = check();
                    let _ = write_synced(&to.join("report-after.txt"), text(&after).as_bytes());
                    print!("\n{}", text(&after));
                }
                Err(e) => {
                    println!("NOT FIXED: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some("undo-partitions") => {
            need_root();
            let (Some(dir), true) = (args.get(1), args.iter().any(|a| a == "--yes")) else {
                eprintln!("usage: doas nexa-bootfix undo-partitions FOLDER --yes  (FOLDER: the NexaOS-bootfix-... folder with backup/disk.txt)");
                std::process::exit(2);
            };
            match undo_partitions(Path::new(dir)) {
                Ok(m) => println!("{m}"),
                Err(e) => {
                    eprintln!("NOT DONE: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some("screenshot") => {
            need_root();
            match screenshot(Path::new(args.get(1).map(String::as_str).unwrap_or("screenshot.png"))) {
                Ok(()) => println!("saved"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Some("drives") => {
            need_root();
            println!("{}", json!(drives()));
        }
        _ => {
            eprintln!("usage: doas nexa-bootfix | nexa-bootfix check [--json] | fix --yes [--to DIR] | drives");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Report {
        Report {
            time: "t".into(), uefi: true, disks: vec![], windows: vec![], boot_order: vec![], boot_current: None,
            keys: secureboot::Keys { uefi: true, setup_mode: Some(false), secure_boot: Some(false), ..Default::default() },
            lost: vec![],
            legacy: vec![],
            esps: vec![Esp {
                dev: "nvme0n1p1".into(), disk: "nvme0n1".into(), guid: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(), number: 1,
                first_lba: 2048, sectors: 204800, readable: true, has_microsoft_boot: true,
                bootmgr: Some(pe::inspect(&pe::tests::fake_pe(&["Microsoft Windows Production PCA 2011"]))), bootmgr_ca: "2011".into(),
                bcd: Some((32768, true)), fallback: Some(pe::inspect(&pe::tests::fake_pe(&["Microsoft Windows Production PCA 2011"]))),
                free_mb: Some(60), ..Default::default()
            }],
            entries: vec![efi::Entry { id: "0001".into(), description: "Windows Boot Manager".into(), active: true,
                partition_guid: Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()), partition_number: Some(1),
                file: Some(efi::WINDOWS_PATH.into()), in_boot_order: true }],
            findings: vec![],
        }
    }

    fn ids(r: &Report) -> Vec<(String, Option<String>)> {
        findings(r).into_iter().filter(|f| f.level != "info").map(|f| (f.title, f.fix)).collect()
    }

    #[test]
    fn healthy_pc_has_no_faults() {
        let f = findings(&base());
        assert!(f.iter().all(|f| f.level == "info"), "{f:?}");
        assert!(f[0].title.starts_with("No fault found"));
    }

    #[test]
    fn faults_and_their_fixes() {
        // The boot manager is cut short, Windows' copy is fine: fixable.
        let mut r = base();
        r.esps[0].bootmgr = Some(pe::inspect(&[]));
        r.windows.push(WinDrive { dev: "nvme0n1p3".into(), fs: "ntfs".into(), readable: true, has_windows: true, winload: true,
            bootmgr_copy: Some(pe::inspect(&pe::tests::fake_pe(&["Windows UEFI CA 2023"]))), bootmgr_copy_ca: "2023".into(), ..Default::default() });
        let f = ids(&r);
        assert_eq!(f[0].1.as_deref(), Some("restore-bootmgr"), "{f:?}");
        // Same, but Windows is locked by BitLocker: explained, not fixable here.
        r.windows[0] = WinDrive { dev: "nvme0n1p3".into(), fs: "bitlocker".into(), error: Some("locked by BitLocker".into()), ..Default::default() };
        let f = findings(&r);
        assert!(f[0].fix.is_none() && f[0].detail.contains("bcdboot"), "{f:?}");
        assert!(f.iter().any(|x| x.title.contains("BitLocker")));
        // The Windows entry points at a partition that's gone.
        let mut r = base();
        r.entries[0].partition_guid = Some("12345678-0000-0000-0000-000000000000".into());
        assert_eq!(ids(&r)[0].1.as_deref(), Some("boot-entry"));
        r.entries.clear();
        assert!(ids(&r)[0].0.contains("no \"Windows Boot Manager\" entry"));
        // BCD gone: bcdboot steps.
        let mut r = base();
        r.esps[0].bcd = None;
        let f = findings(&r);
        assert!(f[0].title.contains("BCD") && f[0].detail.contains("bcdboot"));
        // Secure Boot on, boot manager signed with the 2023 key, db has only 2011.
        let mut r = base();
        r.keys.secure_boot = Some(true);
        r.keys.db = vec!["Microsoft Windows Production PCA 2011".into()];
        r.keys.trusts_windows = true;
        r.esps[0].bootmgr = Some(pe::inspect(&pe::tests::fake_pe(&["Windows UEFI CA 2023"])));
        r.esps[0].bootmgr_ca = "2023".into();
        assert!(ids(&r)[0].0.contains("Secure Boot refuses"), "{:?}", ids(&r));
        r.keys.trusts_windows_2023 = true;
        assert!(ids(&r).is_empty());
        // Cleared keys show as a warning (Secure Boot isn't enforced then).
        let mut r = base();
        r.keys.setup_mode = Some(true);
        r.keys.problems = vec![secureboot::Problem { id: "secureboot-cleared".into(), title: "keys cleared".into(), detail: "d".into() }];
        assert_eq!(findings(&r).iter().filter(|f| f.level == "warning").count(), 1);
        // No fallback: offered.
        let mut r = base();
        r.esps[0].fallback = None;
        assert_eq!(ids(&r)[0].1.as_deref(), Some("restore-fallback"));
        assert!(text(&r).contains("Fallback"));
    }

    #[test]
    fn deleted_partitions() {
        let mut r = base();
        r.esps.clear();
        r.entries.clear();
        let c = lost::Found { start: 1 << 30, size: 200 << 30, fs: "ntfs".into(), what: "Windows (C:)".into(), users: vec!["Bless".into()], from: "scan".into(), ..Default::default() };
        r.lost = vec![lost::Scan { disk: "nvme0n1".into(), size: 256 << 30, sector: 512, checked: 300, with_data: 200, found: vec![c] }];
        let f = findings(&r);
        assert!(f[0].title.contains("still on it") && f[0].detail.contains("Bless") && f[0].detail.contains("copy them"), "{f:?}");
        // The misleading "no EFI partition" step (bcdboot can't help) is left out.
        assert!(!f.iter().any(|x| x.title.contains("No EFI")));
        assert!(text(&r).contains("Lost partitions on nvme0n1"));
        // UEFI Windows: nothing to put back.
        assert!(f[0].fix.is_none());
        // The ThinkPad: Windows in Legacy BIOS mode, its BCD agrees: offered.
        let mut tp = mbr::tests::thinkpad();
        tp[1].users = vec!["Lenovo".into()];
        let mut t = base();
        t.esps.clear();
        t.lost = vec![lost::Scan { disk: "nvme0n1".into(), size: 256060514304, sector: 512, checked: 6, with_data: 3, found: tp }];
        let f = findings(&t);
        assert_eq!(f[0].fix.as_deref(), Some("restore-mbr"), "{f:?}");
        assert!(f[0].detail.contains("A76F4909") && f[0].detail.contains("Legacy BIOS"));
        // ... but not when the BCD names another partition.
        t.lost[0].found[0].bcd_offsets.push(7 << 30);
        let f = findings(&t);
        assert!(f[0].fix.is_none() && f[0].detail.contains("won't put"), "{f:?}");
        // Blank (trimmed) disk: said plainly.
        r.lost[0].found.clear();
        r.lost[0].with_data = 0;
        assert!(findings(&r)[0].title.contains("is blank"));
        // Some data, nothing recognised.
        r.lost[0].with_data = 50;
        assert!(findings(&r)[0].detail.contains("PhotoRec"));
        // A blank second disk next to a working Windows: only a note.
        let mut ok = base();
        ok.lost = r.lost.clone();
        ok.lost[0].with_data = 0;
        assert!(findings(&ok).iter().all(|f| f.level != "fault"));
    }

    #[test]
    fn legacy_windows() {
        let mut r = base();
        r.esps.clear();
        r.entries.clear();
        r.legacy = vec![Legacy { dev: "nvme0n1p1".into(), disk: "nvme0n1".into(), active: true, disk_signature: 0xa76f4909, bcd_signature: Some(0xa76f4909) }];
        let f = findings(&r);
        // No "no EFI partition" fault: it starts the old way; the BIOS steps are given.
        assert!(!f.iter().any(|x| x.level == "fault"), "{f:?}");
        assert!(f.iter().any(|x| x.title.contains("Legacy BIOS") && x.detail.contains("CSM Support")));
        r.legacy[0].bcd_signature = Some(1);
        assert!(findings(&r)[0].title.contains("disk ID"));
        r.legacy[0].active = false;
        assert!(findings(&r)[0].title.contains("active"));
        assert!(text(&r).contains("Legacy BIOS boot on nvme0n1p1"));
    }

    #[test]
    fn dates() {
        let (a, b) = now();
        assert_eq!(a.len(), 15);
        assert!(b.ends_with("UTC") && b.starts_with("20"));
    }
}
