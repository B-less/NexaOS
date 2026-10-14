//! Removable USB and SD drives: list, mount, unmount, eject, plus the udev
//! hot-plug entry points (`nexa-brokerd drive-added|drive-removed NAME`).
//! Never trusts the name it is given: every device is re-checked against
//! the kernel's own sysfs view, only removable or USB-attached plain disks
//! are touched (never internal, NVMe, virtio or loop devices), and mount
//! points are confined to /media (nosuid, nodev, noexec).

use crate::policy::valid_kname;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FS_TYPES: &[&str] = &["vfat", "exfat", "ntfs3", "ext4", "udf", "iso9660"];

pub struct Tree {
    pub sys: PathBuf,
    pub dev: PathBuf,
    pub media: PathBuf,
    pub mounts: PathBuf,
    /// Where mount/umount/blkid/logger come from ("" = PATH); tests use fakes.
    pub tools: PathBuf,
}

impl Tree {
    pub fn real() -> Tree {
        Tree { sys: "/sys".into(), dev: "/dev".into(), media: "/media".into(), mounts: "/proc/mounts".into(), tools: PathBuf::new() }
    }

    fn tool(&self, name: &str) -> Command {
        Command::new(if self.tools.as_os_str().is_empty() { PathBuf::from(name) } else { self.tools.join(name) })
    }

    fn block(&self, k: &str) -> PathBuf {
        self.sys.join("class/block").join(k)
    }

    fn real_dir(&self, k: &str) -> PathBuf {
        fs::canonicalize(self.block(k)).unwrap_or_else(|_| self.block(k))
    }

    pub fn is_partition(&self, k: &str) -> bool {
        self.block(k).join("partition").exists()
    }

    pub fn disk_of(&self, k: &str) -> String {
        if self.is_partition(k) {
            self.real_dir(k).parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            k.to_string()
        }
    }

    pub fn partitions_of(&self, disk: &str) -> Vec<String> {
        let d = self.real_dir(disk);
        let mut v: Vec<String> = fs::read_dir(&d).into_iter().flatten().flatten()
            .filter(|e| e.path().join("partition").exists())
            .map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    /// Only removable disks and USB-attached ones.
    pub fn eligible(&self, k: &str) -> bool {
        if !valid_kname(k) {
            return false;
        }
        let disk = self.disk_of(k);
        let whole = disk.starts_with("sd") && disk[2..].chars().all(|c| c.is_ascii_lowercase())
            || disk.starts_with("mmcblk") && disk[6..].chars().all(|c| c.is_ascii_digit());
        if !whole {
            return false;
        }
        let dir = self.real_dir(&disk);
        dir.is_dir() && (fs::read_to_string(dir.join("removable")).map(|s| s.trim() == "1").unwrap_or(false)
                         || dir.to_string_lossy().contains("/usb"))
    }

    fn mount_table(&self) -> Vec<(String, String, String)> {
        fs::read_to_string(&self.mounts).unwrap_or_default().lines().filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 3).then(|| (f[0].to_string(), f[1].replace("\\040", " "), f[2].to_string()))
        }).collect()
    }

    pub fn mount_of(&self, k: &str) -> Option<(String, String)> {
        let dev = self.dev.join(k).to_string_lossy().into_owned();
        self.mount_table().into_iter().find(|(s, _, _)| *s == dev).map(|(_, m, t)| (m, t))
    }

    fn under_media(&self, p: &str) -> bool {
        p.starts_with(&format!("{}/", self.media.to_string_lossy().trim_end_matches('/')))
    }
}

fn log(t: &Tree, msg: &str) {
    let _ = t.tool("logger").args(["-t", "nexa-drives", msg]).output();
}

fn blkid(t: &Tree, k: &str) -> (Option<String>, Option<String>) {
    let out = t.tool("blkid").arg(t.dev.join(k)).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let field = |name: &str| out.split(&format!(" {name}=\"")).nth(1).and_then(|r| r.split('"').next()).map(str::to_string);
    (field("LABEL"), field("TYPE"))
}

