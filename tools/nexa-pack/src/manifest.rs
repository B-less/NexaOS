//! manifest.json of a .nexa package (format 2) and the rules it must follow.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FORMAT: u32 = 2;
pub const ARCH: &str = "x86_64";
pub const ABI: &str = "musl";
/// What an app may ask for; shown before install, enforced later by the
/// Privacy Center. Anything else makes the package invalid.
pub const PERMISSIONS: &[&str] = &[
    "network", "files.home", "files.documents", "files.pictures", "files.music", "files.videos",
    "camera", "microphone", "location", "notifications", "background",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_categories")]
    pub categories: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub arch: String,
    pub abi: String,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub terminal: bool,
    /// Pictures of the app for the Store, inside the package
    /// (screenshots/N.png or .jpg). Left out of the manifest when there are
    /// none, so older packages stay byte-for-byte the same.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<String>,
    /// Every file in the package except manifest.json and SIGNATURE, with
    /// its SHA-256 (hex). Sorted, so the manifest is byte-for-byte stable.
    pub files: BTreeMap<String, String>,
}

pub const MAX_SCREENSHOTS: usize = 6;
pub const MAX_SCREENSHOT_BYTES: usize = 3 << 20;

fn default_categories() -> String {
    "Utility;".into()
}

fn is_id(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some('a'..='z'))
        && s.len() <= 40
        && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

/// Alpine package names: letters, digits and + - _ . (e.g. gtk+3.0).
pub fn is_apk_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().next().map_or(false, |c| c.is_ascii_alphanumeric())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-_.".contains(c))
}

fn printable(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(|c| c.is_control())
}

/// A relative path inside the package: no absolute paths, no "..", no
/// backslashes or odd characters (zip-slip and lookalike tricks).
pub fn is_safe_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 255
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.chars().any(|c| c.is_control())
        && p.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("unsupported package format {} (this NexaOS reads format {FORMAT})", self.format));
        }
        if !is_id(&self.id) {
            return Err(format!("invalid id '{}': lowercase letters, digits, - and _, starting with a letter", self.id));
        }
        if self.name.trim().is_empty() || !printable(&self.name, 80) {
            return Err("name must be 1-80 printable characters".into());
        }
        if self.version.is_empty() || self.version.len() > 32
            || !self.version.chars().all(|c| c.is_ascii_alphanumeric() || ".-+_".contains(c))
        {
            return Err(format!("invalid version '{}'", self.version));
        }
        if !printable(&self.description, 300) || !printable(&self.categories, 120) {
            return Err("description/categories too long or not printable".into());
        }
        if self.arch != ARCH || self.abi != ABI {
            return Err(format!("built for arch {} with abi {}; NexaOS runs {ARCH} with {ABI}", self.arch, self.abi));
        }
        for d in &self.depends {
            if !is_apk_name(d) {
                return Err(format!("invalid dependency name '{d}'"));
            }
        }
        for p in &self.permissions {
            if !PERMISSIONS.contains(&p.as_str()) {
                return Err(format!("unknown permission '{p}'"));
            }
        }
        let entry = format!("bin/{}", self.id);
        if !self.files.contains_key(&entry) {
            return Err(format!("package has no entry point {entry}"));
        }
        if let Some(icon) = &self.icon {
            if !self.files.contains_key(icon) || !(icon.ends_with(".svg") || icon.ends_with(".png")) {
                return Err(format!("icon '{icon}' must be an .svg or .png inside the package"));
            }
        }
        if self.screenshots.len() > MAX_SCREENSHOTS {
            return Err(format!("at most {MAX_SCREENSHOTS} screenshots"));
        }
        for s in &self.screenshots {
            let pic = s.ends_with(".png") || s.ends_with(".jpg");
            if !self.files.contains_key(s) || !s.starts_with("screenshots/") || !pic {
                return Err(format!("screenshot '{s}' must be a .png or .jpg in screenshots/ inside the package"));
            }
        }
        for (path, hash) in &self.files {
            if !is_safe_path(path) || path == "manifest.json" || path == "SIGNATURE" {
                return Err(format!("unsafe or reserved path '{path}'"));
            }
            if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("bad SHA-256 for '{path}'"));
            }
        }
        Ok(())
    }

    /// The exact bytes that get signed and stored as manifest.json.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = serde_json::to_vec_pretty(self).expect("manifest serialises");
        v.push(b'\n');
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample() -> Manifest {
        let mut files = BTreeMap::new();
        files.insert("bin/hello".into(), "a".repeat(64));
        Manifest {
            format: FORMAT, id: "hello".into(), name: "Hello".into(), version: "1.0.0".into(),
            description: String::new(), categories: "Utility;".into(), icon: None, screenshots: vec![],
            arch: ARCH.into(), abi: ABI.into(), depends: vec!["gtk+3.0".into()],
            permissions: vec!["network".into()], terminal: false, files,
        }
    }

    #[test]
    fn valid_sample() {
        assert_eq!(sample().validate(), Ok(()));
    }

    #[test]
    fn rejects_bad_fields() {
        let cases: Vec<(&str, Box<dyn Fn(&mut Manifest)>)> = vec![
            ("id", Box::new(|m| m.id = "../su".into())),
            ("id", Box::new(|m| m.id = "Hello".into())),
            ("version", Box::new(|m| m.version = "1.0; rm -rf".into())),
            ("arch", Box::new(|m| m.arch = "aarch64".into())),
            ("abi", Box::new(|m| m.abi = "gnu".into())),
            ("dependency", Box::new(|m| m.depends = vec!["gtk; reboot".into()])),
            ("permission", Box::new(|m| m.permissions = vec!["root".into()])),
            ("entry point", Box::new(|m| { m.files.clear(); })),
            ("unsafe", Box::new(|m| { m.files.insert("../etc/passwd".into(), "b".repeat(64)); })),
            ("reserved", Box::new(|m| { m.files.insert("SIGNATURE".into(), "b".repeat(64)); })),
            ("SHA-256", Box::new(|m| { m.files.insert("lib/x".into(), "zz".into()); })),
            ("icon", Box::new(|m| m.icon = Some("missing.svg".into()))),
        ];
        for (what, change) in cases {
            let mut m = sample();
            change(&mut m);
            let err = m.validate().expect_err(what);
            assert!(err.contains(what), "{what}: {err}");
        }
    }

    #[test]
    fn safe_paths() {
        for ok in ["bin/hello", "lib/share/a.txt", "icon.svg"] {
            assert!(is_safe_path(ok), "{ok}");
        }
        for bad in ["", "/etc/passwd", "../x", "a/../b", "a//b", "./a", "a\\b", "a\n"] {
            assert!(!is_safe_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn bytes_are_stable() {
        assert_eq!(sample().to_bytes(), sample().to_bytes());
        let back: Manifest = serde_json::from_slice(&sample().to_bytes()).unwrap();
        assert_eq!(back, sample());
    }
}
