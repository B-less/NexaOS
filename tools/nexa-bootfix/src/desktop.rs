//! The person at the desktop: their Wayland display, and starting a desktop
//! app (Files) as them, not as root. Used to let them pick, in a file
//! explorer, which files to copy off a lost Windows partition.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Wayland sockets that exist: (runtime dir, display name), the ones of the
/// user who ran `doas` first.
pub fn wayland_sockets() -> Vec<(PathBuf, String)> {
    let mut tries: Vec<(PathBuf, String)> = Vec::new();
    if let (Ok(r), Ok(d)) = (std::env::var("XDG_RUNTIME_DIR"), std::env::var("WAYLAND_DISPLAY")) {
        tries.push((PathBuf::from(r), d));
    }
    let mut dirs = Vec::new();
    if let Some(u) = std::env::var("DOAS_USER").or_else(|_| std::env::var("SUDO_USER")).ok().and_then(|n| user_by_name(&n)) {
        dirs.push(PathBuf::from(format!("/run/user/{}", u.uid)));
    }
    let mut others: Vec<PathBuf> = fs::read_dir("/run/user").into_iter().flatten().flatten().map(|e| e.path()).collect();
    others.sort();
    dirs.extend(others);
    for dir in dirs {
        let mut socks: Vec<String> = fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("wayland-") && !n.ends_with(".lock")).collect();
        socks.sort();
        tries.extend(socks.into_iter().map(|s| (dir.clone(), s)));
    }
    tries.retain(|(d, s)| d.join(s).exists());
    tries
}

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Desktop {
    pub user: User,
    pub runtime: PathBuf,
    pub display: String,
}

fn from_passwd(pw: *const libc::passwd) -> Option<User> {
    if pw.is_null() {
        return None;
    }
    // SAFETY: getpw* returned a valid struct; the strings are copied at once.
    unsafe {
        Some(User {
            uid: (*pw).pw_uid,
            gid: (*pw).pw_gid,
            name: CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned(),
            home: PathBuf::from(CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned()),
        })
    }
}

pub fn user_by_name(name: &str) -> Option<User> {
    let c = CString::new(name).ok()?;
    // SAFETY: getpwnam with a NUL-terminated name.
    from_passwd(unsafe { libc::getpwnam(c.as_ptr()) })
}

fn user_by_uid(uid: u32) -> Option<User> {
    // SAFETY: getpwuid.
    from_passwd(unsafe { libc::getpwuid(uid) })
}

/// The (non-root) person whose desktop is showing.
pub fn current() -> Option<Desktop> {
    wayland_sockets().into_iter().find_map(|(runtime, display)| {
        let uid: u32 = runtime.file_name()?.to_str()?.parse().ok()?;
        (uid != 0).then_some(())?;
        Some(Desktop { user: user_by_uid(uid)?, runtime, display })
    })
}

/// Mount options that make a FAT/NTFS/exFAT drive look owned by `u`.
pub fn owner_options(fs_type: &str, u: &User) -> String {
    match fs_type {
        "vfat" => format!("uid={},gid={},umask=022,utf8,shortname=mixed", u.uid, u.gid),
        _ => format!("uid={},gid={},umask=022", u.uid, u.gid),
    }
}

/// Start `program args` as the desktop's user, on their display.
pub fn launch(d: &Desktop, program: &str, args: &[&Path]) -> std::io::Result<Child> {
    let name = CString::new(d.user.name.clone()).map_err(std::io::Error::other)?;
    let (uid, gid) = (d.user.uid, d.user.gid);
    let mut c = Command::new(program);
    c.args(args).env_clear()
        .env("HOME", &d.user.home).env("USER", &d.user.name).env("LOGNAME", &d.user.name)
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .env("XDG_RUNTIME_DIR", &d.runtime).env("WAYLAND_DISPLAY", &d.display)
        .env("LANG", std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()))
        .current_dir(&d.user.home)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let bus = d.runtime.join("bus");
    if bus.exists() {
        c.env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", bus.display()));
    }
    // SAFETY: only async-signal-safe calls between fork and exec. Groups
    // first, then the group, then the user (after which root is gone).
    unsafe {
        c.pre_exec(move || {
            if libc::initgroups(name.as_ptr(), gid as _) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    c.spawn()
}

/// Every file under `root` (relative path -> size), leaving out `skip`
/// and Windows' bookkeeping folders. Used to tell what was copied.
pub fn file_set(root: &Path, skip: &Path) -> BTreeMap<PathBuf, u64> {
    fn walk(dir: &Path, root: &Path, skip: &Path, out: &mut BTreeMap<PathBuf, u64>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            let lower = e.file_name().to_string_lossy().to_lowercase();
            if p == skip || lower == "system volume information" || lower == "$recycle.bin" {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(&p, root, skip, out),
                Ok(t) if t.is_file() => {
                    out.insert(p.strip_prefix(root).unwrap_or(&p).to_path_buf(), e.metadata().map_or(0, |m| m.len()));
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, skip, &mut out);
    out
}

/// Files new or changed in `after` compared with `before`.
pub fn added(before: &BTreeMap<PathBuf, u64>, after: &BTreeMap<PathBuf, u64>) -> Vec<(PathBuf, u64)> {
    after.iter().filter(|(p, n)| before.get(*p) != Some(n)).map(|(p, n)| (p.clone(), *n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_copied() {
        let root = std::env::temp_dir().join(format!("nexa-desktop-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("report")).unwrap();
        fs::create_dir_all(root.join("old")).unwrap();
        fs::write(root.join("old/a.txt"), b"a").unwrap();
        fs::write(root.join("report/r.txt"), b"r").unwrap();
        let before = file_set(&root, &root.join("report"));
        assert_eq!(before.len(), 1);
        fs::create_dir_all(root.join("Pictures")).unwrap();
        fs::write(root.join("Pictures/p.jpg"), b"jpeg").unwrap();
        fs::write(root.join("report/r2.txt"), b"r").unwrap();
        let after = file_set(&root, &root.join("report"));
        assert_eq!(added(&before, &after), vec![(PathBuf::from("Pictures/p.jpg"), 4)]);
        let u = User { uid: 1000, gid: 1000, name: "user".into(), home: "/home/user".into() };
        assert_eq!(owner_options("ntfs", &u), "uid=1000,gid=1000,umask=022");
        assert!(owner_options("vfat", &u).starts_with("uid=1000,gid=1000,umask=022,utf8"));
        fs::remove_dir_all(&root).unwrap();
    }
}
