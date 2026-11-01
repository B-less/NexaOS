//! Windows partitions still on a disk whose partition list is empty (they
//! were deleted in the Windows installer, or with diskpart clean): found by
//! their boot sectors, opened read-only, and their files copied to another
//! drive. Nothing here writes to the scanned disk: it's opened read-only,
//! and so is the loop device laid over a found partition.

use serde::Serialize;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

/// Windows (Vista and later) starts partitions on 1 MiB boundaries.
pub const STEP: u64 = 1 << 20;

#[derive(Debug, Serialize, Clone, PartialEq, Default)]
pub struct Found {
    /// Where it starts and how long it is, in bytes (size 0: unknown).
    pub start: u64,
    pub size: u64,
    /// "ntfs", "bitlocker", "vfat", "exfat"
    pub fs: String,
    pub label: String,
    /// What's inside: "Windows (C:)", "Windows recovery", "EFI boot partition",
    /// "data drive", "locked by BitLocker", or why it can't be opened.
    pub what: String,
    /// User folders under \Users (a Windows partition).
    pub users: Vec<String>,
    /// "scan" or "backup partition table"
    pub from: String,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct Scan {
    pub disk: String,
    pub size: u64,
    /// 1 MiB steps read, and how many of them weren't blank.
    pub checked: u64,
    pub with_data: u64,
    pub found: Vec<Found>,
}

impl Scan {
    /// Share of the disk that isn't blank, in percent.
    pub fn data_percent(&self) -> u64 {
        if self.checked == 0 { 0 } else { self.with_data * 100 / self.checked }
    }
}

fn u16at(b: &[u8], o: usize) -> u64 { u16::from_le_bytes([b[o], b[o + 1]]) as u64 }
fn u32at(b: &[u8], o: usize) -> u64 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as u64 }
fn u64at(b: &[u8], o: usize) -> u64 { u64::from_le_bytes(b[o..o + 8].try_into().unwrap()) }

/// (filesystem, size in bytes or 0 when unknown, label) when `b` starts with
/// a volume boot sector.
pub fn boot_sector(b: &[u8]) -> Option<(&'static str, u64, String)> {
    if b.len() < 512 || b[510] != 0x55 || b[511] != 0xaa {
        return None;
    }
    let at = |o: usize, s: &[u8]| b.get(o..o + s.len()) == Some(s);
    if at(3, b"EXFAT   ") {
        let shift = b[0x6c] as u32;
        return (9..=12).contains(&shift).then(|| ("exfat", u64at(b, 0x48).checked_shl(shift).unwrap_or(0), String::new()));
    }
    let bps = u16at(b, 0x0b);
    if ![512, 1024, 2048, 4096].contains(&bps) || b[0x0d] == 0 {
        return None;
    }
    if at(3, b"NTFS    ") {
        // The volume, then its backup boot sector.
        let size = u64at(b, 0x28).checked_add(1)?.checked_mul(bps)?;
        return (size > bps).then(|| ("ntfs", size, String::new()));
    }
    if at(3, b"-FVE-FS-") {
        return Some(("bitlocker", u64at(b, 0x28).checked_mul(bps).unwrap_or(0), String::new()));
    }
    if at(0x52, b"FAT32   ") {
        let total = if u16at(b, 0x13) != 0 { u16at(b, 0x13) } else { u32at(b, 0x20) };
        let label = String::from_utf8_lossy(&b[0x47..0x52]).trim().to_string();
        return (total > 0).then(|| ("vfat", total * bps, if label == "NO NAME" { String::new() } else { label }));
    }
    None
}

