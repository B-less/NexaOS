//! Writing and reading .nexa files (zip): deterministic output, strict and
//! complete verification on the way in.

use crate::manifest::{is_safe_path, Manifest};
use crate::sign;
use ed25519_dalek::SigningKey;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

/// Hard limits, so a hostile package can't exhaust memory or disk.
pub const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_FILES: usize = 20_000;

#[derive(Debug, PartialEq)]
pub enum Trust {
    /// Signed by a key in the trust directory.
    Trusted(String),
    /// Signed, but by a key this system doesn't know: the signature can't be
    /// checked, so the package is treated like an unsigned one.
    UnknownPublisher(String),
    Unsigned,
}

impl Trust {
    pub fn label(&self) -> &'static str {
        match self {
            Trust::Trusted(_) => "trusted",
            Trust::UnknownPublisher(_) => "unknown-publisher",
            Trust::Unsigned => "unsigned",
        }
    }
    pub fn publisher(&self) -> Option<&str> {
        match self {
            Trust::Trusted(id) | Trust::UnknownPublisher(id) => Some(id),
            Trust::Unsigned => None,
        }
    }
}

#[derive(Debug)]
pub struct Verified {
    pub manifest: Manifest,
    pub trust: Trust,
    /// Payload files (path -> bytes), already checked against the manifest.
    pub files: BTreeMap<String, Vec<u8>>,
}

fn mode_for(path: &str) -> u32 {
    if path.starts_with("bin/") { 0o755 } else { 0o644 }
}

/// Build the package bytes: manifest.json, SIGNATURE (if a key is given),
/// then the files in sorted order, all with a fixed timestamp, so the same
/// input always gives the same package.
pub fn write(manifest: &Manifest, files: &BTreeMap<String, Vec<u8>>, key: Option<&SigningKey>) -> Result<Vec<u8>, String> {
    manifest.validate()?;
    for (path, data) in files {
        if manifest.files.get(path).map(String::as_str) != Some(sign::sha256_hex(data).as_str()) {
            return Err(format!("{path} doesn't match the manifest"));
        }
    }
    if files.len() != manifest.files.len() {
        return Err("manifest lists files that aren't in the package".into());
    }
    let manifest_bytes = manifest.to_bytes();
    let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = |mode| {
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(DateTime::default())
            .unix_permissions(mode)
    };
    let mut put = |name: &str, data: &[u8], mode| -> Result<(), String> {
        zw.start_file(name, opts(mode)).map_err(|e| e.to_string())?;
        zw.write_all(data).map_err(|e| e.to_string())
    };
    put("manifest.json", &manifest_bytes, 0o644)?;
    if let Some(key) = key {
        put("SIGNATURE", sign::signature_text(key, &manifest_bytes).as_bytes(), 0o644)?;
    }
    for (path, data) in files {
        put(path, data, mode_for(path))?;
    }
    Ok(zw.finish().map_err(|e| e.to_string())?.into_inner())
}

fn read_limited(entry: &mut impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    entry.take(limit + 1).read_to_end(&mut buf).map_err(|e| format!("corrupt entry: {e}"))?;
    if buf.len() as u64 > limit {
        return Err("a file in the package is too large".into());
    }
    Ok(buf)
}

