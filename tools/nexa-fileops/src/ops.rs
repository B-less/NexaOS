//! Copy, move, rename, create, delete and measure - the low-level part.
//!
//! Nothing here ever overwrites: new names are claimed exclusively
//! (O_EXCL, mkdir, symlink, renameat2(RENAME_NOREPLACE)) and a taken name
//! becomes "name (copy).ext" or "name (2).ext". Symbolic links are copied as
//! links and never followed when deleting. File data moves with
//! copy_file_range(2) (in-kernel, reflinks where the filesystem can),
//! falling back to read/write across filesystems that can't.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::ffi::{CString, OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// One change, recorded so it can be undone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Step {
    /// Something new appeared at `path` (undo: it goes to the Trash).
    Created { path: PathBuf },
    /// Moved or renamed (undo: move back).
    Moved { from: PathBuf, to: PathBuf },
    /// Moved to the Trash (undo: restore it).
    Trashed { path: PathBuf, trashed: PathBuf, info: PathBuf },
}

/// Progress lines on stdout ({"progress": {...}}), at most ~7 a second.
pub struct Progress {
    pub total: u64,
    pub done: u64,
    pub quiet: bool,
    last: Option<Instant>,
}

impl Progress {
    pub fn new(total: u64) -> Progress {
        Progress { total, done: 0, quiet: false, last: None }
    }

    pub fn add(&mut self, n: u64, current: &Path) {
        self.done += n;
        if self.quiet {
            return;
        }
        let now = Instant::now();
        if self.last.map_or(true, |t| now.duration_since(t).as_millis() >= 150) {
            self.last = Some(now);
            let name = current.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mut out = io::stdout().lock();
            let _ = writeln!(out, "{}", json!({"progress": {"done": self.done, "total": self.total, "current": name}}));
            let _ = out.flush();
        }
    }
}

impl Progress {
    /// Everything is copied; now waiting for it to reach the drive.
    pub fn finishing(&mut self) {
        if !self.quiet {
            let mut out = io::stdout().lock();
            let _ = writeln!(out, "{}", json!({"progress": {"done": self.done, "total": self.total, "current": "", "finishing": true}}));
            let _ = out.flush();
        }
    }
}

/// Write everything waiting for the filesystem holding `p` to the drive
/// (syncfs). Until this returns, a copy may exist only in memory: pulling a
/// USB stick then loses it - and with a move, the original is already gone.
/// A drive that failed to store it reports the error here.
pub fn flush_fs(p: &Path) -> io::Result<()> {
    let f = fs::File::open(p)?;
    // SAFETY: f is an open file descriptor.
    if unsafe { libc::syncfs(f.as_raw_fd()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// An error in words a person understands.
pub fn say(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(libc::ENOSPC) => "the drive is full".into(),
        Some(libc::EROFS) => "the drive is read-only".into(),
        Some(libc::EACCES) | Some(libc::EPERM) => "you don't have permission".into(),
        Some(libc::ENOENT) => "it isn't there any more".into(),
        Some(libc::EEXIST) => "something with that name is already there".into(),
        Some(libc::ENOTEMPTY) => "the folder isn't empty".into(),
        Some(libc::ENAMETOOLONG) => "the name is too long".into(),
        Some(libc::EBUSY) => "it's in use".into(),
        Some(libc::EXDEV) => "it's on another drive".into(),
        // Most often a name a USB drive (FAT32/exFAT) can't store.
        Some(libc::EINVAL) => "that isn't possible here (on USB drives, names can't contain : * ? \" < > | or \\)".into(),
        _ => e.to_string(),
    }
}

fn cstr(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

const RENAME_NOREPLACE: libc::c_uint = 1;

/// rename(2) that fails with AlreadyExists instead of replacing.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let (a, b) = (cstr(from)?, cstr(to)?);
    // SAFETY: both are valid C strings; renameat2 only reads them.
    let rc = unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), RENAME_NOREPLACE) };
    if rc == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    if matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOSYS)) && fs::symlink_metadata(from).is_ok() {
        // A filesystem without RENAME_NOREPLACE: check, then rename.
        if fs::symlink_metadata(to).is_ok() {
            return Err(io::Error::from_raw_os_error(libc::EEXIST));
        }
        return fs::rename(from, to);
    }
    Err(e)
}