/// Look for partitions on every 1 MiB boundary (skipping over the ones it
/// finds). Returns (found, steps read, steps not blank).
pub fn scan(read_at: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>, disk_bytes: u64, progress: &mut dyn FnMut(u64)) -> (Vec<Found>, u64, u64) {
    let (mut found, mut checked, mut with_data) = (Vec::new(), 0u64, 0u64);
    // The first MiB holds the partition list itself.
    let mut pos = STEP;
    let mut told = u64::MAX;
    while pos + 512 <= disk_bytes {
        let pct = pos * 100 / disk_bytes;
        if pct / 5 != told {
            told = pct / 5;
            progress(pct);
        }
        let Some(b) = read_at(pos, 4096.min(disk_bytes - pos) as usize) else {
            pos += STEP;
            continue;
        };
        checked += 1;
        if b.iter().any(|&x| x != 0) {
            with_data += 1;
        }
        if let Some((fs, size, label)) = boot_sector(&b) {
            if size <= disk_bytes - pos {
                found.push(Found { start: pos, size, fs: fs.into(), label, from: "scan".into(), ..Default::default() });
                if size > 0 {
                    pos = (pos + size).div_ceil(STEP) * STEP;
                    continue;
                }
            }
        }
        pos += STEP;
    }
    progress(100);
    (found, checked, with_data)
}

// ---------- a read-only loop device over a found partition ----------

const LOOP_CTL_GET_FREE: u64 = 0x4c82;
const LOOP_CONFIGURE: u64 = 0x4c0a;
const LO_FLAGS_READ_ONLY: u32 = 1;
const LO_FLAGS_AUTOCLEAR: u32 = 4;

#[repr(C)]
struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdevice: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; 64],
    lo_crypt_name: [u8; 64],
    lo_encrypt_key: [u8; 32],
    lo_init: [u64; 2],
}

#[repr(C)]
struct LoopConfig {
    fd: u32,
    block_size: u32,
    info: LoopInfo64,
    reserved: [u64; 8],
}

