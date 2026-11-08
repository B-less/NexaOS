//! Nexa Store catalogues: a folder with `packages/*.nexa`, `icons/` and an
//! `index.json` describing every app (name, version, what it asks for, the
//! package's size and SHA-256), signed as a whole in `index.sig` with the
//! same Ed25519 keys as packages. The store trusts an app's listing only if
//! the index is signed by a trusted key, and a downloaded package only if
//! its SHA-256 matches the index - then the broker verifies the package's
//! own signature again before installing it.
//!
//!   nexa-pack repo build DIR [--key FILE]      (re)write index.json + icons
//!   nexa-pack repo verify DIR [--keys DIR] [--json]

use crate::manifest::is_safe_path;
use crate::package::{self, Trust};
use crate::sign;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const INDEX_FORMAT: u32 = 1;
const MAX_INDEX_BYTES: u64 = 8 << 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub categories: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub terminal: bool,
    /// Who signed the package itself (key id), if anyone.
    #[serde(default)]
    pub publisher: Option<String>,
    /// Package size in bytes and its SHA-256, checked after downloading.
    pub size: u64,
    pub sha256: String,
    /// Paths inside the catalogue folder.
    pub package: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// Pictures of the app: screenshots/<id>-N.png|jpg in the catalogue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Index {
    pub format: u32,
    pub apps: Vec<Entry>,
}

/// "1.10.2" > "1.9" (numeric parts; anything else compares as text).
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split(['.', '-', '+']).map(|p| p.parse::<u64>().map_err(|_| p.to_string())).collect::<Vec<_>>();
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(i), pb.get(i));
        match (x, y) {
            (Some(Ok(x)), Some(Ok(y))) if x != y => return x > y,
            (Some(Err(x)), Some(Err(y))) if x != y => return x > y,
            (Some(Ok(_)), Some(Err(_))) => return true,
            (Some(Err(_)), Some(Ok(_))) => return false,
            (Some(_), None) => return true,
            (None, Some(_)) => return false,
            _ => {}
        }
    }
    false
}

impl Index {
    /// Every path must stay inside the catalogue, every id be a real id.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != INDEX_FORMAT {
            return Err(format!("unsupported catalogue format {}", self.format));
        }
        for e in &self.apps {
            let id_ok = e.id.chars().next().map_or(false, |c| c.is_ascii_lowercase()) && e.id.len() <= 40
                && e.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
            if !id_ok {
                return Err(format!("bad app id in catalogue: {:?}", e.id));
            }
            if !is_safe_path(&e.package) || !e.package.starts_with("packages/") || !e.package.ends_with(".nexa") {
                return Err(format!("bad package path for {}: {:?}", e.id, e.package));
            }
            if let Some(icon) = &e.icon {
                if !is_safe_path(icon) || !icon.starts_with("icons/") {
                    return Err(format!("bad icon path for {}: {:?}", e.id, icon));
                }
            }
            if e.screenshots.len() > crate::manifest::MAX_SCREENSHOTS
                || e.screenshots.iter().any(|s| !is_safe_path(s) || !s.starts_with("screenshots/")) {
                return Err(format!("bad screenshots for {}", e.id));
            }
            if e.sha256.len() != 64 || !e.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("bad checksum for {}", e.id));
            }
        }
        Ok(())
    }
}

/// Write index.json (and index.sig with a key) for the packages in DIR.
pub fn build(dir: &Path, key: Option<&SigningKey>, trust_dir: Option<&Path>) -> Result<Index, String> {
    let pkg_dir = dir.join("packages");
    let mut names: Vec<String> = fs::read_dir(&pkg_dir).map_err(|e| format!("{}: {e}", pkg_dir.display()))?
        .flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".nexa")).collect();
    names.sort();
    fs::create_dir_all(dir.join("icons")).map_err(|e| e.to_string())?;
    let mut newest: BTreeMap<String, Entry> = BTreeMap::new();
    for name in names {
        let bytes = fs::read(pkg_dir.join(&name)).map_err(|e| format!("{name}: {e}"))?;
        let v = package::verify(&bytes, trust_dir).map_err(|e| format!("{name}: {e}"))?;
        let m = &v.manifest;
        let icon = m.icon.as_ref().and_then(|p| v.files.get(p).map(|data| (p, data))).and_then(|(p, data)| {
            let ext = Path::new(p).extension()?.to_str()?.to_ascii_lowercase();
            (ext == "svg" || ext == "png").then(|| (format!("icons/{}.{ext}", m.id), data))
        });
        if let Some((rel, data)) = &icon {
            fs::write(dir.join(rel), data).map_err(|e| e.to_string())?;
        }
        let mut shots = Vec::new();
        for (i, s) in m.screenshots.iter().enumerate() {
            if let Some(data) = v.files.get(s) {
                let ext = if s.ends_with(".jpg") { "jpg" } else { "png" };
                let rel = format!("screenshots/{}-{}.{ext}", m.id, i + 1);
                fs::create_dir_all(dir.join("screenshots")).map_err(|e| e.to_string())?;
                fs::write(dir.join(&rel), data).map_err(|e| e.to_string())?;
                shots.push(rel);
            }
        }
        let entry = Entry {
            id: m.id.clone(), name: m.name.clone(), version: m.version.clone(), description: m.description.clone(),
            categories: m.categories.clone(), permissions: m.permissions.clone(), depends: m.depends.clone(),
            terminal: m.terminal, publisher: v.trust.publisher().map(str::to_string), size: bytes.len() as u64,
            sha256: sign::sha256_hex(&bytes), package: format!("packages/{name}"), icon: icon.map(|(rel, _)| rel), screenshots: shots,
        };
        match newest.get(&m.id) {
            Some(old) if !newer(&entry.version, &old.version) => {}
            _ => {
                newest.insert(m.id.clone(), entry);
            }
        }
    }
    let index = Index { format: INDEX_FORMAT, apps: newest.into_values().collect() };
    index.validate()?;
    let mut text = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
    text.push(b'\n');
    fs::write(dir.join("index.json"), &text).map_err(|e| e.to_string())?;
    match key {
        Some(k) => fs::write(dir.join("index.sig"), sign::signature_text(k, &text)).map_err(|e| e.to_string())?,
        None => {
            let _ = fs::remove_file(dir.join("index.sig"));
        }
    }
    Ok(index)
}