/// "report.tar.gz" -> ("report", ".tar.gz"); folders and ".bashrc" have none.
fn split_ext(name: &[u8], is_dir: bool) -> (&[u8], &[u8]) {
    if is_dir {
        return (name, b"");
    }
    let lower = name.to_ascii_lowercase();
    for multi in [&b".tar.gz"[..], b".tar.xz", b".tar.bz2", b".tar.zst"] {
        if lower.len() > multi.len() && lower.ends_with(multi) {
            return name.split_at(name.len() - multi.len());
        }
    }
    match name.iter().rposition(|&c| c == b'.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, b""),
    }
}

/// The n-th name to try: n=0 is the name itself, then "x (copy).txt",
/// "x (copy 2).txt"... for copies, "x (2).txt", "x (3).txt"... otherwise.
pub fn candidate(name: &OsStr, n: u32, copy: bool, is_dir: bool) -> OsString {
    if n == 0 {
        return name.to_os_string();
    }
    let (stem, ext) = split_ext(name.as_bytes(), is_dir);
    let tag = match (copy, n) {
        (true, 1) => " (copy)".to_string(),
        (true, n) => format!(" (copy {n})"),
        (false, n) => format!(" ({})", n + 1),
    };
    let mut v = stem.to_vec();
    v.extend_from_slice(tag.as_bytes());
    v.extend_from_slice(ext);
    OsString::from_vec(v)
}

/// Claim a free name in `dir` for `name`: `make` must create the target
/// exclusively and return AlreadyExists if the name is taken.
pub fn place(dir: &Path, name: &OsStr, copy: bool, is_dir: bool, mut make: impl FnMut(&Path) -> io::Result<()>) -> io::Result<PathBuf> {
    for n in 0..10_000 {
        let target = dir.join(candidate(name, n, copy, is_dir));
        match make(&target) {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::from_raw_os_error(libc::EEXIST))
}

/// Errors from inside a copy must not look like "name taken" to `place`.
fn inner(e: io::Error) -> io::Error {
    if e.kind() == io::ErrorKind::AlreadyExists { io::Error::other(e.to_string()) } else { e }
}

/// A name a person typed: one path component.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("The name can't be empty.".into());
    }
    if name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err("Names can't contain \"/\" or be \".\" or \"..\".".into());
    }
    if name.len() > 255 {
        return Err("That name is too long.".into());
    }
    Ok(())
}

fn set_times(p: &Path, md: &fs::Metadata) {
    let Ok(c) = cstr(p) else { return };
    // SAFETY: timespec is plain data; zeroed is a valid value.
    let mut ts: [libc::timespec; 2] = unsafe { std::mem::zeroed() };
    ts[0].tv_sec = md.atime() as _;
    ts[0].tv_nsec = md.atime_nsec() as _;
    ts[1].tv_sec = md.mtime() as _;
    ts[1].tv_nsec = md.mtime_nsec() as _;
    // SAFETY: c is a valid path and ts two valid timespecs.
    unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), ts.as_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
}

/// Move the bytes: in the kernel with copy_file_range, else read/write.
fn copy_data(input: &fs::File, output: &fs::File, len: u64, prog: &mut Progress, name: &Path) -> io::Result<()> {
    let mut copied = 0u64;
    loop {
        // SAFETY: two open fds; null offsets use and advance the file positions.
        let n = unsafe {
            libc::syscall(libc::SYS_copy_file_range, input.as_raw_fd(), std::ptr::null_mut::<i64>(),
                          output.as_raw_fd(), std::ptr::null_mut::<i64>(), 1usize << 24, 0u32)
        };
        if n > 0 {
            copied += n as u64;
            prog.add(n as u64, name);
            continue;
        }
        if n == 0 && copied >= len {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if n < 0 && !matches!(e.raw_os_error(), Some(libc::EXDEV | libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP | libc::EPERM)) {
            return Err(e);
        }
        break; // Not possible here (or a file whose size lies, like /proc): plain copy.
    }
    let mut buf = vec![0u8; 1 << 20];
    let (mut r, mut w) = (input, output);
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        w.write_all(&buf[..n])?;
        prog.add(n as u64, name);
    }
}

/// Things inside a folder that couldn't be copied (the rest still is).
pub type Failures = Vec<(PathBuf, String)>;

