//! Which paths the assistant may touch: inside the user's home folder only,
//! never secrets (SSH and GPG keys, keyrings, password stores, browsers'
//! saved logins) and never Nexa Mind's own records (undo store, activity
//! log). Paths are resolved first, so "..", symlinks and "~/x/../.ssh"
//! can't sneak around the rules.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// Relative to home. A path equal to or inside any of these is refused.
pub const SENSITIVE: &[&str] = &[
    ".ssh", ".gnupg", ".password-store", ".pki",
    ".local/share/keyrings", ".local/share/nexa-mind", ".local/share/nexa-browser",
    ".mozilla", ".config/chromium", ".config/google-chrome", ".config/nexa-ai",
    ".netrc", ".git-credentials", ".config/gh", ".aws", ".kube", ".docker",
];

#[derive(Debug, PartialEq)]
pub enum PathError {
    OutsideHome(String),
    Sensitive(String),
    Invalid(String),
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PathError::OutsideHome(p) => write!(f, "{p} is outside your home folder"),
            PathError::Sensitive(p) => write!(f, "{p} holds private keys or records; the assistant may not touch it"),
            PathError::Invalid(p) => write!(f, "{p} is not a valid path"),
        }
    }
}

/// Make a path absolute and clean without requiring it to exist: "~" and
/// relative paths are taken from home; ".." is resolved; the deepest
/// existing ancestor is resolved through symlinks (so a link pointing
/// outside home is caught).
pub fn resolve(home: &Path, raw: &str) -> Result<PathBuf, PathError> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(PathError::Invalid(raw.into()));
    }
    let joined = if raw == "~" {
        home.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else if raw.starts_with('/') {
        PathBuf::from(raw)
    } else {
        home.join(raw)
    };
    let mut clean = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::RootDir => clean.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            Component::Normal(p) => clean.push(p),
            Component::Prefix(_) => return Err(PathError::Invalid(raw.into())),
        }
    }
    // Resolve symlinks in the part that exists.
    let mut existing = clean.clone();
    let mut tail = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => tail.push(name.to_os_string()),
            None => break,
        }
        if !existing.pop() {
            break;
        }
    }
    let mut real = existing.canonicalize().unwrap_or(existing);
    let mut matching = true;
    for name in tail.into_iter().rev() {
        // "~/downloads" means ~/Downloads: a missing name is matched to a
        // folder whose name differs only in case, if exactly one does.
        // Folders only, so a write can never land on a different file.
        if matching {
            if let Some(dir) = folder_ignoring_case(&real, &name) {
                real = real.join(dir).canonicalize().unwrap_or_else(|_| real.join(&name));
                continue;
            }
            matching = false;
        }
        real.push(name);
    }
    Ok(real)
}

fn folder_ignoring_case(parent: &Path, name: &std::ffi::OsStr) -> Option<std::ffi::OsString> {
    let want = name.to_str()?.to_lowercase();
    let mut found = fs::read_dir(parent).ok()?.flatten()
        .filter(|e| e.file_name().to_str().map_or(false, |n| n.to_lowercase() == want))
        .filter(|e| e.path().is_dir());
    let first = found.next()?;
    found.next().is_none().then(|| first.file_name())
}

