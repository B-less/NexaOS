//! Nexa Link's wire format: the signals ("beacons") that only Nexa devices
//! can read, pairing codes, the safety checks on what arrives, and the
//! frames the encrypted channel is carried in.
//!
//! A beacon is 32 bytes: 16 random bytes, then a 16-byte tag
//! HMAC-SHA256(key, "beacon" | 5-minute window | sender's public key |
//! the random bytes). A paired device holds the key (made at pairing) and
//! recognises it; anything else sees 32 random bytes that change every
//! time - no name, no device id, nothing to follow a device by.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const PORT: u16 = 47420;
pub const PAIR_PORT: u16 = 47421;
pub const NOISE: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
pub const NOISE_PAIR: &str = "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s";
pub const BEACON_LEN: usize = 32;
pub const WINDOW_SECS: u64 = 300;
/// Messages inside the channel: JSON, or a piece of a file.
pub const MSG_JSON: u8 = 1;
pub const MSG_DATA: u8 = 2;
/// Largest piece of a file per message (a Noise message is at most 65535).
pub const CHUNK: usize = 60_000;
/// Open "Nexa Drop" beacons (receiving from any Nexa device for a while)
/// use this public key: any Nexa device can recognise one, anything else
/// sees random bytes. Who it is comes only inside the encrypted channel.
const DROP_KEY: &[u8] = b"nexa-link drop beacon v1";

type HmacSha256 = Hmac<Sha256>;

pub fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC takes any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

fn window(now_secs: u64) -> u64 {
    now_secs / WINDOW_SECS
}

fn tag(key: &[u8], win: u64, sender: &[u8], nonce: &[u8]) -> [u8; 32] {
    hmac(key, &[b"beacon", &win.to_be_bytes(), sender, nonce])
}

/// A beacon for one paired device (or, with `drop_key()`, for anyone).
pub fn beacon(key: &[u8], sender_public: &[u8], now_secs: u64, nonce: &[u8; 16]) -> [u8; BEACON_LEN] {
    let mut out = [0u8; BEACON_LEN];
    out[..16].copy_from_slice(nonce);
    out[16..].copy_from_slice(&tag(key, window(now_secs), sender_public, nonce)[..16]);
    out
}

/// Did `sender_public`, holding `key`, send this? Clocks may differ by a
/// window either way. The comparison takes the same time whatever matches.
pub fn beacon_from(key: &[u8], sender_public: &[u8], packet: &[u8], now_secs: u64) -> bool {
    if packet.len() != BEACON_LEN {
        return false;
    }
    let (nonce, got) = packet.split_at(16);
    let w = window(now_secs);
    [w.wrapping_sub(1), w, w + 1].iter().any(|&win| {
        let want = tag(key, win, sender_public, nonce);
        want[..16].iter().zip(got).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    })
}

pub fn drop_key() -> &'static [u8] {
    DROP_KEY
}

/// Open drop beacons carry no sender (unknown until the channel is up).
pub const NO_SENDER: &[u8] = b"";

/// The key two devices share after pairing, for their beacons.
pub fn beacon_key(handshake_hash: &[u8]) -> [u8; 32] {
    hmac(handshake_hash, &[b"nexa-link beacon key v1"])
}

/// Six digits both screens show while pairing: they match only if both
/// devices took part in the same handshake (no one in between).
pub fn sas(handshake_hash: &[u8]) -> String {
    let h = hmac(handshake_hash, &[b"nexa-link sas v1"]);
    format!("{:06}", u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000)
}

/// A device's id: the first 8 bytes of SHA-256 of its public key, in hex.
pub fn device_id(public: &[u8]) -> String {
    hex(&Sha256::digest(public)[..8])
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// What a QR code (or the typed code) carries: where to connect, and a
/// one-time secret that keys the handshake, so only the device that saw
/// the code can pair.
#[derive(Debug, PartialEq)]
pub struct PairCode {
    pub addr: String,
    pub secret: [u8; 16],
}

impl PairCode {
    pub fn encode(&self) -> String {
        format!("NEXA-LINK:1:{}:{}", self.addr, hex(&self.secret))
    }

    pub fn parse(s: &str) -> Option<PairCode> {
        let mut it = s.trim().splitn(4, ':');
        if it.next()? != "NEXA-LINK" || it.next()? != "1" {
            return None;
        }
        let addr = it.next()?.to_string();
        addr.parse::<std::net::Ipv4Addr>().ok()?;
        let secret: [u8; 16] = unhex(it.next()?)?.try_into().ok()?;
        Some(PairCode { addr, secret })
    }

    pub fn psk(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"nexa-link pair psk v1");
        h.update(self.secret);
        h.finalize().into()
    }
}

