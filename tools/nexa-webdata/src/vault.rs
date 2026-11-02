//! Saved passwords, encrypted. passwords.vault is
//!   "NXV1" | mode (1 byte) | salt (16) | nonce (12) | ChaCha20-Poly1305(JSON)
//! with the header as associated data, so it can't be changed unnoticed.
//! The key is either
//! - mode 0: 32 random bytes in vault.key (0600) next to it: a copy of the
//!   vault alone (a backup, a synced folder) can't be read; or
//! - mode 1: PBKDF2-HMAC-SHA256 of a primary password the person chooses:
//!   nothing on the disk opens it without that password.
//! Every save uses a fresh nonce and replaces the file atomically (0600).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"NXV1";
const HEADER: usize = 4 + 1 + 16 + 12;
pub const ITERATIONS: u32 = 210_000;

#[derive(Debug)]
pub struct Fail {
    pub error: String,
    pub code: &'static str,
}

fn fail(code: &'static str, error: &str) -> Fail {
    Fail { error: error.into(), code }
}

pub fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mac = <Hmac<Sha256> as Mac>::new_from_slice(password).expect("any key length works");
    let mut u = mac.clone().chain_update(salt).chain_update(1u32.to_be_bytes()).finalize().into_bytes();
    let mut out: [u8; 32] = u.into();
    for _ in 1..iterations {
        u = mac.clone().chain_update(u).finalize().into_bytes();
        for (o, x) in out.iter_mut().zip(u.iter()) {
            *o ^= x;
        }
    }
    out
}

fn random<const N: usize>() -> Result<[u8; N], Fail> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|_| fail("random", "No random numbers available."))?;
    Ok(b)
}

pub struct Vault {
    pub dir: PathBuf,
    pub iterations: u32,
}

impl Vault {
    fn file(&self) -> PathBuf { self.dir.join("passwords.vault") }
    fn keyfile(&self) -> PathBuf { self.dir.join("vault.key") }

