//! Ed25519 publisher keys and package signatures.
//!
//! Key files are text:
//!   secret: "nexa-secret-key v1\n<64 hex>\n"        (mode 0600)
//!   public: "nexa-public-key v1 <key id>\n<64 hex>\n"
//! The key id is the first 16 hex digits of SHA-256(public key). The
//! SIGNATURE file in a package is "ed25519 <key id> <128 hex>\n", a
//! signature over the exact bytes of manifest.json (which in turn pins
//! every file's SHA-256).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

pub fn key_id(public: &VerifyingKey) -> String {
    sha256_hex(public.as_bytes())[..16].to_string()
}

pub fn generate() -> std::io::Result<SigningKey> {
    let mut seed = [0u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut seed)?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn secret_text(key: &SigningKey) -> String {
    format!("nexa-secret-key v1\n{}\n", hex(&key.to_bytes()))
}

pub fn public_text(key: &VerifyingKey) -> String {
    format!("nexa-public-key v1 {}\n{}\n", key_id(key), hex(key.as_bytes()))
}

fn body(text: &str, header: &str) -> Result<[u8; 32], String> {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    if !first.starts_with(header) {
        return Err(format!("not a {header} file"));
    }
    let bytes = unhex(lines.next().unwrap_or("")).ok_or("bad hex in key file")?;
    bytes.try_into().map_err(|_| "key must be 32 bytes".to_string())
}

pub fn read_secret(path: &Path) -> Result<SigningKey, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(SigningKey::from_bytes(&body(&text, "nexa-secret-key v1")?))
}

pub fn parse_public(text: &str) -> Result<VerifyingKey, String> {
    VerifyingKey::from_bytes(&body(text, "nexa-public-key v1")?).map_err(|e| format!("bad public key: {e}"))
}

pub fn signature_text(key: &SigningKey, manifest: &[u8]) -> String {
    format!("ed25519 {} {}\n", key_id(&key.verifying_key()), hex(&key.sign(manifest).to_bytes()))
}

/// (key id, signature) from a SIGNATURE file.
pub fn parse_signature(text: &str) -> Result<(String, Signature), String> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "ed25519" || parts[1].len() != 16 {
        return Err("malformed SIGNATURE".into());
    }
    let raw: [u8; 64] = unhex(parts[2]).and_then(|b| b.try_into().ok()).ok_or("malformed signature bytes")?;
    Ok((parts[1].to_string(), Signature::from_bytes(&raw)))
}

/// Find a trusted key by id in a directory of <id>.pub files.
pub fn trusted_key(dir: &Path, id: &str) -> Option<VerifyingKey> {
    if !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let key = parse_public(&fs::read_to_string(dir.join(format!("{id}.pub"))).ok()?).ok()?;
    (key_id(&key) == id).then_some(key)
}

pub fn verify(key: &VerifyingKey, manifest: &[u8], sig: &Signature) -> bool {
    key.verify(manifest, sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(unhex(&hex(&[0, 1, 254, 255])).unwrap(), vec![0, 1, 254, 255]);
        assert!(unhex("abc").is_none() && unhex("zz").is_none());
    }

    #[test]
    fn sign_and_verify() {
        let key = generate().unwrap();
        let text = signature_text(&key, b"manifest");
        let (id, sig) = parse_signature(&text).unwrap();
        assert_eq!(id, key_id(&key.verifying_key()));
        assert!(verify(&key.verifying_key(), b"manifest", &sig));
        assert!(!verify(&key.verifying_key(), b"manifest!", &sig), "changed manifest must fail");
        let other = generate().unwrap();
        assert!(!verify(&other.verifying_key(), b"manifest", &sig), "other key must fail");
    }

    #[test]
    fn key_files_round_trip() {
        let key = generate().unwrap();
        let back = SigningKey::from_bytes(&body(&secret_text(&key), "nexa-secret-key v1").unwrap());
        assert_eq!(back.to_bytes(), key.to_bytes());
        assert_eq!(parse_public(&public_text(&key.verifying_key())).unwrap(), key.verifying_key());
        assert!(parse_public(&secret_text(&key)).is_err(), "a secret key is not a public key");
    }

    #[test]
    fn trust_dir_lookup() {
        let dir = std::env::temp_dir().join(format!("nexa-keys-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let key = generate().unwrap();
        let id = key_id(&key.verifying_key());
        fs::write(dir.join(format!("{id}.pub")), public_text(&key.verifying_key())).unwrap();
        assert!(trusted_key(&dir, &id).is_some());
        assert!(trusted_key(&dir, "../../etc/passwd").is_none());
        // a key file renamed to another id isn't accepted under that id
        let other = generate().unwrap();
        let other_id = key_id(&other.verifying_key());
        fs::write(dir.join(format!("{other_id}.pub")), public_text(&key.verifying_key())).unwrap();
        assert!(trusted_key(&dir, &other_id).is_none());
        fs::remove_dir_all(&dir).ok();
    }
}