fn safe_name(label: Option<&str>, k: &str) -> String {
    let cleaned: String = label.unwrap_or("").chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).take(32).collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == '_').to_string();
    if cleaned.is_empty() { k.to_string() } else { cleaned }
}

/// The desktop account (uid 1000, whatever the installer named it).
fn options(fstype: &str) -> String {
    let base = "nosuid,nodev,noexec,noatime";
    match fstype {
        "vfat" => format!("{base},uid=1000,gid=1000,umask=022,flush"),
        "exfat" | "ntfs3" => format!("{base},uid=1000,gid=1000,umask=022"),
        _ => base.into(),
    }
}

pub fn mount(t: &Tree, k: &str) -> Result<Value, String> {
    if !t.eligible(k) {
        return Err("That isn't a removable drive.".into());
    }
    if !t.is_partition(k) && !t.partitions_of(k).is_empty() {
        return Err("This disk is partitioned - open one of its partitions instead.".into());
    }
    if let Some((mp, fstype)) = t.mount_of(k) {
        if t.under_media(&mp) {
            return Ok(json!({"mountpoint": mp, "fstype": fstype}));
        }
        return Err("This drive is already in use by the system.".into());
    }
    let (label, detected) = blkid(t, k);
    let base = t.media.join(safe_name(label.as_deref(), k));
    let taken: Vec<String> = t.mount_table().into_iter().map(|(_, m, _)| m).collect();
    let mp = [base.clone(), PathBuf::from(format!("{}-{k}", base.display()))].into_iter()
        .find(|c| !taken.contains(&c.to_string_lossy().into_owned())).ok_or("Couldn't pick a folder to mount this drive in.")?;
    fs::create_dir_all(&mp).map_err(|e| e.to_string())?;
    let first = match detected.as_deref() {
        Some("vfat") => Some("vfat"), Some("exfat") => Some("exfat"), Some("ntfs") => Some("ntfs3"),
        Some("ext4" | "ext3" | "ext2") => Some("ext4"), Some("udf") => Some("udf"), Some("iso9660") => Some("iso9660"),
        _ => None,
    };
    let order: Vec<&str> = first.into_iter().chain(FS_TYPES.iter().copied().filter(|f| Some(*f) != first)).collect();
    let dev = t.dev.join(k);
    for fstype in order {
        let ok = t.tool("mount").args(["-t", fstype, "-o", &options(fstype)]).arg(&dev).arg(&mp)
            .output().map(|o| o.status.success()).unwrap_or(false);
        if ok {
            log(t, &format!("mounted {} ({fstype}) at {}", dev.display(), mp.display()));
            return Ok(json!({"mountpoint": mp, "fstype": fstype, "done": format!("Opened {}", mp.display())}));
        }
    }
    let _ = fs::remove_dir(&mp);
    Err("Couldn't open this drive (unsupported or damaged filesystem).".into())
}

pub fn unmount(t: &Tree, k: &str) -> Result<Value, String> {
    let (mp, _) = t.mount_of(k).ok_or("That drive isn't mounted.")?;
    if !t.under_media(&mp) {
        return Err("That drive isn't one of the removable drives.".into());
    }
    let ok = t.tool("umount").arg(&mp).output().map(|o| o.status.success()).unwrap_or(false);
    if !ok {
        return Err("The drive is still in use - close anything using it and try again.".into());
    }
    let _ = fs::remove_dir(&mp);
    Ok(json!({"unmounted": mp, "done": format!("Closed {mp}")}))
}

pub fn eject(t: &Tree, k: &str) -> Result<Value, String> {
    if !t.eligible(k) {
        return Err("That isn't a removable drive.".into());
    }
    let disk = t.disk_of(k);
    for name in t.partitions_of(&disk).into_iter().chain([disk.clone()]) {
        if t.mount_of(&name).is_some() {
            unmount(t, &name)?;
        }
    }
    // SAFETY: sync(2) has no failure mode.
    unsafe { libc::sync() };
    let dir = t.real_dir(&disk);
    let delete = dir.join("device/delete");
    if dir.to_string_lossy().contains("/usb") && delete.exists() {
        fs::write(&delete, "1").map_err(|e| e.to_string())?;
    }
    Ok(json!({"ejected": disk, "done": "You can unplug the drive now"}))
}

