//! Installed models and verified downloads.
//!
//! A download goes to <file>.part (curl, HTTPS only, resumable), then must
//! match the catalogue's exact size and SHA-256 before it is renamed into
//! place; a <file>.verified marker records the check so loading doesn't
//! re-hash a gigabyte every time.

use crate::catalog::{Model, MODELS};
use crate::config::Config;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

pub fn path(cfg: &Config, m: &Model) -> PathBuf {
    cfg.models_dir().join(m.file)
}

fn marker(cfg: &Config, m: &Model) -> PathBuf {
    cfg.models_dir().join(format!("{}.verified", m.file))
}

pub fn is_installed(cfg: &Config, m: &Model) -> bool {
    let size_ok = fs::metadata(path(cfg, m)).map(|md| md.len() == m.size).unwrap_or(false);
    let marked = fs::read_to_string(marker(cfg, m)).map(|s| s.trim() == m.sha256).unwrap_or(false);
    size_ok && marked
}

pub fn installed(cfg: &Config) -> Vec<&'static str> {
    MODELS.iter().filter(|m| is_installed(cfg, m)).map(|m| m.name).collect()
}

pub fn sha256_file(p: &std::path::Path) -> std::io::Result<String> {
    let mut f = fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Check a finished file against the catalogue and put it in place.
pub fn accept(cfg: &Config, m: &Model, part: &std::path::Path) -> Result<(), String> {
    let len = fs::metadata(part).map_err(|e| e.to_string())?.len();
    if len != m.size {
        return Err(format!("download is {len} bytes, expected {}", m.size));
    }
    let sum = sha256_file(part).map_err(|e| e.to_string())?;
    if sum != m.sha256 {
        fs::remove_file(part).ok();
        return Err("downloaded file doesn't match its checksum; deleted".into());
    }
    fs::rename(part, path(cfg, m)).map_err(|e| e.to_string())?;
    fs::write(marker(cfg, m), format!("{}\n", m.sha256)).map_err(|e| e.to_string())
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Phase {
    Downloading,
    Verifying,
    Done,
    Failed(String),
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Progress {
    pub phase: Phase,
    pub bytes: u64,
    pub total: u64,
}

pub type Downloads = Arc<Mutex<HashMap<String, Progress>>>;

/// Start a background download unless one is running or it's installed.
pub fn start_download(cfg: &Config, m: &'static Model, downloads: &Downloads) -> bool {
    {
        let mut d = downloads.lock().unwrap();
        if is_installed(cfg, m) || matches!(d.get(m.name), Some(p) if p.phase == Phase::Downloading || p.phase == Phase::Verifying) {
            return false;
        }
        d.insert(m.name.into(), Progress { phase: Phase::Downloading, bytes: 0, total: m.size });
    }
    let (cfg, downloads) = (cfg.clone(), downloads.clone());
    std::thread::spawn(move || {
        let set = |phase: Phase, bytes| {
            downloads.lock().unwrap().insert(m.name.into(), Progress { phase, bytes, total: m.size });
        };
        fs::create_dir_all(cfg.models_dir()).ok();
        let part = cfg.models_dir().join(format!("{}.part", m.file));
        let mut child = match Command::new(&cfg.curl)
            .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "3", "-C", "-", "-o"])
            .arg(&part).arg(m.url).spawn()
        {
            Ok(c) => c,
            Err(e) => return set(Phase::Failed(format!("can't run curl: {e}")), 0),
        };
        loop {
            let bytes = fs::metadata(&part).map(|md| md.len()).unwrap_or(0);
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break,
                Ok(Some(status)) => return set(Phase::Failed(format!("download failed ({status})")), bytes),
                Ok(None) => set(Phase::Downloading, bytes),
                Err(e) => return set(Phase::Failed(e.to_string()), bytes),
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        set(Phase::Verifying, m.size);
        match accept(&cfg, m, &part) {
            Ok(()) => set(Phase::Done, m.size),
            Err(e) => set(Phase::Failed(e), m.size),
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Role;

    fn cfg() -> Config {
        let mut c = Config::from_env();
        c.state_dir = std::env::temp_dir().join(format!("nexa-models-{}", std::process::id()));
        fs::create_dir_all(c.models_dir()).unwrap();
        c
    }

    #[test]
    fn only_exact_files_are_accepted() {
        let c = cfg();
        let data = b"pretend weights";
        let sum: String = Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect();
        let sha: &'static str = Box::leak(sum.into_boxed_str());
        let m: &'static Model = Box::leak(Box::new(Model { name: "t", title: "t", file: "t.gguf", url: "https://x", size: data.len() as u64,
                                                           sha256: sha, licence: "MIT", role: Role::Chat, ram_mb: 1 }));
        let part = c.models_dir().join("t.gguf.part");
        fs::write(&part, b"pretend weightz").unwrap();
        assert!(accept(&c, m, &part).unwrap_err().contains("checksum"));
        assert!(!part.exists() && !is_installed(&c, m), "a bad download is deleted, not kept");
        fs::write(&part, b"short").unwrap();
        assert!(accept(&c, m, &part).unwrap_err().contains("bytes"));
        fs::write(&part, data).unwrap();
        accept(&c, m, &part).unwrap();
        assert!(is_installed(&c, m));
        fs::write(path(&c, m), b"pretend weightZ").unwrap();
        // same size, no re-hash at load time, but the marker stays tied to the catalogue hash
        assert!(is_installed(&c, m));
        fs::remove_dir_all(&c.state_dir).ok();
    }
}