/// A read-only loop device showing `size` bytes of `disk` from `offset`
/// (size 0: to the end). It goes away by itself once it's unmounted and
/// the returned handle is closed.
pub fn attach(dev_dir: &Path, disk: &Path, offset: u64, size: u64, sector: u64) -> Result<(PathBuf, fs::File), String> {
    // Read-only: the kernel then makes the loop device read-only as well.
    let backing = fs::File::open(disk).map_err(|e| format!("can't read {}: {e}", disk.display()))?;
    let ctl = fs::OpenOptions::new().read(true).write(true).open(dev_dir.join("loop-control")).map_err(|e| format!("no loop devices: {e}"))?;
    let mut last = String::new();
    for _ in 0..4 {
        // SAFETY: LOOP_CTL_GET_FREE takes no argument and returns a number.
        let n = unsafe { libc::ioctl(ctl.as_raw_fd(), LOOP_CTL_GET_FREE as _) };
        if n < 0 {
            return Err(format!("no free loop device: {}", std::io::Error::last_os_error()));
        }
        let dev = dev_dir.join(format!("loop{n}"));
        let mut lo = None;
        for _ in 0..30 {
            if let Ok(f) = fs::File::open(&dev) {
                lo = Some(f);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let lo = lo.ok_or_else(|| format!("{} didn't appear", dev.display()))?;
        // SAFETY: an all-zero loop_config is valid; fields are filled below.
        let mut cfg: LoopConfig = unsafe { std::mem::zeroed() };
        cfg.fd = backing.as_raw_fd() as u32;
        cfg.block_size = if sector > 512 { sector as u32 } else { 0 };
        cfg.info.lo_offset = offset;
        cfg.info.lo_sizelimit = size;
        cfg.info.lo_flags = LO_FLAGS_READ_ONLY | LO_FLAGS_AUTOCLEAR;
        // SAFETY: LOOP_CONFIGURE reads a struct loop_config.
        if unsafe { libc::ioctl(lo.as_raw_fd(), LOOP_CONFIGURE as _, &cfg as *const LoopConfig) } == 0 {
            return Ok((dev, lo));
        }
        // Another program took this one first: ask again.
        last = std::io::Error::last_os_error().to_string();
    }
    Err(format!("can't set up a loop device: {last}"))
}

// ---------- copying files out ----------

/// User folders that are Windows' own, not someone's files.
const NOT_USERS: [&str; 5] = ["all users", "default", "default user", "defaultuser0", "defaultuser100000"];
/// Inside a user folder: settings and caches (big, and useless elsewhere).
const NOT_FILES: [&str; 3] = ["appdata", "application data", "local settings"];
/// Anywhere: Windows' own bookkeeping.
const SKIP: [&str; 3] = ["system volume information", "$recycle.bin", "$windows.~bt"];

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// e.g. "Bless - Documents"
    pub label: String,
    pub path: PathBuf,
    /// Where it goes under the copy folder, e.g. "Bless/Documents"
    pub dest: PathBuf,
    pub bytes: u64,
    pub files: u64,
}

fn lower(p: &fs::DirEntry) -> String {
    p.file_name().to_string_lossy().to_lowercase()
}

/// Real folders only (Windows' junctions show up as symlinks).
fn subdirs(p: &Path) -> Vec<fs::DirEntry> {
    let mut v: Vec<fs::DirEntry> = fs::read_dir(p).into_iter().flatten().flatten()
        .filter(|e| e.file_type().map_or(false, |t| t.is_dir())).collect();
    v.sort_by_key(|e| e.file_name());
    v
}

/// The people with a folder under \Users.
pub fn users(root: &Path) -> Vec<String> {
    let Some(users) = fs::read_dir(root).into_iter().flatten().flatten().find(|e| lower(e) == "users") else { return vec![] };
    subdirs(&users.path()).iter().filter(|u| !NOT_USERS.contains(&lower(u).as_str()) && lower(u) != "public")
        .map(|u| u.file_name().to_string_lossy().into_owned()).collect()
}

/// Bytes and files under a folder (not following links).
pub fn tree_size(p: &Path) -> (u64, u64) {
    let (mut bytes, mut files) = (0, 0);
    for e in fs::read_dir(p).into_iter().flatten().flatten() {
        if SKIP.contains(&lower(&e).as_str()) {
            continue;
        }
        match e.file_type() {
            Ok(t) if t.is_dir() => {
                let (b, f) = tree_size(&e.path());
                bytes += b;
                files += f;
            }
            Ok(t) if t.is_file() => {
                bytes += e.metadata().map_or(0, |m| m.len());
                files += 1;
            }
            _ => {}
        }
    }
    (bytes, files)
}

/// The folders worth copying from a mounted partition: each user's folders
/// (Desktop, Documents, Pictures, ...) on a Windows partition, the whole
/// drive otherwise. Empty ones are left out.
pub fn items(root: &Path, windows: bool) -> Vec<Item> {
    let mut out = Vec::new();
    let users = fs::read_dir(root).into_iter().flatten().flatten().find(|e| lower(e) == "users").map(|e| e.path());
    match users {
        Some(users) if windows => {
            for u in subdirs(&users).into_iter().filter(|u| !NOT_USERS.contains(&lower(u).as_str())) {
                let user = u.file_name().to_string_lossy().into_owned();
                for f in subdirs(&u.path()).into_iter().filter(|f| !NOT_FILES.contains(&lower(f).as_str())) {
                    let name = f.file_name().to_string_lossy().into_owned();
                    let (bytes, files) = tree_size(&f.path());
                    if files > 0 {
                        out.push(Item { label: format!("{user} - {name}"), path: f.path(), dest: Path::new(&user).join(&name), bytes, files });
                    }
                }
            }
        }
        _ => {
            let (bytes, files) = tree_size(root);
            if files > 0 {
                out.push(Item { label: "the whole drive".into(), path: root.into(), dest: "drive".into(), bytes, files });
            }
        }
    }
    out
}

#[derive(Debug, Default)]
pub struct Copied {
    pub bytes: u64,
    pub files: u64,
    /// "path: why" for each file that couldn't be copied.
    pub failed: Vec<String>,
}

fn copy_file(from: &Path, to: &Path, len: u64) -> std::io::Result<()> {
    let mut src = fs::File::open(from)?;
    let mut dst = fs::File::create(to)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut left = len;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])?;
        left = left.saturating_sub(n as u64);
    }
    if left > 0 {
        return Err(std::io::Error::other("the file was cut short while reading it"));
    }
    if let Ok(t) = fs::metadata(from).and_then(|m| m.modified()) {
        let _ = dst.set_modified(t);
    }
    Ok(())
}