    fn write_private(&self, path: &Path, data: &[u8]) -> Result<(), Fail> {
        fs::create_dir_all(&self.dir).map_err(|e| fail("io", &format!("can't make {}: {e}", self.dir.display())))?;
        let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700));
        let tmp = path.with_extension("tmp");
        let w = (|| -> std::io::Result<()> {
            let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
            fs::rename(&tmp, path)
        })();
        w.map_err(|e| fail("io", &format!("The saved passwords couldn't be written: {e}")))
    }

    pub fn status(&self) -> Value {
        let head = fs::read(self.file()).ok();
        json!({"exists": head.is_some(), "primary": head.as_ref().map_or(false, |h| h.get(4) == Some(&1))})
    }

    fn key(&self, mode: u8, salt: &[u8], primary: Option<&str>) -> Result<[u8; 32], Fail> {
        if mode == 1 {
            let p = primary.filter(|p| !p.is_empty()).ok_or(fail("locked", "Type your primary password to use your saved passwords."))?;
            Ok(pbkdf2(p.as_bytes(), salt, self.iterations))
        } else {
            let k = fs::read(self.keyfile()).map_err(|_| fail("nokey", "The key for your saved passwords is missing, so they can't be read."))?;
            k.try_into().map_err(|_| fail("nokey", "The key for your saved passwords is damaged, so they can't be read."))
        }
    }

    /// The vault's contents, or an empty one if there's no vault yet.
    pub fn open(&self, primary: Option<&str>) -> Result<Value, Fail> {
        let Ok(b) = fs::read(self.file()) else { return Ok(json!({"entries": [], "never": []})) };
        if b.len() < HEADER + 16 || &b[..4] != MAGIC || b[4] > 1 {
            return Err(fail("damaged", "The saved passwords file is damaged."));
        }
        let key = self.key(b[4], &b[5..21], primary)?;
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        let plain = cipher.decrypt(Nonce::from_slice(&b[21..33]), Payload { msg: &b[HEADER..], aad: &b[..HEADER] }).map_err(|_| {
            if b[4] == 1 { fail("locked", "That primary password isn't right.") } else { fail("damaged", "The saved passwords can't be opened: the file or its key is damaged.") }
        })?;
        serde_json::from_slice(&plain).map_err(|_| fail("damaged", "The saved passwords file is damaged."))
    }

    /// Save the contents with `primary` (Some: mode 1) or the key file (None).
    pub fn seal(&self, data: &Value, primary: Option<&str>) -> Result<(), Fail> {
        let salt: [u8; 16] = random()?;
        let nonce: [u8; 12] = random()?;
        let (mode, key) = match primary.filter(|p| !p.is_empty()) {
            Some(p) => (1u8, pbkdf2(p.as_bytes(), &salt, self.iterations)),
            None => {
                let key = match fs::read(self.keyfile()).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()) {
                    Some(k) => k,
                    None => {
                        let k: [u8; 32] = random()?;
                        self.write_private(&self.keyfile(), &k)?;
                        k
                    }
                };
                (0u8, key)
            }
        };
        let mut head = MAGIC.to_vec();
        head.push(mode);
        head.extend(salt);
        head.extend(nonce);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        let body = cipher.encrypt(Nonce::from_slice(&nonce), Payload { msg: data.to_string().as_bytes(), aad: &head })
            .map_err(|_| fail("crypto", "The passwords couldn't be encrypted."))?;
        self.write_private(&self.file(), &[head, body].concat())
    }

    /// The primary password currently needed (None in mode 0).
    fn mode_primary<'a>(&self, primary: Option<&'a str>) -> Option<&'a str> {
        if self.status()["primary"].as_bool() == Some(true) { primary } else { None }
    }

    pub fn save(&self, primary: Option<&str>, origin: &str, username: &str, password: &str, now: u64) -> Result<Value, Fail> {
        let mut v = self.open(primary)?;
        let entries = v["entries"].as_array_mut().ok_or(fail("damaged", "damaged"))?;
        let found = entries.iter_mut().find(|e| e["origin"] == origin && e["username"] == username);
        let (id, changed) = match found {
            Some(e) => {
                let changed = e["password"] != password;
                e["password"] = json!(password);
                e["updated"] = json!(now);
                (e["id"].clone(), changed)
            }
            None => {
                let id: [u8; 8] = random()?;
                let id = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
                entries.push(json!({"id": id, "origin": origin, "username": username, "password": password, "created": now, "updated": now}));
                (json!(id), true)
            }
        };
        if changed {
            self.seal(&v, self.mode_primary(primary))?;
        }
        Ok(json!({"id": id, "changed": changed}))
    }

    pub fn delete(&self, primary: Option<&str>, id: &str) -> Result<Value, Fail> {
        let mut v = self.open(primary)?;
        let entries = v["entries"].as_array_mut().ok_or(fail("damaged", "damaged"))?;
        let n = entries.len();
        entries.retain(|e| e["id"] != id);
        let gone = entries.len() != n;
        if gone {
            self.seal(&v, self.mode_primary(primary))?;
        }
        Ok(json!({"deleted": gone}))
    }

    pub fn never(&self, primary: Option<&str>, origin: &str, add: bool) -> Result<Value, Fail> {
        let mut v = self.open(primary)?;
        let list = v["never"].as_array_mut().ok_or(fail("damaged", "damaged"))?;
        list.retain(|o| o != origin);
        if add {
            list.push(json!(origin));
        }
        self.seal(&v, self.mode_primary(primary))?;
        Ok(json!({"never": add}))
    }

    /// Set, change (new = Some) or remove (new = None) the primary password.
    pub fn set_primary(&self, current: Option<&str>, new: Option<&str>) -> Result<Value, Fail> {
        let v = self.open(current)?;
        self.seal(&v, new)?;
        if new.is_some() {
            // The random key isn't needed any more: the password is the key.
            let _ = fs::remove_file(self.keyfile());
        }
        Ok(self.status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_known_answer() {
        // RFC 7914 section 11, PBKDF2-HMAC-SHA256 ("passwd", "salt", 1), first bytes.
        let k = pbkdf2(b"passwd", b"salt", 1);
        assert_eq!(k[..8], [0x55, 0xac, 0x04, 0x6e, 0x56, 0xe3, 0x08, 0x9f]);
    }

    #[test]
    fn keyfile_and_primary() {
        let dir = std::env::temp_dir().join(format!("nexa-vault-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let v = Vault { dir: dir.clone(), iterations: 10 };
        assert_eq!(v.status()["exists"], false);
        v.save(None, "https://mail.example", "ama", "s3cret!", 1).unwrap();
        assert_eq!(v.save(None, "https://mail.example", "ama", "s3cret!", 2).unwrap()["changed"], false);
        v.save(None, "https://mail.example", "kofi", "other", 3).unwrap();
        let raw = fs::read(dir.join("passwords.vault")).unwrap();
        assert!(!raw.windows(7).any(|w| w == b"s3cret!") && !raw.windows(3).any(|w| w == b"ama"));
        assert_eq!(fs::metadata(dir.join("passwords.vault")).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(v.open(None).unwrap()["entries"].as_array().unwrap().len(), 2);
        // A changed byte anywhere: refused.
        let mut bad = raw.clone();
        bad[10] ^= 1;
        fs::write(dir.join("passwords.vault"), &bad).unwrap();
        assert_eq!(v.open(None).err().unwrap().code, "damaged");
        fs::write(dir.join("passwords.vault"), &raw).unwrap();
        // A primary password: the key file goes, the password is needed.
        v.set_primary(None, Some("hexagon")).unwrap();
        assert!(!dir.join("vault.key").exists());
        assert_eq!(v.status()["primary"], true);
        assert_eq!(v.open(None).err().unwrap().code, "locked");
        assert_eq!(v.open(Some("wrong")).err().unwrap().error, "That primary password isn't right.");
        let id = v.open(Some("hexagon")).unwrap()["entries"][0]["id"].as_str().unwrap().to_string();
        v.delete(Some("hexagon"), &id).unwrap();
        v.never(Some("hexagon"), "https://bank.example", true).unwrap();
        let open = v.open(Some("hexagon")).unwrap();
        assert_eq!((open["entries"].as_array().unwrap().len(), open["never"][0].as_str()), (1, Some("https://bank.example")));
        // Removing it: back to a key file.
        v.set_primary(Some("hexagon"), None).unwrap();
        assert!(dir.join("vault.key").exists() && v.open(None).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }
}