/// Check everything: zip structure, paths, manifest rules, every file's
/// SHA-256 against the manifest (nothing extra, nothing missing), and the
/// signature against the trusted keys. Any problem is an error.
pub fn verify(bytes: &[u8], trust_dir: Option<&Path>) -> Result<Verified, String> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err("package is too large".into());
    }
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a .nexa package: {e}"))?;
    if zip.len() > MAX_FILES + 2 {
        return Err("package has too many files".into());
    }
    let mut manifest_bytes = None;
    let mut signature = None;
    let mut files = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("corrupt package: {e}"))?;
        let name = entry.name().to_string();
        if !seen.insert(name.clone()) {
            return Err(format!("'{name}' appears twice in the package"));
        }
        if entry.is_dir() {
            continue;
        }
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                return Err(format!("'{name}' is a symbolic link; packages may not contain links"));
            }
        }
        if !is_safe_path(&name) {
            return Err(format!("unsafe path in package: {name:?}"));
        }
        let data = read_limited(&mut entry, MAX_FILE_BYTES)?;
        total += data.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err("package unpacks to more than 2 GiB".into());
        }
        match name.as_str() {
            "manifest.json" => manifest_bytes = Some(data),
            "SIGNATURE" => signature = Some(data),
            _ => {
                files.insert(name, data);
            }
        }
    }
    let manifest_bytes = manifest_bytes.ok_or("package has no manifest.json")?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|e| format!("bad manifest.json: {e}"))?;
    manifest.validate()?;
    for (path, data) in &files {
        match manifest.files.get(path) {
            None => return Err(format!("'{path}' is in the package but not in the manifest")),
            Some(h) if *h != sign::sha256_hex(data) => return Err(format!("'{path}' was changed after the package was made")),
            _ => {}
        }
    }
    if let Some(missing) = manifest.files.keys().find(|p| !files.contains_key(*p)) {
        return Err(format!("'{missing}' is listed in the manifest but missing"));
    }
    let trust = match signature {
        None => Trust::Unsigned,
        Some(sig) => {
            let text = std::str::from_utf8(&sig).map_err(|_| "malformed SIGNATURE")?;
            let (id, sig) = sign::parse_signature(text)?;
            match trust_dir.and_then(|d| sign::trusted_key(d, &id)) {
                Some(key) if sign::verify(&key, &manifest_bytes, &sig) => Trust::Trusted(id),
                Some(_) => return Err("the signature doesn't match: the package was changed after signing".into()),
                None => Trust::UnknownPublisher(id),
            }
        }
    };
    Ok(Verified { manifest, trust, files })
}

/// Write verified files into `dest` (which must not exist yet): created
/// 0700, bin/ executable, nothing outside it, no links.
pub fn extract(v: &Verified, dest: &Path) -> Result<(), String> {
    fs::DirBuilder::new().mode(0o700).create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    for (path, data) in &v.files {
        let target = dest.join(path);
        if let Some(parent) = target.parent() {
            fs::DirBuilder::new().recursive(true).mode(0o755).create(parent).map_err(|e| e.to_string())?;
        }
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode_for(path))
            .custom_flags(libc_o_nofollow())
            .open(&target)
            .map_err(|e| format!("{path}: {e}"))?;
        f.write_all(data).map_err(|e| format!("{path}: {e}"))?;
        fs::set_permissions(&target, fs::Permissions::from_mode(mode_for(path))).map_err(|e| e.to_string())?;
    }
    Ok(())
}