/// Copy a folder tree, going on past files that fail. `fat`: the target
/// can't hold files of 4 GiB or more.
pub fn copy_into(from: &Path, to: &Path, fat: bool, done: &mut Copied, progress: &mut dyn FnMut(&Copied)) {
    if let Err(e) = fs::create_dir_all(to) {
        done.failed.push(format!("{}: {e}", to.display()));
        return;
    }
    let mut entries: Vec<fs::DirEntry> = match fs::read_dir(from) {
        Ok(r) => r.flatten().collect(),
        Err(e) => {
            done.failed.push(format!("{}: {e}", from.display()));
            return;
        }
    };
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if SKIP.contains(&lower(&e).as_str()) {
            continue;
        }
        let dst = to.join(e.file_name());
        match e.file_type() {
            Ok(t) if t.is_dir() => copy_into(&e.path(), &dst, fat, done, progress),
            Ok(t) if t.is_file() => {
                let len = e.metadata().map_or(0, |m| m.len());
                if fat && len >= 1 << 32 {
                    done.failed.push(format!("{}: 4 GB or bigger, too big for a FAT32 drive (use an exFAT or NTFS drive)", e.path().display()));
                    continue;
                }
                match copy_file(&e.path(), &dst, len) {
                    Ok(()) => {
                        done.bytes += len;
                        done.files += 1;
                        progress(done);
                    }
                    Err(err) => {
                        let _ = fs::remove_file(&dst);
                        done.failed.push(format!("{}: {err}", e.path().display()));
                    }
                }
            }
            _ => {}
        }
    }
}

