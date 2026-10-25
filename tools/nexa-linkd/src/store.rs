//! What Nexa Link keeps on disk, in ~/.local/share/nexa-link (0700):
//! this device's key pair (identity.json, 0600) and the devices it's
//! paired with (paired.json): their public key, the beacon key made at
//! pairing, and what each may do.

use crate::proto;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct Identity {
    pub name: String,
    pub secret: String,
    pub public: String,
}

impl Identity {
    pub fn secret(&self) -> Vec<u8> {
        proto::unhex(&self.secret).unwrap_or_default()
    }
    pub fn public(&self) -> Vec<u8> {
        proto::unhex(&self.public).unwrap_or_default()
    }
    pub fn id(&self) -> String {
        proto::device_id(&self.public())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub public: String,
    pub beacon_key: String,
    /// Files from this device: "ask" (the default), "always" or "never".
    #[serde(default = "ask")]
    pub files: String,
    #[serde(default)]
    pub paired_at: u64,
}

fn ask() -> String {
    "ask".into()
}

impl Peer {
    pub fn public(&self) -> Vec<u8> {
        proto::unhex(&self.public).unwrap_or_default()
    }
    pub fn beacon_key(&self) -> Vec<u8> {
        proto::unhex(&self.beacon_key).unwrap_or_default()
    }
}

pub struct Store {
    pub dir: PathBuf,
}

/// ~/.local/share/nexa-link (NEXA_LINK_HOME overrides it, for tests).
pub fn default_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("NEXA_LINK_HOME") {
        return PathBuf::from(d);
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join(".local/share")
    });
    data.join("nexa-link")
}

fn hostname() -> String {
    fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "NexaOS".into())
}

fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    io::Write::write_all(&mut f, data)?;
    f.sync_all()?;
    fs::rename(tmp, path)
}

impl Store {
    pub fn open(dir: PathBuf) -> io::Result<Store> {
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        Ok(Store { dir })
    }

    /// This device's key pair, made on first use.
    pub fn identity(&self) -> io::Result<Identity> {
        let path = self.dir.join("identity.json");
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(id) = serde_json::from_str::<Identity>(&text) {
                return Ok(id);
            }
        }
        let kp = snow::Builder::new(proto::NOISE.parse().expect("valid pattern"))
            .generate_keypair()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        let id = Identity { name: hostname(), secret: proto::hex(&kp.private), public: proto::hex(&kp.public) };
        write_private(&path, serde_json::to_string_pretty(&id)?.as_bytes())?;
        Ok(id)
    }

    pub fn set_name(&self, name: &str) -> io::Result<()> {
        let mut id = self.identity()?;
        id.name = name.chars().filter(|c| !c.is_control()).take(40).collect();
        write_private(&self.dir.join("identity.json"), serde_json::to_string_pretty(&id)?.as_bytes())
    }

    pub fn peers(&self) -> Vec<Peer> {
        fs::read_to_string(self.dir.join("paired.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save_peers(&self, peers: &[Peer]) -> io::Result<()> {
        write_private(&self.dir.join("paired.json"), serde_json::to_string_pretty(peers)?.as_bytes())
    }

    /// Pairing again with the same device replaces its old entry.
    pub fn add_peer(&self, peer: Peer) -> io::Result<()> {
        let mut peers: Vec<Peer> = self.peers().into_iter().filter(|p| p.id != peer.id).collect();
        peers.push(peer);
        self.save_peers(&peers)
    }

    /// "Forget this device": its key is no longer accepted, at once.
    pub fn forget(&self, id: &str) -> io::Result<bool> {
        let peers = self.peers();
        let left: Vec<Peer> = peers.iter().filter(|p| p.id != id).cloned().collect();
        let gone = left.len() != peers.len();
        self.save_peers(&left)?;
        Ok(gone)
    }

    pub fn set_files(&self, id: &str, policy: &str) -> io::Result<bool> {
        if !["ask", "always", "never"].contains(&policy) {
            return Ok(false);
        }
        let mut peers = self.peers();
        let mut found = false;
        for p in peers.iter_mut().filter(|p| p.id == id) {
            p.files = policy.to_string();
            found = true;
        }
        self.save_peers(&peers)?;
        Ok(found)
    }

    pub fn paired_mtime(&self) -> Option<std::time::SystemTime> {
        fs::metadata(self.dir.join("paired.json")).and_then(|m| m.modified()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_made_once_and_kept_private() {
        let dir = std::env::temp_dir().join(format!("nexa-link-store-{}", std::process::id()));
        let s = Store::open(dir.clone()).unwrap();
        let a = s.identity().unwrap();
        let b = s.identity().unwrap();
        assert_eq!(a.public, b.public, "the same key every time");
        assert_eq!(a.secret().len(), 32);
        let mode = fs::metadata(dir.join("identity.json")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let peer = Peer { id: "abcd".into(), name: "Phone".into(), public: "00".into(), beacon_key: "11".into(), files: ask(), paired_at: 1 };
        s.add_peer(peer.clone()).unwrap();
        s.add_peer(Peer { name: "Phone 2".into(), ..peer.clone() }).unwrap();
        assert_eq!(s.peers().len(), 1, "pairing again replaces");
        assert!(s.set_files("abcd", "always").unwrap());
        assert!(!s.set_files("abcd", "sometimes").unwrap());
        assert_eq!(s.peers()[0].files, "always");
        assert!(s.forget("abcd").unwrap());
        assert!(s.peers().is_empty());
        fs::remove_dir_all(dir).ok();
    }
}