const fn libc_o_nofollow() -> i32 {
    0o400000 // O_NOFOLLOW on Linux x86_64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ABI, ARCH, FORMAT};

    fn sample() -> (Manifest, BTreeMap<String, Vec<u8>>) {
        let mut files = BTreeMap::new();
        files.insert("bin/hello".to_string(), b"#!/bin/sh\necho hi\n".to_vec());
        files.insert("lib/readme.txt".to_string(), b"hello".to_vec());
        let hashes = files.iter().map(|(p, d)| (p.clone(), sign::sha256_hex(d))).collect();
        let m = Manifest {
            format: FORMAT, id: "hello".into(), name: "Hello".into(), version: "1.0".into(),
            description: "test".into(), categories: "Utility;".into(), icon: None,
            arch: ARCH.into(), abi: ABI.into(), depends: vec![], permissions: vec![],
            terminal: false, files: hashes,
        };
        (m, files)
    }

    fn trust_dir_with(key: &SigningKey) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nexa-trust-{}-{}", std::process::id(), sign::key_id(&key.verifying_key())));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{}.pub", sign::key_id(&key.verifying_key()))), sign::public_text(&key.verifying_key())).unwrap();
        dir
    }

    /// Rewrite one zip entry's data in a built package (a tampering attacker).
    fn tamper(pkg: &[u8], name: &str, new: &[u8]) -> Vec<u8> {
        let mut zip = ZipArchive::new(Cursor::new(pkg)).unwrap();
        let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..zip.len() {
            let mut e = zip.by_index(i).unwrap();
            let n = e.name().to_string();
            let mut d = Vec::new();
            e.read_to_end(&mut d).unwrap();
            zw.start_file(n.clone(), SimpleFileOptions::default()).unwrap();
            zw.write_all(if n == name { new } else { &d }).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }

    #[test]
    fn deterministic() {
        let (m, f) = sample();
        assert_eq!(write(&m, &f, None).unwrap(), write(&m, &f, None).unwrap());
    }

    #[test]
    fn trusted_unknown_unsigned() {
        let (m, f) = sample();
        let key = sign::generate().unwrap();
        let dir = trust_dir_with(&key);
        let signed = write(&m, &f, Some(&key)).unwrap();
        assert!(matches!(verify(&signed, Some(&dir)).unwrap().trust, Trust::Trusted(_)));
        assert!(matches!(verify(&signed, None).unwrap().trust, Trust::UnknownPublisher(_)));
        assert_eq!(verify(&write(&m, &f, None).unwrap(), Some(&dir)).unwrap().trust, Trust::Unsigned);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tampering_is_caught() {
        let (m, f) = sample();
        let key = sign::generate().unwrap();
        let dir = trust_dir_with(&key);
        let signed = write(&m, &f, Some(&key)).unwrap();
        let changed_file = tamper(&signed, "bin/hello", b"#!/bin/sh\nrm -rf ~\n");
        assert!(verify(&changed_file, Some(&dir)).unwrap_err().contains("changed after"));
        // attacker also fixes the hash in the manifest: the signature catches it
        let mut m2 = m.clone();
        m2.files.insert("bin/hello".into(), sign::sha256_hex(b"#!/bin/sh\nrm -rf ~\n"));
        let both = tamper(&tamper(&signed, "bin/hello", b"#!/bin/sh\nrm -rf ~\n"), "manifest.json", &m2.to_bytes());
        assert!(verify(&both, Some(&dir)).unwrap_err().contains("signature doesn't match"));
        // an extra file not in the manifest
        let mut extra = ZipWriter::new(Cursor::new(Vec::new()));
        let mut zip = ZipArchive::new(Cursor::new(&signed[..])).unwrap();
        for i in 0..zip.len() {
            extra.raw_copy_file(zip.by_index(i).unwrap()).unwrap();
        }
        extra.start_file("lib/evil.so", SimpleFileOptions::default()).unwrap();
        extra.write_all(b"x").unwrap();
        assert!(verify(&extra.finish().unwrap().into_inner(), Some(&dir)).unwrap_err().contains("not in the manifest"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn hostile_archives() {
        let (m, f) = sample();
        let good = write(&m, &f, None).unwrap();
        // zip-slip path
        let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
        zw.start_file("../../etc/cron.d/x", SimpleFileOptions::default()).unwrap();
        zw.write_all(b"x").unwrap();
        assert!(verify(&zw.finish().unwrap().into_inner(), None).unwrap_err().contains("unsafe path"));
        // symlink entry
        let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
        zw.add_symlink("bin/hello", "/bin/su", SimpleFileOptions::default()).unwrap();
        assert!(verify(&zw.finish().unwrap().into_inner(), None).unwrap_err().contains("symbolic link"));
        // not a zip at all
        assert!(verify(b"not a zip", None).is_err());
        assert!(verify(&good, None).is_ok());
    }

    #[test]
    fn extract_writes_modes_and_refuses_existing() {
        let (m, f) = sample();
        let v = verify(&write(&m, &f, None).unwrap(), None).unwrap();
        let dest = std::env::temp_dir().join(format!("nexa-extract-{}", std::process::id()));
        fs::remove_dir_all(&dest).ok();
        extract(&v, &dest).unwrap();
        let mode = |p: &str| fs::metadata(dest.join(p)).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode("bin/hello"), mode("lib/readme.txt")), (0o755, 0o644));
        assert_eq!(fs::read(dest.join("lib/readme.txt")).unwrap(), b"hello");
        assert!(extract(&v, &dest).is_err(), "must not reuse an existing directory");
        fs::remove_dir_all(dest).ok();
    }
}