/// "1.2 GB", "340 MB", "12 KB"
pub fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{} MB", b >> 20),
        b => format!("{} KB", b.div_ceil(1024)),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn ntfs_sector(total: u64) -> Vec<u8> {
        let mut b = vec![0u8; 4096];
        b[3..11].copy_from_slice(b"NTFS    ");
        b[0x0b..0x0d].copy_from_slice(&512u16.to_le_bytes());
        b[0x0d] = 8;
        b[0x28..0x30].copy_from_slice(&total.to_le_bytes());
        b[510] = 0x55;
        b[511] = 0xaa;
        b
    }

    pub fn fat32_sector(total: u32, label: &str) -> Vec<u8> {
        let mut b = vec![0u8; 4096];
        b[0x0b..0x0d].copy_from_slice(&512u16.to_le_bytes());
        b[0x0d] = 1;
        b[0x20..0x24].copy_from_slice(&total.to_le_bytes());
        b[0x47..0x52].copy_from_slice(format!("{label:<11}").as_bytes());
        b[0x52..0x5a].copy_from_slice(b"FAT32   ");
        b[510] = 0x55;
        b[511] = 0xaa;
        b
    }

    #[test]
    fn boot_sectors() {
        assert_eq!(boot_sector(&ntfs_sector(2047)), Some(("ntfs", 2048 * 512, String::new())));
        assert_eq!(boot_sector(&fat32_sector(204800, "SYSTEM")), Some(("vfat", 204800 * 512, "SYSTEM".into())));
        let mut bl = ntfs_sector(0);
        bl[3..11].copy_from_slice(b"-FVE-FS-");
        assert_eq!(boot_sector(&bl).unwrap().0, "bitlocker");
        // No 55 AA, or a nonsense sector size: not a boot sector.
        let mut x = ntfs_sector(10);
        x[511] = 0;
        assert_eq!(boot_sector(&x), None);
        let mut y = ntfs_sector(10);
        y[0x0b] = 7;
        assert_eq!(boot_sector(&y), None);
        assert_eq!(boot_sector(&[0u8; 4096]), None);
    }

    #[test]
    fn finds_deleted_partitions() {
        // 64 MiB disk: FAT32 at 1 MiB (8 MiB), a gap, NTFS at 10 MiB to the
        // end, a fake NTFS sector inside it (a disk image file) to skip.
        let mib = STEP as usize;
        let mut disk = vec![0u8; 64 * mib];
        disk[mib..mib + 4096].copy_from_slice(&fat32_sector(16384, "SYSTEM"));
        disk[2 * mib] = 1; // file data
        disk[10 * mib..10 * mib + 4096].copy_from_slice(&ntfs_sector((54 * STEP / 512) - 1));
        disk[20 * mib..20 * mib + 4096].copy_from_slice(&ntfs_sector(100));
        let mut read = |at: u64, len: usize| disk.get(at as usize..at as usize + len).map(|s| s.to_vec());
        let mut seen = Vec::new();
        let (found, checked, with_data) = scan(&mut read, 64 * STEP, &mut |p| seen.push(p));
        assert_eq!(found.iter().map(|f| (f.start, f.fs.as_str())).collect::<Vec<_>>(), vec![(STEP, "vfat"), (10 * STEP, "ntfs")]);
        assert_eq!(found[1].size, 54 * STEP);
        assert_eq!(found[0].label, "SYSTEM");
        // Read: MiB 1 (FAT, skips to 9), 9 (blank), 10 (NTFS, skips to the end).
        assert_eq!(checked, 3);
        assert_eq!(with_data, 2);
        assert_eq!(seen.last(), Some(&100));
        // A blank (trimmed) disk: nothing found, no data.
        let blank = vec![0u8; 8 * mib];
        let mut read = |at: u64, len: usize| blank.get(at as usize..at as usize + len).map(|s| s.to_vec());
        let (f, c, d) = scan(&mut read, 8 * STEP, &mut |_| {});
        assert!(f.is_empty() && c == 7 && d == 0);
    }

    #[test]
    fn lists_and_copies_user_files() {
        let root = std::env::temp_dir().join(format!("nexa-lost-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let w = root.join("c");
        for d in ["Windows/System32", "Users/Bless/Documents/Work", "Users/Bless/AppData/Local", "Users/Bless/Pictures",
                  "Users/Bless/Music", "Users/Default/Documents", "Users/Public/Documents", "$Recycle.Bin/x"] {
            fs::create_dir_all(w.join(d)).unwrap();
        }
        fs::write(w.join("Users/Bless/Documents/Work/plan.txt"), b"plan").unwrap();
        fs::write(w.join("Users/Bless/Documents/cv.docx"), vec![7u8; 3000]).unwrap();
        fs::write(w.join("Users/Bless/AppData/Local/cache.bin"), b"cache").unwrap();
        fs::write(w.join("Users/Bless/Pictures/p.jpg"), b"jpg").unwrap();
        fs::write(w.join("Users/Default/Documents/d.txt"), b"d").unwrap();
        fs::write(w.join("$Recycle.Bin/x/old.txt"), b"old").unwrap();
        std::os::unix::fs::symlink(w.join("Users/Bless/Documents"), w.join("Users/Bless/My Documents")).unwrap();
        let it = items(&w, true);
        assert_eq!(it.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), vec!["Bless - Documents", "Bless - Pictures"]);
        assert_eq!((it[0].bytes, it[0].files), (3004, 2));
        let out = root.join("usb");
        let mut done = Copied::default();
        let mut calls = 0;
        for i in &it {
            copy_into(&i.path, &out.join(&i.dest), true, &mut done, &mut |_| calls += 1);
        }
        assert_eq!((done.files, done.bytes, calls), (3, 3007, 3));
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert_eq!(fs::read(out.join("Bless/Documents/Work/plan.txt")).unwrap(), b"plan");
        assert!(!out.join("Bless/AppData").exists());
        // A data drive: everything but the recycle bin.
        let d = items(&w, false);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].files, 5);
        assert_eq!(human(3 << 30), "3.0 GB");
        assert_eq!(human(5 << 20), "5 MB");
        assert_eq!(human(10), "1 KB");
        fs::remove_dir_all(&root).unwrap();
    }
}