/// "~/x doesn't exist", plus what is there, so the model can correct
/// itself instead of asking the user for a path.
pub fn not_found(home: &Path, p: &Path) -> String {
    let shown = |q: &Path| q.strip_prefix(home).map(|r| if r.as_os_str().is_empty() { "~".to_string() } else { format!("~/{}", r.display()) })
        .unwrap_or_else(|_| q.display().to_string());
    let mut msg = format!("{} doesn't exist", shown(p));
    let mut dir = p.parent().map(Path::to_path_buf);
    while let Some(d) = dir.clone() {
        if d.is_dir() {
            break;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    if let Some(d) = dir.filter(|d| d.starts_with(home)) {
        let mut names: Vec<String> = fs::read_dir(&d).into_iter().flatten().flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.') && !is_sensitive(home, &e.path()))
            .map(|e| format!("{}{}", e.file_name().to_string_lossy(), if e.path().is_dir() { "/" } else { "" }))
            .collect();
        names.sort();
        let more = names.len().saturating_sub(30);
        names.truncate(30);
        if names.is_empty() {
            msg += &format!("; {} is empty", shown(&d));
        } else {
            msg += &format!(". {} has: {}{}", shown(&d), names.join(", "), if more > 0 { format!(" and {more} more") } else { String::new() });
        }
    }
    msg
}

/// Resolve and check a path the assistant wants to use.
pub fn allowed(home: &Path, raw: &str) -> Result<PathBuf, PathError> {
    let home_real = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let p = resolve(&home_real, raw)?;
    let rel = p.strip_prefix(&home_real).map_err(|_| PathError::OutsideHome(raw.into()))?;
    for s in SENSITIVE {
        if rel.starts_with(s) {
            return Err(PathError::Sensitive(raw.into()));
        }
    }
    Ok(p)
}

/// For walks (search, organise): skip these folders entirely.
pub fn is_sensitive(home: &Path, p: &Path) -> bool {
    p.strip_prefix(home).map(|rel| SENSITIVE.iter().any(|s| rel.starts_with(s))).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn home() -> PathBuf {
        let h = std::env::temp_dir().join(format!("nexa-paths-{}", std::process::id()));
        fs::create_dir_all(h.join("Documents")).unwrap();
        fs::create_dir_all(h.join(".ssh")).unwrap();
        h.canonicalize().unwrap()
    }

    #[test]
    fn folder_names_ignore_case_but_files_do_not() {
        let h = home();
        fs::create_dir_all(h.join("Downloads/Trip")).unwrap();
        fs::write(h.join("Documents/Report.txt"), "x").unwrap();
        assert_eq!(allowed(&h, "downloads").unwrap(), h.join("Downloads"));
        assert_eq!(allowed(&h, "~/DOWNLOADS/trip/new.txt").unwrap(), h.join("Downloads/Trip/new.txt"));
        assert_eq!(allowed(&h, "documents/report.txt").unwrap(), h.join("Documents/report.txt"), "files keep their exact name");
        fs::create_dir_all(h.join("photos")).unwrap();
        fs::create_dir_all(h.join("Photos")).unwrap();
        assert_eq!(allowed(&h, "PHOTOS").unwrap(), h.join("PHOTOS"), "two candidates: no guessing");
        assert!(matches!(allowed(&h, ".SSH/id_ed25519"), Err(PathError::Sensitive(_))), "case folding leads to .ssh, which is refused");
    }

    #[test]
    fn not_found_lists_what_is_there() {
        let h = home();
        let msg = not_found(&h, &h.join("Downlaods"));
        assert!(msg.starts_with("~/Downlaods doesn't exist. ~ has: ") && msg.contains("Documents/") && !msg.contains(".ssh"), "{msg}");
    }

    #[test]
    fn inside_home_is_fine() {
        let h = home();
        assert_eq!(allowed(&h, "Documents/new.txt").unwrap(), h.join("Documents/new.txt"));
        assert_eq!(allowed(&h, "~/Documents").unwrap(), h.join("Documents"));
        assert_eq!(allowed(&h, &h.join("Documents/a/b").to_string_lossy()).unwrap(), h.join("Documents/a/b"));
        assert_eq!(allowed(&h, "~").unwrap(), h);
    }

    #[test]
    fn escapes_are_refused() {
        let h = home();
        for raw in ["/etc/shadow", "../../etc/passwd", "~/../other-user", "Documents/../../x", "/"] {
            assert!(matches!(allowed(&h, raw), Err(PathError::OutsideHome(_))), "{raw}");
        }
        for raw in [".ssh/id_ed25519", "~/.ssh", "Documents/../.ssh/config", ".local/share/nexa-mind/activity.jsonl",
                    ".local/share/keyrings/login.keyring", ".mozilla/firefox"] {
            assert!(matches!(allowed(&h, raw), Err(PathError::Sensitive(_))), "{raw}");
        }
        assert!(matches!(allowed(&h, ""), Err(PathError::Invalid(_))));
        assert!(matches!(allowed(&h, "a\0b"), Err(PathError::Invalid(_))));
    }

    #[test]
    fn symlinks_out_of_home_are_caught() {
        let h = home();
        let link = h.join("Documents/escape");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink("/etc", &link).unwrap();
        assert!(matches!(allowed(&h, "Documents/escape/passwd"), Err(PathError::OutsideHome(_))));
        let key_link = h.join("Documents/keys");
        let _ = fs::remove_file(&key_link);
        std::os::unix::fs::symlink(h.join(".ssh"), &key_link).unwrap();
        assert!(matches!(allowed(&h, "Documents/keys/id_rsa"), Err(PathError::Sensitive(_))));
    }
}