/// Copy `src` to the new name `dst`. Creating `dst` is the exclusive step;
/// AlreadyExists comes only from that, so `place` can pick another name.
pub fn copy_item(src: &Path, dst: &Path, prog: &mut Progress, failed: &mut Failures) -> io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    let ft = md.file_type();
    if ft.is_symlink() {
        let target = fs::read_link(src)?;
        return std::os::unix::fs::symlink(target, dst);
    }
    if ft.is_dir() {
        let entries = fs::read_dir(src)?;
        fs::create_dir(dst)?;
        let result = (|| -> io::Result<()> {
            for e in entries {
                let e = e?;
                let (s, d) = (e.path(), dst.join(e.file_name()));
                if let Err(err) = copy_item(&s, &d, prog, failed) {
                    failed.push((s, say(&err)));
                }
            }
            Ok(())
        })();
        // Permissions last, so a read-only folder could still be filled.
        let _ = fs::set_permissions(dst, fs::Permissions::from_mode(md.mode() & 0o1777));
        set_times(dst, &md);
        return result.map_err(inner);
    }
    if ft.is_file() {
        let input = fs::File::open(src)?;
        let output = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(dst)?;
        let result = copy_data(&input, &output, md.len(), prog, src).and_then(|_| {
            // No setuid/setgid on copies; filesystems without modes (FAT) refuse: fine.
            let _ = output.set_permissions(fs::Permissions::from_mode(md.mode() & 0o1777));
            Ok(())
        });
        drop(output);
        if let Err(e) = result {
            let _ = fs::remove_file(dst); // never leave half a file behind
            return Err(inner(e));
        }
        set_times(dst, &md);
        return Ok(());
    }
    Err(io::Error::other("devices, pipes and sockets can't be copied"))
}

/// Delete for good, never following symbolic links.
pub fn remove(p: &Path) -> io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

/// Move `src` to the new name `dst`: a rename on the same filesystem, else
/// copy everything and only then delete the original.
pub fn move_item(src: &Path, dst: &Path, prog: &mut Progress) -> io::Result<()> {
    match rename_noreplace(src, dst) {
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            let mut failed = Failures::new();
            copy_item(src, dst, prog, &mut failed)?;
            if let Some((p, why)) = failed.first() {
                let _ = remove(dst);
                return Err(io::Error::other(format!("couldn't copy {} ({why}), so nothing was moved", p.display())));
            }
            // The original goes only once the copy is really on the drive.
            prog.finishing();
            if let Err(e) = flush_fs(dst) {
                let _ = remove(dst);
                return Err(io::Error::other(format!("the drive couldn't store it ({}), so nothing was moved", say(&e))));
            }
            remove(src).map_err(inner)
        }
        other => other,
    }
}

/// True if `inside` is `dir` or somewhere below it.
pub fn is_within(inside: &Path, dir: &Path) -> bool {
    inside == dir || inside.starts_with(dir)
}

#[derive(Debug, Default, PartialEq)]
pub struct Usage {
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
}

/// What a set of paths holds, without following links and counting
/// hard-linked files once (like du's apparent size).
pub fn usage(paths: &[PathBuf]) -> Usage {
    let mut u = Usage::default();
    let mut seen = HashSet::new();
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(p) = stack.pop() {
        let Ok(md) = fs::symlink_metadata(&p) else { continue };
        if md.is_dir() {
            u.folders += 1;
            if let Ok(rd) = fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
            continue;
        }
        if md.nlink() > 1 && !seen.insert((md.dev(), md.ino())) {
            continue;
        }
        u.files += 1;
        u.bytes += md.len();
    }
    u
}

/// The device a path's filesystem is on (for its nearest existing ancestor).
pub fn device(p: &Path) -> Option<u64> {
    p.ancestors().find_map(|a| fs::symlink_metadata(a).ok()).map(|m| m.dev())
}