/// A file name as sent by another device, made safe to save: no folders,
/// no hidden files, no control characters, not too long.
pub fn safe_name(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() {
        return None;
    }
    Some(clean.chars().take(200).collect())
}

/// Never overwrite: "photo.jpg", then "photo (2).jpg", "photo (3).jpg"...
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).expect("some name is free")
}

pub fn write_frame(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
    let len = u16::try_from(data.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too big"))?;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(data)?;
    w.flush()
}

pub fn read_frame(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    r.read_exact(&mut len)?;
    let mut buf = vec![0u8; u16::from_be_bytes(len) as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &[u8] = b"public key of the PC.............";
    const PHONE: &[u8] = b"public key of the phone..........";

    #[test]
    fn a_paired_device_recognises_the_beacon_and_nothing_else_does() {
        let key = [7u8; 32];
        let b = beacon(&key, ME, 1_000_000, &[1; 16]);
        assert!(beacon_from(&key, ME, &b, 1_000_000), "the paired device knows it");
        assert!(!beacon_from(&[8u8; 32], ME, &b, 1_000_000), "another key doesn't");
        assert!(!beacon_from(&key, PHONE, &b, 1_000_000), "it isn't mistaken for the other side's own beacon");
        assert!(beacon_from(&key, ME, &b, 1_000_000 + WINDOW_SECS), "a clock one window ahead still works");
        assert!(!beacon_from(&key, ME, &b, 1_000_000 + 3 * WINDOW_SECS), "an old recording is useless");
        assert!(!beacon_from(&key, ME, &b[..31], 1_000_000), "wrong length");
    }

    #[test]
    fn beacons_never_repeat_and_carry_nothing_readable() {
        let key = [7u8; 32];
        let a = beacon(&key, ME, 5_000, &[1; 16]);
        let b = beacon(&key, ME, 5_000, &[2; 16]);
        assert_ne!(a[16..], b[16..], "a new nonce gives a different tag");
        let text = String::from_utf8_lossy(&a);
        assert!(!text.contains("NEXA") && !text.contains("nexa"), "no name or marker inside");
    }

    #[test]
    fn drop_beacons_are_recognised_by_any_nexa_device() {
        let b = beacon(drop_key(), NO_SENDER, 42_000, &[9; 16]);
        assert!(beacon_from(drop_key(), NO_SENDER, &b, 42_000));
        assert!(!beacon_from(&[7u8; 32], ME, &b, 42_000));
    }

    #[test]
    fn pairing_codes_round_trip_and_reject_junk() {
        let c = PairCode { addr: "192.168.1.20".into(), secret: [0xab; 16] };
        assert_eq!(PairCode::parse(&c.encode()), Some(c));
        for bad in ["", "NEXA-LINK:2:1.2.3.4:00", "NEXA-LINK:1:not-an-ip:abababababababababababababababab",
                    "NEXA-LINK:1:1.2.3.4:abab", "http://evil/"] {
            assert_eq!(PairCode::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn sas_is_six_digits_and_follows_the_handshake() {
        assert_eq!(sas(b"one").len(), 6);
        assert_eq!(sas(b"one"), sas(b"one"));
        assert_ne!(sas(b"one"), sas(b"two"));
    }

    #[test]
    fn names_from_other_devices_are_made_safe() {
        assert_eq!(safe_name("photo.jpg").as_deref(), Some("photo.jpg"));
        assert_eq!(safe_name("../../.bashrc").as_deref(), Some("bashrc"));
        assert_eq!(safe_name("/etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(safe_name("C:\\Users\\x\\a.txt").as_deref(), Some("a.txt"));
        assert_eq!(safe_name("..").as_deref(), None);
        assert_eq!(safe_name("bad\u{7}\nname.txt").as_deref(), Some("badname.txt"));
        assert_eq!(safe_name(&"x".repeat(500)).map(|s| s.len()), Some(200));
    }

    #[test]
    fn received_files_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("nexa-link-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (2).txt"));
        std::fs::write(dir.join("a (2).txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (3).txt"));
        std::fs::write(dir.join("README"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "README"), dir.join("README (2)"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"hello").unwrap();
        assert_eq!(read_frame(&mut &buf[..]).unwrap(), b"hello");
        assert!(write_frame(&mut Vec::new(), &vec![0; 70_000]).is_err());
    }
}
