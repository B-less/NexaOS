//! The Trash, as the freedesktop.org Trash spec describes it, so other
//! apps' trashes and ours are the same: files/<name> plus
//! info/<name>.trashinfo (original path, percent-encoded, and deletion date).
//! Things in your home folder's filesystem go to ~/.local/share/Trash;
//! things on another drive (a USB stick) go to that drive's own
//! .Trash-<uid> folder, so deleting never copies gigabytes across drives.

use crate::ops::{self, place, rename_noreplace, Step};
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

/// A trash folder, and the top of the drive it belongs to (None: the home
/// trash, whose .trashinfo paths are absolute).
#[derive(Debug, PartialEq)]
pub struct TrashDir {
    pub dir: PathBuf,
    pub topdir: Option<PathBuf>,
}

pub fn home_trash(home: &Path) -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME").filter(|v| Path::new(v).is_absolute()) {
        Some(d) => PathBuf::from(d).join("Trash"),
        None => home.join(".local/share/Trash"),
    }
}

fn uid() -> u32 {
    // SAFETY: getuid never fails.
    unsafe { libc::getuid() }
}

/// The top folder of the filesystem `p` is on.
fn mount_root(p: &Path) -> PathBuf {
    let dev = ops::device(p);
    let mut cur = ops::existing_ancestor(p);
    while let Some(parent) = cur.parent() {
        if fs::symlink_metadata(parent).map(|m| m.dev()).ok() != dev {
            break;
        }
        cur = parent.to_path_buf();
    }
    cur
}

/// Where `p` goes when it is trashed.
pub fn trash_for(p: &Path, home: &Path) -> TrashDir {
    let home_trash = home_trash(home);
    let parent = p.parent().unwrap_or(Path::new("/"));
    if ops::device(parent) == ops::device(&home_trash) {
        return TrashDir { dir: home_trash, topdir: None };
    }
    let top = mount_root(parent);
    TrashDir { dir: top.join(format!(".Trash-{}", uid())), topdir: Some(top) }
}

/// Create files/ and info/ (the drive's .Trash-<uid> only you can open).
pub fn prepare(t: &TrashDir) -> io::Result<()> {
    if t.topdir.is_some() && !t.dir.exists() {
        fs::DirBuilder::new().mode(0o700).create(&t.dir)?;
    }
    fs::DirBuilder::new().recursive(true).mode(0o700).create(t.dir.join("files"))?;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(t.dir.join("info"))
}

/// Percent-encode a path for a .trashinfo file (RFC 2396 style, as the spec asks).
pub fn encode(p: &Path) -> String {
    let mut s = String::new();
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/!*'()".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

pub fn decode(s: &str) -> PathBuf {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    PathBuf::from(OsStr::from_bytes(&out))
}

/// Local time as the spec's DeletionDate (YYYY-MM-DDThh:mm:ss).
fn now_local() -> String {
    // SAFETY: time with a null pointer only returns; tm is plain data that
    // localtime_r fills in.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        libc::localtime_r(&t, &mut tm);
    }
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

pub fn info_text(original: &Path, t: &TrashDir, date: &str) -> String {
    let shown = match &t.topdir {
        Some(top) => original.strip_prefix(top).unwrap_or(original).to_path_buf(),
        None => original.to_path_buf(),
    };
    format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", encode(&shown))
}

/// Move `p` to the Trash. The info file is claimed first (O_EXCL), as the
/// spec requires, then the item is renamed in beside it.
pub fn trash(p: &Path, t: &TrashDir) -> io::Result<Step> {
    prepare(t)?;
    let name = p.file_name().ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let is_dir = fs::symlink_metadata(p)?.is_dir();
    let text = info_text(p, t, &now_local());
    let mut claimed = None;
    let trashed = place(&t.dir.join("files"), name, false, is_dir, |target| {
        let mut info_name = target.file_name().unwrap().to_os_string();
        info_name.push(".trashinfo");
        let info = t.dir.join("info").join(info_name);
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&info)?;
        f.write_all(text.as_bytes())?;
        drop(f);
        if let Err(e) = rename_noreplace(p, target) {
            let _ = fs::remove_file(&info);
            return Err(e);
        }
        claimed = Some(info);
        Ok(())
    })?;
    Ok(Step::Trashed { path: p.to_path_buf(), trashed, info: claimed.unwrap() })
}

/// The .trashinfo beside an item in some trash's files/ folder.
pub fn info_of(trashed: &Path) -> Option<PathBuf> {
    let files = trashed.parent()?;
    if files.file_name()? != "files" {
        return None;
    }
    let mut n = trashed.file_name()?.to_os_string();
    n.push(".trashinfo");
    Some(files.parent()?.join("info").join(n))
}

/// Where a trashed item came from, from its .trashinfo.
pub fn original_of(trashed: &Path) -> Option<(PathBuf, String)> {
    let info = info_of(trashed)?;
    let text = fs::read_to_string(info).ok()?;
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).map(str::trim).map(str::to_string);
    let path = decode(&field("Path=")?);
    let date = field("DeletionDate=").unwrap_or_default();
    let tdir = trashed.parent()?.parent()?;
    let full = if path.is_absolute() { path } else { tdir.parent()?.join(path) };
    Some((full, date))
}

