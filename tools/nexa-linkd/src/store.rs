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
    /// Where it was last heard (IPv4), "" if never: signals also go there
    /// directly, for Wi-Fi networks that hold back broadcasts.
    #[serde(default)]
    pub addr: String,
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

/// Every NexaOS computer is called "nexa" until someone renames it, so a
/// device's name comes from, in order: a hostname someone chose, the
/// computer's model ("HP EliteBook 840 G6"), or "NexaOS PC" with the start
/// of its id, so two machines never show up with the same name.
pub fn default_name(public_hex: &str) -> String {
    let host = hostname();
    if host != "nexa" && host != "NexaOS" {
        return host;
    }
    let dmi = |f: &str| fs::read_to_string(format!("/sys/class/dmi/id/{f}")).map(|s| s.trim().to_string()).unwrap_or_default();
    let model = dmi("product_name");
    // Virtual machines and unbranded boards name nothing useful.
    let vendor = dmi("sys_vendor");
    let virtual_machine = ["QEMU", "innotek", "VMware", "Xen", "Microsoft Corporation", "Red Hat"].iter().any(|v| vendor.starts_with(v))
        || model.contains("i440FX") || model.contains("Q35");
    let generic = virtual_machine || model.is_empty() || model.len() > 40
        || ["Standard PC", "To Be Filled", "System Product Name", "Default string", "Not Specified"].iter().any(|g| model.starts_with(g));
    let short = proto::unhex(public_hex).map(|k| proto::device_id(&k)[..4].to_string()).unwrap_or_default();
    if generic { format!("NexaOS PC ({short})") } else { model }
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
            if let Ok(mut id) = serde_json::from_str::<Identity>(&text) {
                // Made before default_name: still the stock "nexa".
                if id.name == "nexa" || id.name == "NexaOS" {
                    id.name = default_name(&id.public);
                    write_private(&path, serde_json::to_string_pretty(&id)?.as_bytes())?;
                }
                return Ok(id);
            }
        }
        let kp = snow::Builder::new(proto::NOISE.parse().expect("valid pattern"))
            .generate_keypair()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        let public = proto::hex(&kp.public);
        let id = Identity { name: default_name(&public), secret: proto::hex(&kp.private), public };
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

    /// Remember where a paired device was last heard. True if it changed.
    pub fn set_addr(&self, id: &str, addr: &str) -> io::Result<bool> {
        let mut peers = self.peers();
        let Some(p) = peers.iter_mut().find(|p| p.id == id) else { return Ok(false) };
        if p.addr == addr {
            return Ok(false);
        }
        p.addr = addr.to_string();
        self.save_peers(&peers)?;
        Ok(true)
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
        let peer = Peer { id: "abcd".into(), name: "Phone".into(), public: "00".into(), beacon_key: "11".into(), files: ask(), paired_at: 1,
                          addr: String::new() };
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

    #[test]
    fn remembers_where_a_device_was_heard() {
        let dir = std::env::temp_dir().join(format!("nexa-link-addr-{}", std::process::id()));
        let s = Store::open(dir.clone()).unwrap();
        s.add_peer(Peer { id: "p1".into(), name: "Laptop".into(), public: "00".into(), beacon_key: "11".into(), files: ask(),
                          paired_at: 1, addr: "192.168.1.7".into() }).unwrap();
        assert!(!s.set_addr("p1", "192.168.1.7").unwrap(), "same address: nothing written");
        assert!(s.set_addr("p1", "192.168.1.9").unwrap());
        assert_eq!(s.peers()[0].addr, "192.168.1.9");
        assert!(!s.set_addr("nobody", "1.2.3.4").unwrap());
        // Files from before this field still load.
        fs::write(dir.join("paired.json"), r#"[{"id":"old","name":"Old","public":"00","beacon_key":"11"}]"#).unwrap();
        assert_eq!(s.peers()[0].addr, "");
        fs::remove_dir_all(&dir).ok();
    }
}
