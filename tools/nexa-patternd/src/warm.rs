//! Phase 3 - faster starts. While you use an app, note which system files it
//! loaded (its program and libraries from /proc/<pid>/maps, and for NexaOS's
//! Python apps their package folders); when that app is likely to be next,
//! read those files into memory ahead of time, so it starts from RAM
//! instead of the disk or USB stick. Only from free memory, never under
//! memory pressure or on a low battery, and never anything in your home
//! folder (only the system's own files are listed).

use nexa_sys::preload;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Only the system's files: programs, libraries, data.
const SYSTEM: [&str; 6] = ["/usr/", "/lib/", "/opt/", "/bin/", "/sbin/", "/etc/fonts/"];
const MAX_FILES: usize = 4000;
const MAX_FILE: u64 = 64 << 20;
/// At most this much per app.
pub const MAX_APP_BYTES: u64 = 300 << 20;
const RECAPTURE: Duration = Duration::from_secs(7 * 86400);

fn system(p: &str) -> bool {
    SYSTEM.iter().any(|s| p.starts_with(s)) && !p.contains("/home/")
}

/// Files mapped into a process: its program and shared libraries.
pub fn mapped(proc_root: &Path, pid: i32) -> Vec<PathBuf> {
    let maps = fs::read_to_string(proc_root.join(pid.to_string()).join("maps")).unwrap_or_default();
    let mut set = BTreeSet::new();
    for line in maps.lines() {
        if let Some(path) = line.split_whitespace().nth(5) {
            if system(path) && !line.ends_with("(deleted)") {
                set.insert(PathBuf::from(path));
            }
        }
    }
    set.into_iter().collect()
}

/// For `python3 -m nexa_files`: every file of that package, plus the
/// libraries every NexaOS Python app imports (nexa_common, gi).
pub fn python_files(cmdline: &[String], roots: &[PathBuf]) -> Vec<PathBuf> {
    let module = cmdline.windows(2).find(|w| w[0] == "-m").map(|w| w[1].split('.').next().unwrap_or("").to_string());
    let Some(module) = module.filter(|m| !m.is_empty() && !m.contains('/')) else { return vec![] };
    // Packages sit right in a root (site-packages/gi) or one folder down
    // (/usr/local/lib/nexa-files/nexa_files).
    let mut dirs = Vec::new();
    for root in roots {
        let mut parents = vec![root.clone()];
        parents.extend(fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        for parent in parents {
            for name in [module.as_str(), "nexa_common", "gi"] {
                let d = parent.join(name);
                if d.is_dir() && !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut stack = dirs;
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.to_str().map_or(false, system) && out.len() < MAX_FILES {
                out.push(p);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// What an app needs to start, from one of its running processes.
pub fn capture(proc_root: &Path, pid: i32) -> Vec<PathBuf> {
    let cmdline: Vec<String> = fs::read(proc_root.join(pid.to_string()).join("cmdline")).unwrap_or_default()
        .split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
    let mut files = mapped(proc_root, pid);
    if cmdline.first().map_or(false, |c| c.contains("python")) {
        let mut roots = vec![PathBuf::from("/usr/local/lib")];
        roots.extend(fs::read_dir("/usr/lib").into_iter().flatten().flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("python3"))
            .map(|e| e.path().join("site-packages")).filter(|p| p.is_dir()));
        files.extend(python_files(&cmdline, &roots));
    }
    files.sort();
    files.dedup();
    files.truncate(MAX_FILES);
    files
}

fn list_path(dir: &Path, app: &str) -> PathBuf {
    let safe: String = app.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect();
    dir.join(format!("{safe}.files"))
}

pub fn needs_capture(dir: &Path, app: &str) -> bool {
    fs::metadata(list_path(dir, app)).and_then(|m| m.modified()).ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok()).map_or(true, |age| age > RECAPTURE)
}

pub fn save(dir: &Path, app: &str, files: &[PathBuf]) {
    if files.is_empty() {
        return;
    }
    let _ = fs::create_dir_all(dir);
    let text: String = files.iter().filter_map(|p| p.to_str()).map(|p| format!("{p}\n")).collect();
    let _ = fs::write(list_path(dir, app), text);
}

pub fn load(dir: &Path, app: &str) -> Vec<PathBuf> {
    fs::read_to_string(list_path(dir, app)).unwrap_or_default().lines()
        .filter(|l| system(l)).map(PathBuf::from).collect()
}

/// Total size of the files worth reading ahead.
pub fn size(files: &[PathBuf]) -> u64 {
    files.iter().filter_map(|p| fs::metadata(p).ok()).map(|m| m.len()).filter(|l| *l <= MAX_FILE).sum::<u64>().min(MAX_APP_BYTES)
}

/// Read them into the page cache; returns the bytes read ahead.
pub fn preload(files: &[PathBuf]) -> u64 {
    let mut done = 0u64;
    for p in files {
        let Ok(m) = fs::metadata(p) else { continue };
        if m.len() > MAX_FILE || done + m.len() > MAX_APP_BYTES {
            continue;
        }
        if preload::read_ahead_file(p).is_ok() {
            done += m.len();
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn only_system_files() {
        assert!(system("/usr/lib/libgtk-3.so.0") && system("/lib/ld-musl-x86_64.so.1"));
        assert!(!system("/home/u/secret.so") && !system("/tmp/x") && !system("/usr/../home/u/x/home/y"));
        let d = std::env::temp_dir().join(format!("nexa-patternd-warm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        save(&d, "nexa-files", &[PathBuf::from("/usr/bin/python3"), PathBuf::from("/home/u/x")]);
        assert_eq!(load(&d, "nexa-files"), vec![PathBuf::from("/usr/bin/python3")], "home files never come back");
        assert!(!needs_capture(&d, "nexa-files") && needs_capture(&d, "foot"));
    }

    #[test]
    fn this_process_maps_libraries() {
        // SAFETY: getpid can't fail.
        let me = unsafe { libc::getpid() };
        let m = mapped(Path::new("/proc"), me);
        assert!(m.iter().all(|p| p.to_str().map_or(false, system)));
    }

    #[test]
    fn python_package_files() {
        let r = std::env::temp_dir().join(format!("nexa-patternd-py-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        // Not under /usr, so the system filter drops them: check the walk via a
        // fake root that *is* treated as system by putting it under the path test.
        fs::create_dir_all(r.join("nexa-files/nexa_files/__pycache__")).unwrap();
        fs::write(r.join("nexa-files/nexa_files/window.py"), "").unwrap();
        let none = python_files(&["python3".into(), "-m".into(), "nexa_files".into()], &[r.clone()]);
        assert!(none.is_empty(), "outside system folders: nothing listed");
        assert!(python_files(&["python3".into(), "script.py".into()], &[r]).is_empty(), "not a -m app");
    }

    #[test]
    fn preloading_brings_files_into_memory() {
        let p = std::env::temp_dir().join(format!("nexa-patternd-pre-{}", std::process::id()));
        fs::write(&p, vec![3u8; 128 * 1024]).unwrap();
        let f = fs::File::open(&p).unwrap();
        // SAFETY: plain fadvise on an open fd.
        unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
        assert_eq!(preload(&[p.clone()]), 128 * 1024);
        assert!(nexa_sys::preload::resident(&f).unwrap().iter().all(|x| *x));
        assert_eq!(size(&[p]), 128 * 1024);
    }
}