/// Put a trashed item back where it was (as "name (2)" if that's taken
/// now), recreating its folder if needed.
pub fn restore(trashed: &Path) -> io::Result<PathBuf> {
    let (original, _) = original_of(trashed).ok_or_else(|| io::Error::other("its Trash record is missing"))?;
    let parent = original.parent().ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    fs::create_dir_all(parent)?;
    let is_dir = fs::symlink_metadata(trashed)?.is_dir();
    let name = original.file_name().ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let back = place(parent, name, false, is_dir, |target| rename_noreplace(trashed, target))?;
    if let Some(info) = info_of(trashed) {
        let _ = fs::remove_file(info);
    }
    Ok(back)
}

/// After deleting something from a trash for good, drop its record too.
pub fn forget(trashed: &Path) {
    if let Some(info) = info_of(trashed) {
        let _ = fs::remove_file(info);
    }
}

/// Every trash we can see: home's, and each mounted drive's .Trash-<uid>.
pub fn all(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![home_trash(home)];
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    for line in mounts.lines() {
        if let Some(mp) = line.split_whitespace().nth(1) {
            let d = Path::new(&mp.replace("\\040", " ")).join(format!(".Trash-{}", uid()));
            if d.join("info").is_dir() && !dirs.contains(&d) {
                dirs.push(d);
            }
        }
    }
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// What's in the Trash, newest first.
pub fn list(dirs: &[PathBuf]) -> Vec<Value> {
    let mut items: Vec<(String, Value)> = Vec::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(d.join("files")) else { continue };
        for e in rd.flatten() {
            let trashed = e.path();
            let Ok(md) = fs::symlink_metadata(&trashed) else { continue };
            let (original, date) = original_of(&trashed).unwrap_or_else(|| (trashed.clone(), String::new()));
            let size = if md.is_dir() { ops::usage(&[trashed.clone()]).bytes } else { md.len() };
            items.push((date.clone(), json!({
                "name": original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                "trashed": trashed, "original": original, "deleted": date, "size": size, "is_dir": md.is_dir(),
            })));
        }
    }
    items.sort_by(|a, b| b.0.cmp(&a.0));
    items.into_iter().map(|(_, v)| v).collect()
}

/// Delete everything in these trashes for good. Returns how many items.
pub fn empty(dirs: &[PathBuf]) -> (u64, Vec<(PathBuf, String)>) {
    let (mut n, mut failed) = (0, Vec::new());
    for d in dirs {
        for sub in ["files", "info"] {
            for e in fs::read_dir(d.join(sub)).into_iter().flatten().flatten() {
                match ops::remove(&e.path()) {
                    Ok(()) => n += (sub == "files") as u64,
                    Err(err) => failed.push((e.path(), ops::say(&err))),
                }
            }
        }
        let _ = fs::remove_file(d.join("directorysizes"));
    }
    (n, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_round_trips() {
        let p = Path::new("/home/user/My Files/naïve 100%.txt");
        let e = encode(p);
        assert_eq!(e, "/home/user/My%20Files/na%C3%AFve%20100%25.txt");
        assert_eq!(decode(&e), p);
        assert_eq!(decode("/broken%2"), Path::new("/broken%2"), "a stray % stays");
    }

    #[test]
    fn info_paths_are_relative_on_drives() {
        let drive = TrashDir { dir: "/media/STICK/.Trash-1000".into(), topdir: Some("/media/STICK".into()) };
        assert_eq!(info_text(Path::new("/media/STICK/a b.txt"), &drive, "2026-10-17T12:00:00"),
                   "[Trash Info]\nPath=a%20b.txt\nDeletionDate=2026-10-17T12:00:00\n");
        let home = TrashDir { dir: "/home/u/.local/share/Trash".into(), topdir: None };
        assert!(info_text(Path::new("/home/u/x"), &home, "d").contains("Path=/home/u/x\n"));
    }

    #[test]
    fn trash_and_restore() {
        let d = std::env::temp_dir().join(format!("nexa-fileops-{}-trash", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("docs")).unwrap();
        let t = TrashDir { dir: d.join("Trash"), topdir: None };
        fs::write(d.join("docs/note.txt"), "1").unwrap();
        let first = trash(&d.join("docs/note.txt"), &t).unwrap();
        fs::write(d.join("docs/note.txt"), "2").unwrap();
        let second = trash(&d.join("docs/note.txt"), &t).unwrap();
        let Step::Trashed { trashed: t1, info: i1, .. } = first else { panic!() };
        let Step::Trashed { trashed: t2, .. } = second else { panic!() };
        assert_eq!((t1.file_name().unwrap(), t2.file_name().unwrap()), (OsStr::new("note.txt"), OsStr::new("note (2).txt")));
        assert!(fs::read_to_string(&i1).unwrap().starts_with(&format!("[Trash Info]\nPath={}/docs/note.txt\nDeletionDate=20", encode(&d))));
        assert_eq!(list(&[t.dir.clone()]).len(), 2);
        // Restore the first: its folder is gone, so it's recreated.
        fs::remove_dir(d.join("docs")).unwrap();
        assert_eq!(restore(&t1).unwrap(), d.join("docs/note.txt"));
        assert_eq!(fs::read_to_string(d.join("docs/note.txt")).unwrap(), "1");
        assert!(!i1.exists(), "record removed");
        // The second comes back beside it, not over it.
        assert_eq!(restore(&t2).unwrap(), d.join("docs/note (2).txt"));
        assert_eq!(empty(&[t.dir.clone()]).0, 0);
        fs::write(d.join("x"), "").unwrap();
        trash(&d.join("x"), &t).unwrap();
        assert_eq!(empty(&[t.dir.clone()]).0, 1);
        assert!(list(&[t.dir]).is_empty());
    }
}