pub fn list(t: &Tree) -> Value {
    let mut names: Vec<String> = fs::read_dir(t.sys.join("class/block")).into_iter().flatten().flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let vols: Vec<Value> = names.into_iter().filter(|k| t.eligible(k) && (t.is_partition(k) || t.partitions_of(k).is_empty())).map(|k| {
        let (label, fstype) = blkid(t, &k);
        let sectors: u64 = fs::read_to_string(t.real_dir(&k).join("size")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let mp = t.mount_of(&k).map(|(m, _)| m).filter(|m| t.under_media(m));
        json!({"name": k, "label": label.unwrap_or_default(), "size_bytes": sectors * 512, "mountpoint": mp, "fstype": fstype.unwrap_or_default()})
    }).collect();
    json!({"drives": vols})
}

/// udev: a drive appeared. Never fails loudly, never blocks udev for long.
pub fn added(t: &Tree, k: &str) -> Value {
    if !t.eligible(k) {
        return json!({"skipped": "not removable"});
    }
    if !t.is_partition(k) {
        // The kernel announces a disk before its partitions.
        std::thread::sleep(std::time::Duration::from_secs(1));
        if !t.partitions_of(k).is_empty() {
            return json!({"skipped": "has partitions"});
        }
    }
    mount(t, k).unwrap_or_else(|e| {
        log(t, &format!("auto-mount of {k} skipped: {e}"));
        json!({"skipped": e})
    })
}

/// udev: a drive was unplugged; drop a leftover mount and its folder.
pub fn removed(t: &Tree, k: &str) -> Value {
    if !valid_kname(k) {
        return json!({"skipped": "bad name"});
    }
    let dev = t.dev.join(k).to_string_lossy().into_owned();
    for (s, mp, _) in t.mount_table() {
        if s == dev && t.under_media(&mp) {
            let _ = t.tool("umount").args(["-l", &mp]).output();
            let _ = fs::remove_dir(Path::new(&mp));
        }
    }
    json!({"cleaned": k})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    /// A fake machine: sdb (USB stick, one partition sdb1 "MY DRIVE"),
    /// sdc (USB disk with removable=0, no partitions), sda (internal SATA),
    /// vda (virtio). Fake mount/umount/blkid keep /proc/mounts up to date
    /// and record every call.
    fn fake(name: &str) -> (Tree, PathBuf) {
        let r = std::env::temp_dir().join(format!("nexa-broker-drives-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let devices = r.join("sys/devices");
        let usb = devices.join("pci0000:00/0000:00:14.0/usb1/1-1/1-1:1.0/host6/target6:0:0/6:0:0:0/block/sdb");
        let usb2 = devices.join("pci0000:00/0000:00:14.0/usb1/1-2/1-2:1.0/host7/target7:0:0/7:0:0:0/block/sdc");
        let sata = devices.join("pci0000:00/0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0/block/sda");
        let virtio = devices.join("pci0000:00/0000:00:04.0/virtio1/block/vda");
        for (d, removable) in [(&usb, "1"), (&usb2, "0"), (&sata, "0"), (&virtio, "0")] {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join("removable"), format!("{removable}\n")).unwrap();
            fs::write(d.join("size"), "4096\n").unwrap();
        }
        for (disk, part) in [(&usb, "sdb1"), (&sata, "sda1")] {
            fs::create_dir_all(disk.join(part)).unwrap();
            fs::write(disk.join(part).join("partition"), "1").unwrap();
            fs::write(disk.join(part).join("size"), "2048\n").unwrap();
        }
        // USB devices can be deleted (powered off) through device/delete.
        for d in [&usb, &usb2] {
            fs::create_dir_all(d.join("device")).unwrap();
            fs::write(d.join("device/delete"), "").unwrap();
        }
        let cb = r.join("sys/class/block");
        fs::create_dir_all(&cb).unwrap();
        for (n, p) in [("sdb", usb.clone()), ("sdb1", usb.join("sdb1")), ("sdc", usb2.clone()), ("sda", sata.clone()),
                       ("sda1", sata.join("sda1")), ("vda", virtio.clone())] {
            symlink(p, cb.join(n)).unwrap();
        }
        fs::create_dir_all(r.join("proc")).unwrap();
        let mounts = r.join("proc/mounts");
        fs::write(&mounts, "/dev/sda1 / ext4 rw 0 0\n").unwrap();
        // Fakes: blkid answers from files, mount succeeds only for the fs
        // type in accept/<name> (or any type if the file says "any").
        let bin = r.join("bin");
        fs::create_dir_all(r.join("blkid")).unwrap();
        fs::create_dir_all(r.join("accept")).unwrap();
        fs::write(r.join("blkid/sdb1"), "/dev/sdb1: LABEL=\"MY DRIVE\" UUID=\"1234\" TYPE=\"vfat\"\n").unwrap();
        fs::write(r.join("accept/sdb1"), "vfat").unwrap();
        fs::create_dir_all(&bin).unwrap();
        let calls = r.join("calls");
        let scripts = [
            ("blkid", format!("#!/bin/sh\ncat {}/blkid/$(basename $1) 2>/dev/null\n", r.display())),
            ("mount", format!("#!/bin/sh\necho \"mount $*\" >> {c}\nn=$(basename $5)\nok=$(cat {r}/accept/$n 2>/dev/null)\n\
                               [ \"$ok\" = any ] || [ \"$ok\" = \"$2\" ] || exit 32\n\
                               echo \"$5 $(echo $6 | sed 's/ /\\\\040/g') $2 rw 0 0\" >> {m}\n",
                              c = calls.display(), r = r.display(), m = mounts.display())),
            ("umount", format!("#!/bin/sh\necho \"umount $*\" >> {c}\nmp=$(eval echo \\${{$#}})\n\
                                grep -v \" $(echo $mp | sed 's/ /\\\\\\\\040/g') \" {m} > {m}.n; mv {m}.n {m}\n",
                               c = calls.display(), m = mounts.display())),
            ("logger", "#!/bin/sh\nexit 0\n".to_string()),
        ];
        for (n, body) in scripts {
            fs::write(bin.join(n), body).unwrap();
            fs::set_permissions(bin.join(n), fs::Permissions::from_mode(0o755)).unwrap();
        }
        (Tree { sys: r.join("sys"), dev: "/dev".into(), media: r.join("media"), mounts, tools: bin }, r)
    }

    fn calls(r: &Path) -> String {
        fs::read_to_string(r.join("calls")).unwrap_or_default()
    }

    #[test]
    fn which_drives() {
        let (t, _) = fake("which");
        assert!(t.eligible("sdb1") && t.eligible("sdb"), "USB partition and disk");
        assert!(t.eligible("sdc"), "USB disk with removable=0 (it's under /usb)");
        assert!(!t.eligible("sda") && !t.eligible("sda1") && !t.eligible("vda"), "internal and virtio disks");
        for n in ["../sdb1", "sdb1; rm -rf /", "", "nvme0n1", "loop0"] {
            assert!(!t.eligible(n), "{n}");
        }
    }

    #[test]
    fn mounting() {
        let (t, r) = fake("mount");
        let m = mount(&t, "sdb1").unwrap();
        let mp = t.media.join("MY_DRIVE");
        assert_eq!((m["mountpoint"].as_str().unwrap(), m["fstype"].as_str().unwrap()), (mp.to_str().unwrap(), "vfat"), "detected fs first");
        assert!(calls(&r).contains("-o nosuid,nodev,noexec,noatime,uid=1000,gid=1000,umask=022,flush"), "{}", calls(&r));
        assert!(mp.is_dir());
        assert_eq!(mount(&t, "sdb1").unwrap()["mountpoint"], m["mountpoint"], "idempotent");
        assert!(calls(&r).matches("mount -t").count() == 1, "no second mount");
        assert!(mount(&t, "sdb").unwrap_err().contains("partition"));
        assert_eq!(mount(&t, "sda1").unwrap_err(), "That isn't a removable drive.");
        // A drive the system mounted elsewhere is left alone.
        fs::write(&t.mounts, format!("{}/dev/sdc /mnt/backup ext4 rw 0 0\n", fs::read_to_string(&t.mounts).unwrap())).unwrap();
        assert!(mount(&t, "sdc").unwrap_err().contains("in use by the system"));
        assert!(unmount(&t, "sdc").unwrap_err().contains("isn't one of the removable"));
    }

    #[test]
    fn filesystems_and_folder_names() {
        let (t, r) = fake("fs");
        // Unknown fs: tries the types in order until one works; no label: device name.
        fs::write(r.join("accept/sdc"), "ntfs3").unwrap();
        let m = mount(&t, "sdc").unwrap();
        assert!(calls(&r).contains("-t vfat") && calls(&r).contains("-t exfat") && calls(&r).contains("-t ntfs3"));
        assert_eq!(m["mountpoint"].as_str().unwrap(), t.media.join("sdc").to_str().unwrap());
        // Nothing mounts: clear error, no folder left behind.
        fs::write(r.join("blkid/sdb1"), "").unwrap();
        fs::write(r.join("accept/sdb1"), "none").unwrap();
        assert!(mount(&t, "sdb1").unwrap_err().contains("unsupported or damaged"));
        assert!(!t.media.join("sdb1").exists());
    }

    #[test]
    fn same_label_twice_gets_a_unique_folder() {
        let (t, r) = fake("twice");
        fs::write(r.join("blkid/sdc"), "/dev/sdc: LABEL=\"MY DRIVE\" TYPE=\"vfat\"\n").unwrap();
        fs::write(r.join("accept/sdc"), "vfat").unwrap();
        let a = mount(&t, "sdb1").unwrap();
        let b = mount(&t, "sdc").unwrap();
        assert_ne!(a["mountpoint"], b["mountpoint"]);
        assert!(b["mountpoint"].as_str().unwrap().ends_with("MY_DRIVE-sdc"));
    }

    #[test]
    fn listing_unmounting_ejecting() {
        let (t, r) = fake("eject");
        let names: Vec<String> = list(&t)["drives"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap().into()).collect();
        assert_eq!(names, vec!["sdb1", "sdc"], "removable only, never a disk that has partitions");
        let d = &list(&t)["drives"][0];
        assert_eq!((d["label"].as_str(), d["size_bytes"].as_u64()), (Some("MY DRIVE"), Some(2048 * 512)));
        assert!(unmount(&t, "sdb1").unwrap_err().contains("isn't mounted"));
        let m = mount(&t, "sdb1").unwrap();
        unmount(&t, "sdb1").unwrap();
        assert!(!Path::new(m["mountpoint"].as_str().unwrap()).exists(), "folder removed");
        mount(&t, "sdb1").unwrap();
        eject(&t, "sdb1").unwrap();
        assert!(t.mount_of("sdb1").is_none(), "eject unmounts every volume");
        let delete = fs::canonicalize(t.sys.join("class/block/sdb")).unwrap().join("device/delete");
        assert_eq!(fs::read_to_string(delete).unwrap(), "1", "and powers the USB device off");
        let _ = r;
    }

    #[test]
    fn udev_entry_points_never_fail() {
        let (t, r) = fake("udev");
        assert!(added(&t, "sdb1").get("mountpoint").is_some(), "a USB partition is mounted");
        assert_eq!(added(&t, "sdb"), json!({"skipped": "has partitions"}));
        assert_eq!(added(&t, "sda1"), json!({"skipped": "not removable"}));
        fs::write(r.join("accept/sdc"), "none").unwrap();
        assert!(added(&t, "sdc").get("skipped").is_some(), "unreadable filesystem: skipped, no error");
        removed(&t, "sdb1");
        assert!(calls(&r).contains("umount -l"), "a yanked drive's leftover mount is cleaned up");
        assert_eq!(removed(&t, "../../etc"), json!({"skipped": "bad name"}));
    }

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(safe_name(Some("My Photos/../x"), "sdb1"), "My_Photos_.._x");
        assert_eq!(safe_name(Some(""), "sdb1"), "sdb1");
        assert_eq!(safe_name(Some("..."), "sdb1"), "sdb1");
    }
}
