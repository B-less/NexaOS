//! Which paths the assistant may touch: inside the user's home folder only,
//! never secrets (SSH and GPG keys, keyrings, password stores, browsers'
//! saved logins) and never Nexa Mind's own records (undo store, activity
//! log). Paths are resolved first, so "..", symlinks and "~/x/../.ssh"
//! can't sneak around the rules.

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
    for name in tail.into_iter().rev() {
        real.push(name);
    }
    Ok(real)
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