/// The closest folder that exists at or above `p`.
pub fn existing_ancestor(p: &Path) -> PathBuf {
    p.ancestors().find(|a| a.exists()).unwrap_or(Path::new("/")).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nexa-fileops-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn quiet() -> Progress {
        Progress { quiet: true, ..Progress::new(0) }
    }

    #[test]
    fn names() {
        let c = |n, i, copy, dir| candidate(OsStr::new(n), i, copy, dir).into_string().unwrap();
        assert_eq!(c("photo.jpg", 0, true, false), "photo.jpg");
        assert_eq!(c("photo.jpg", 1, true, false), "photo (copy).jpg");
        assert_eq!(c("photo.jpg", 2, true, false), "photo (copy 2).jpg");
        assert_eq!(c("photo.jpg", 1, false, false), "photo (2).jpg");
        assert_eq!(c("backup.tar.gz", 1, true, false), "backup (copy).tar.gz");
        assert_eq!(c(".bashrc", 1, true, false), ".bashrc (copy)");
        assert_eq!(c("v1.2 folder", 1, false, true), "v1.2 folder (2)");
        assert!(check_name("a/b").is_err() && check_name("..").is_err() && check_name(" ").is_err());
        assert!(check_name("Holiday photos").is_ok());
    }

    #[test]
    fn moving_to_another_drive_flushes_before_deleting() {
        // /dev/shm (tmpfs) and the temp folder are different filesystems here.
        let shm = Path::new("/dev/shm");
        let dir = std::env::temp_dir().join(format!("fileops-across-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        if !shm.is_dir() || device(shm) == device(&dir) {
            return;
        }
        let src = shm.join(format!("fileops-across-{}", std::process::id()));
        fs::write(&src, b"holiday photos").unwrap();
        assert!(flush_fs(&dir).is_ok());
        assert!(flush_fs(&dir.join("missing")).is_err());
        move_item(&src, &dir.join("photos"), &mut quiet()).unwrap();
        assert!(!src.exists(), "the original goes once the copy is stored");
        assert_eq!(fs::read(dir.join("photos")).unwrap(), b"holiday photos");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copying_never_overwrites_and_keeps_metadata() {
        let d = tmp("copy");
        fs::write(d.join("a.txt"), "hello").unwrap();
        fs::set_permissions(d.join("a.txt"), fs::Permissions::from_mode(0o640)).unwrap();
        let mut f = Failures::new();
        let src = d.join("a.txt");
        let got = place(&d, OsStr::new("a.txt"), true, false, |t| copy_item(&src, t, &mut quiet(), &mut f)).unwrap();
        assert_eq!(got, d.join("a (copy).txt"));
        assert_eq!(fs::read_to_string(&got).unwrap(), "hello");
        let (a, b) = (fs::metadata(&src).unwrap(), fs::metadata(&got).unwrap());
        assert_eq!((b.mode() & 0o777, b.mtime()), (0o640, a.mtime()), "mode and time kept");
        assert_eq!(fs::read_to_string(&src).unwrap(), "hello", "original untouched");
    }

    #[test]
    fn folders_links_and_progress() {
        let d = tmp("tree");
        let src = d.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/big.bin"), vec![7u8; 3 << 20]).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", src.join("link")).unwrap();
        let mut p = quiet();
        let mut f = Failures::new();
        copy_item(&src, &d.join("dst"), &mut p, &mut f).unwrap();
        assert!(f.is_empty(), "{f:?}");
        assert_eq!(fs::read(d.join("dst/sub/big.bin")).unwrap().len(), 3 << 20);
        assert_eq!(fs::read_link(d.join("dst/link")).unwrap(), Path::new("/etc/passwd"), "links copied as links");
        assert_eq!(p.done, 3 << 20);
        assert_eq!(usage(&[src.clone()]), Usage { bytes: (3 << 20) + 11, files: 2, folders: 2 });
        // Hard links count once.
        fs::hard_link(src.join("sub/big.bin"), src.join("again.bin")).unwrap();
        assert_eq!(usage(&[src]).bytes, (3 << 20) + 11);
    }

    #[test]
    fn rename_and_move_never_replace() {
        let d = tmp("move");
        fs::write(d.join("a"), "A").unwrap();
        fs::write(d.join("b"), "B").unwrap();
        assert_eq!(rename_noreplace(&d.join("a"), &d.join("b")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(d.join("b")).unwrap(), "B");
        fs::create_dir(d.join("into")).unwrap();
        fs::write(d.join("into/a"), "old").unwrap();
        let src = d.join("a");
        let got = place(&d.join("into"), OsStr::new("a"), false, false, |t| move_item(&src, t, &mut quiet())).unwrap();
        assert_eq!(got, d.join("into/a (2)"));
        assert_eq!((fs::read_to_string(&got).unwrap(), fs::read_to_string(d.join("into/a")).unwrap()), ("A".into(), "old".into()));
    }

    #[test]
    fn deleting_never_follows_links() {
        let d = tmp("rm");
        fs::create_dir_all(d.join("keep")).unwrap();
        fs::write(d.join("keep/precious"), "x").unwrap();
        fs::create_dir(d.join("gone")).unwrap();
        std::os::unix::fs::symlink(d.join("keep"), d.join("gone/link")).unwrap();
        remove(&d.join("gone")).unwrap();
        assert!(d.join("keep/precious").exists() && !d.join("gone").exists());
    }
}