/// Read and check a catalogue: the index's signature (a bad one is an
/// error; none, or an unknown key, is reported), then its contents.
pub fn read(index_bytes: &[u8], sig: Option<&str>, trust_dir: Option<&Path>) -> Result<(Index, Trust), String> {
    if index_bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err("catalogue index is too large".into());
    }
    let trust = match sig {
        None => Trust::Unsigned,
        Some(text) => {
            let (id, signature) = sign::parse_signature(text)?;
            match trust_dir.and_then(|d| sign::trusted_key(d, &id)) {
                Some(k) if sign::verify(&k, index_bytes, &signature) => Trust::Trusted(id),
                Some(_) => return Err("the catalogue's signature doesn't match: it was changed after signing".into()),
                None => Trust::UnknownPublisher(id),
            }
        }
    };
    let index: Index = serde_json::from_slice(index_bytes).map_err(|e| format!("bad catalogue index: {e}"))?;
    index.validate()?;
    Ok((index, trust))
}

pub fn verify_dir(dir: &Path, trust_dir: Option<&Path>) -> Result<(Index, Trust), String> {
    let bytes = fs::read(dir.join("index.json")).map_err(|e| format!("{}: {e}", dir.display()))?;
    let sig = fs::read_to_string(dir.join("index.sig")).ok();
    read(&bytes, sig.as_deref(), trust_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;

    fn package(dir: &Path, id: &str, version: &str, key: &SigningKey) {
        let mut m = Manifest {
            format: crate::manifest::FORMAT, id: id.into(), name: format!("App {id}"), version: version.into(),
            description: "A test app".into(), categories: "Utility;".into(), icon: Some("icon.svg".into()),
            arch: crate::manifest::ARCH.into(), abi: crate::manifest::ABI.into(), depends: vec![],
            permissions: vec!["network".into()], terminal: false, screenshots: vec![], opens: vec![], files: BTreeMap::new(),
        };
        let mut files = BTreeMap::new();
        files.insert(format!("bin/{id}"), b"#!/bin/sh\n".to_vec());
        files.insert("icon.svg".to_string(), b"<svg/>".to_vec());
        m.files = files.iter().map(|(p, d)| (p.clone(), sign::sha256_hex(d))).collect();
        let bytes = package::write(&m, &files, Some(key)).unwrap();
        fs::write(dir.join("packages").join(format!("{id}-{version}.nexa")), bytes).unwrap();
    }

    #[test]
    fn versions() {
        assert!(newer("1.10.0", "1.9.9") && newer("2.0", "1.99") && newer("1.0.1", "1.0") && !newer("1.0", "1.0"));
        assert!(!newer("1.0", "1.0.1"));
    }

    #[test]
    fn build_sign_verify_and_tamper() {
        let d = std::env::temp_dir().join(format!("nexa-pack-repo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("packages")).unwrap();
        fs::create_dir_all(d.join("keys")).unwrap();
        let key = sign::generate().unwrap();
        let id = sign::key_id(&key.verifying_key());
        fs::write(d.join("keys").join(format!("{id}.pub")), sign::public_text(&key.verifying_key())).unwrap();
        package(&d, "notes", "1.0.0", &key);
        package(&d, "notes", "1.2.0", &key);
        package(&d, "clock", "0.3", &key);
        let index = build(&d, Some(&key), Some(&d.join("keys"))).unwrap();
        assert_eq!(index.apps.iter().map(|a| (a.id.as_str(), a.version.as_str())).collect::<Vec<_>>(),
                   vec![("clock", "0.3"), ("notes", "1.2.0")], "newest version of each app");
        assert_eq!(index.apps[1].publisher.as_deref(), Some(id.as_str()));
        assert_eq!(index.apps[1].icon.as_deref(), Some("icons/notes.svg"));
        assert!(d.join("icons/notes.svg").exists());
        let (_, trust) = verify_dir(&d, Some(&d.join("keys"))).unwrap();
        assert_eq!(trust, Trust::Trusted(id.clone()));
        // Not trusted here: reported, not failed.
        assert!(matches!(verify_dir(&d, Some(&d.join("packages"))).unwrap().1, Trust::UnknownPublisher(_)));
        // Changed after signing: refused.
        let text = fs::read_to_string(d.join("index.json")).unwrap().replace("1.2.0", "9.9.9");
        fs::write(d.join("index.json"), text).unwrap();
        assert!(verify_dir(&d, Some(&d.join("keys"))).unwrap_err().contains("changed after signing"));
        // A path escaping the catalogue: refused even unsigned.
        let evil = br#"{"format":1,"apps":[{"id":"x","name":"X","version":"1","size":1,"sha256":"0000000000000000000000000000000000000000000000000000000000000000","package":"packages/../../etc/passwd.nexa"}]}"#;
        assert!(read(evil, None, None).unwrap_err().contains("bad package path"));
    }
}
