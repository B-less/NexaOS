//! Installed models and verified downloads.
//!
//! A download goes to <file>.part (curl, HTTPS only, resumable), then must
//! match the catalogue's exact size and SHA-256 before it is renamed into
//! place; a <file>.verified marker records the check so loading doesn't
//! re-hash a gigabyte every time.

use crate::catalog::{Model, MODELS};
#[cfg(test)]
use crate::catalog::Extra;
use crate::config::Config;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// One file to fetch and check: a model's own, or one of its extras.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub file: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub fn items(m: &Model) -> Vec<Item> {
    std::iter::once(Item { file: m.file, url: m.url, size: m.size, sha256: m.sha256 })
        .chain(m.extras.iter().map(|e| Item { file: e.file, url: e.url, size: e.size, sha256: e.sha256 }))
        .collect()
}

pub fn path(cfg: &Config, m: &Model) -> PathBuf {
    cfg.models_dir().join(m.file)
}

fn marker(cfg: &Config, it: &Item) -> PathBuf {
    cfg.models_dir().join(format!("{}.verified", it.file))
}

fn item_ok(cfg: &Config, it: &Item) -> bool {
    let size_ok = fs::metadata(cfg.models_dir().join(it.file)).map(|md| md.len() == it.size).unwrap_or(false);
    let marked = fs::read_to_string(marker(cfg, it)).map(|s| s.trim() == it.sha256).unwrap_or(false);
    size_ok && marked
}

/// The model and every file that comes with it are in place and checked.
pub fn is_installed(cfg: &Config, m: &Model) -> bool {
    items(m).iter().all(|it| item_ok(cfg, it))
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
pub fn accept(cfg: &Config, it: &Item, part: &std::path::Path) -> Result<(), String> {
    let len = fs::metadata(part).map_err(|e| e.to_string())?.len();
    if len != it.size {
        return Err(format!("download is {len} bytes, expected {}", it.size));
    }
    let sum = sha256_file(part).map_err(|e| e.to_string())?;
    if sum != it.sha256 {
        fs::remove_file(part).ok();
        return Err("downloaded file doesn't match its checksum; deleted".into());
    }
    fs::rename(part, cfg.models_dir().join(it.file)).map_err(|e| e.to_string())?;
    fs::write(marker(cfg, it), format!("{}\n", it.sha256)).map_err(|e| e.to_string())
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
/// The model's own file comes first, then the files that come with it;
/// progress counts them all.
pub fn start_download(cfg: &Config, m: &'static Model, downloads: &Downloads) -> bool {
    let total: u64 = items(m).iter().map(|it| it.size).sum();
    {
        let mut d = downloads.lock().unwrap();
        if is_installed(cfg, m) || matches!(d.get(m.name), Some(p) if p.phase == Phase::Downloading || p.phase == Phase::Verifying) {
            return false;
        }
        d.insert(m.name.into(), Progress { phase: Phase::Downloading, bytes: 0, total });
    }
    let (cfg, downloads) = (cfg.clone(), downloads.clone());
    std::thread::spawn(move || {
        let set = |phase: Phase, bytes| {
            downloads.lock().unwrap().insert(m.name.into(), Progress { phase, bytes, total });
        };
        fs::create_dir_all(cfg.models_dir()).ok();
        let mut before = 0;
        for it in items(m) {
            if item_ok(&cfg, &it) {
                before += it.size;
                continue;
            }
            let part = cfg.models_dir().join(format!("{}.part", it.file));
            let mut child = match Command::new(&cfg.curl)
                .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "3", "-C", "-", "-o"])
                .arg(&part).arg(it.url).spawn()
            {
                Ok(c) => c,
                Err(e) => return set(Phase::Failed(format!("can't run curl: {e}")), before),
            };
            loop {
                let bytes = before + fs::metadata(&part).map(|md| md.len()).unwrap_or(0);
                match child.try_wait() {
                    Ok(Some(status)) if status.success() => break,
                    Ok(Some(status)) => return set(Phase::Failed(format!("download failed ({status})")), bytes),
                    Ok(None) => set(Phase::Downloading, bytes),
                    Err(e) => return set(Phase::Failed(e.to_string()), bytes),
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            set(Phase::Verifying, before + it.size);
            if let Err(e) = accept(&cfg, &it, &part) {
                return set(Phase::Failed(e), before + it.size);
            }
            before += it.size;
        }
        set(Phase::Done, total)
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Role;

    fn cfg(name: &str) -> Config {
        let mut c = Config::from_env();
        c.state_dir = std::env::temp_dir().join(format!("nexa-models-{name}-{}", std::process::id()));
        fs::create_dir_all(c.models_dir()).unwrap();
        c
    }

    fn leak(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }

    fn sha(data: &[u8]) -> &'static str {
        leak(Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect())
    }

    #[test]
    fn only_exact_files_are_accepted() {
        let c = cfg("exact");
        let data = b"pretend weights";
        let m: &'static Model = Box::leak(Box::new(Model { name: "t", title: "t", file: "t.gguf", url: "https://x", size: data.len() as u64,
                                                           sha256: sha(data), licence: "MIT", role: Role::Chat, ram_mb: 1, extras: &[] }));
        let it = items(m)[0];
        let part = c.models_dir().join("t.gguf.part");
        fs::write(&part, b"pretend weightz").unwrap();
        assert!(accept(&c, &it, &part).unwrap_err().contains("checksum"));
        assert!(!part.exists() && !is_installed(&c, m), "a bad download is deleted, not kept");
        fs::write(&part, b"short").unwrap();
        assert!(accept(&c, &it, &part).unwrap_err().contains("bytes"));
        fs::write(&part, data).unwrap();
        accept(&c, &it, &part).unwrap();
        assert!(is_installed(&c, m));
        fs::write(path(&c, m), b"pretend weightZ").unwrap();
        // same size, no re-hash at load time, but the marker stays tied to the catalogue hash
        assert!(is_installed(&c, m));
        fs::remove_dir_all(&c.state_dir).ok();
    }

    #[test]
    fn a_model_needs_its_extras() {
        let c = cfg("extras");
        let (w, v) = (b"weights".as_slice(), b"a voice".as_slice());
        let extras: &'static [Extra] = Box::leak(Box::new([Extra { file: "v.bin", url: "https://x/v", size: v.len() as u64, sha256: sha(v) }]));
        let m: &'static Model = Box::leak(Box::new(Model { name: "k", title: "k", file: "k.onnx", url: "https://x/k", size: w.len() as u64,
                                                           sha256: sha(w), licence: "MIT", role: Role::Voice, ram_mb: 1, extras }));
        let its = items(m);
        assert_eq!(its.iter().map(|i| i.file).collect::<Vec<_>>(), ["k.onnx", "v.bin"]);
        let part = c.models_dir().join("k.part");
        fs::write(&part, w).unwrap();
        accept(&c, &its[0], &part).unwrap();
        assert!(!is_installed(&c, m), "the voice is still missing");
        fs::write(&part, v).unwrap();
        accept(&c, &its[1], &part).unwrap();
        assert!(is_installed(&c, m));
        fs::remove_dir_all(&c.state_dir).ok();
    }
}
