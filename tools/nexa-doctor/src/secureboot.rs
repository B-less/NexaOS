//! The PC's Secure Boot keys, read from the firmware (efivarfs), read-only.
//!
//! Why: a PC whose keys were cleared ("Clear All Secure Boot Keys", "Reset
//! to Setup Mode") no longer trusts Microsoft's Windows key, and Windows
//! Boot Manager then refuses to start (the firmware menu just blinks back).
//! NexaOS never writes these keys; it only finds the problem and says how
//! to fix it in the BIOS (restore the factory keys).
//!
//! Variables (each file: 4 attribute bytes, then the value):
//!   SecureBoot, SetupMode, PK, KEK   global (8be4df61-...)
//!   db, dbx                          image security (d719b2cb-...)
//!   Boot####                         boot entries, to find Windows

use serde::Serialize;
use std::fs;
use std::path::Path;

const GLOBAL: &str = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
const IMAGE_SECURITY: &str = "d719b2cb-3d3a-4596-a3bc-dad00e67656f";
/// EFI_CERT_X509_GUID a5c059a1-94e4-4aa7-87b5-ab155c2bf072, as stored (mixed endian).
const X509: [u8; 16] = [0xa1, 0x59, 0xc0, 0xa5, 0xe4, 0x94, 0xa7, 0x4a, 0x87, 0xb5, 0xab, 0x15, 0x5c, 0x2b, 0xf0, 0x72];

/// Microsoft's certificates, by the subject name each one carries.
pub const WINDOWS_CAS: [&str; 2] = ["Microsoft Windows Production PCA 2011", "Windows UEFI CA 2023"];
pub const THIRD_PARTY_CAS: [&str; 2] = ["Microsoft Corporation UEFI CA 2011", "Microsoft UEFI CA 2023"];

#[derive(Debug, Serialize, PartialEq, Default)]
pub struct Keys {
    /// false: the PC started in legacy BIOS mode (no UEFI variables).
    pub uefi: bool,
    pub secure_boot: Option<bool>,
    /// true: no platform key, i.e. the keys were cleared.
    pub setup_mode: Option<bool>,
    pub pk: Vec<String>,
    pub kek: Vec<String>,
    /// Certificates the firmware trusts to start programs.
    pub db: Vec<String>,
    /// Plain file hashes in db.
    pub db_hashes: usize,
    /// Entries in the forbidden list (revoked programs).
    pub dbx_entries: usize,
    /// Boot entries that start Windows Boot Manager.
    pub windows_entries: Vec<String>,
    /// db trusts Windows' own boot manager.
    pub trusts_windows: bool,
    /// db trusts Windows' 2023 CA (new boot managers are signed with it).
    pub trusts_windows_2023: bool,
    /// db trusts Microsoft's third-party CA (shim, so the NexaOS USB).
    pub trusts_third_party: bool,
    /// What is wrong, worst first: (id, title, what to do).
    pub problems: Vec<Problem>,
}

#[derive(Debug, Serialize, PartialEq, Clone)]
pub struct Problem {
    pub id: String,
    pub title: String,
    pub detail: String,
}

fn var(dir: &Path, name: &str, guid: &str) -> Option<Vec<u8>> {
    let b = fs::read(dir.join(format!("{name}-{guid}"))).ok()?;
    (b.len() >= 4).then(|| b[4..].to_vec())
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// One DER element at `at`: (tag, start of content, content length).
fn der(b: &[u8], at: usize) -> Option<(u8, usize, usize)> {
    let tag = *b.get(at)?;
    let first = *b.get(at + 1)? as usize;
    if first < 0x80 {
        return Some((tag, at + 2, first));
    }
    let n = first & 0x7f;
    if n == 0 || n > 4 {
        return None;
    }
    let mut len = 0usize;
    for i in 0..n {
        len = (len << 8) | *b.get(at + 2 + i)? as usize;
    }
    Some((tag, at + 2 + n, len))
}

/// The subject's common name (or organisation) of a DER X.509 certificate.
/// The issuer comes before the subject, so the last name found in the
/// first two name blocks is the subject's.
pub fn subject_name(cert: &[u8]) -> Option<String> {
    // Certificate SEQUENCE { tbsCertificate SEQUENCE { [0] version?, serial, sigAlg, issuer, validity, subject, ... } }
    let (_, c, _) = der(cert, 0)?;
    let (_, tbs, tbs_len) = der(cert, c)?;
    let end = (tbs + tbs_len).min(cert.len());
    let mut at = tbs;
    let mut fields = Vec::new();
    while at < end && fields.len() < 7 {
        let (tag, start, len) = der(cert, at)?;
        fields.push((tag, start, len));
        at = start + len;
    }
    // Skip the optional [0] version.
    let f: Vec<_> = if fields.first().map(|f| f.0) == Some(0xa0) { fields[1..].to_vec() } else { fields };
    let (_, s, l) = *f.get(4)?; // serial, sigAlg, issuer, validity, subject
    name_in(cert.get(s..s + l)?)
}

/// CN (2.5.4.3) from a Name, else O (2.5.4.10).
fn name_in(name: &[u8]) -> Option<String> {
    let find = |oid: &[u8]| -> Option<String> {
        let pos = name.windows(oid.len()).position(|w| w == oid)?;
        let (tag, start, len) = der(name, pos + oid.len())?;
        let raw = name.get(start..start + len)?;
        Some(if tag == 0x1e {
            // BMPString: UTF-16 big endian.
            String::from_utf16_lossy(&raw.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>())
        } else {
            String::from_utf8_lossy(raw).into_owned()
        })
    };
    find(&[0x06, 0x03, 0x55, 0x04, 0x03]).or_else(|| find(&[0x06, 0x03, 0x55, 0x04, 0x0a]))
}

/// Walk EFI_SIGNATURE_LISTs: (certificate names, other entries).
pub fn signature_lists(b: &[u8]) -> (Vec<String>, usize) {
    let (mut names, mut others) = (Vec::new(), 0);
    let mut at = 0;
    while at + 28 <= b.len() {
        let (Some(list), Some(header), Some(size)) = (u32le(b, at + 16), u32le(b, at + 20), u32le(b, at + 24)) else { break };
        let (list, header, size) = (list as usize, header as usize, size as usize);
        if list < 28 || at + list > b.len() || size <= 16 {
            break;
        }
        let x509 = b[at..at + 16] == X509;
        let mut e = at + 28 + header;
        while e + size <= at + list {
            let data = &b[e + 16..e + size]; // after the owner GUID
            if x509 {
                names.push(subject_name(data).unwrap_or_else(|| "(unreadable certificate)".into()));
            } else {
                others += 1;
            }
            e += size;
        }
        at += list;
    }
    (names, others)
}

/// The description of a Boot#### entry if it starts Windows Boot Manager.
pub fn windows_entry(opt: &[u8]) -> Option<String> {
    // EFI_LOAD_OPTION: attributes u32, path length u16, description (UTF-16, NUL), device path.
    let units: Vec<u16> = opt.get(6..)?.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let desc_end = units.iter().position(|&u| u == 0)?;
    let desc = String::from_utf16_lossy(&units[..desc_end]);
    // The device path's file name is UTF-16 too, and every node before it
    // has an even length, so it lines up with the description's units.
    let rest = String::from_utf16_lossy(&units[desc_end..]).to_ascii_lowercase();
    (rest.contains("bootmgfw.efi") || desc.to_ascii_lowercase().contains("windows boot manager")).then_some(desc)
}

fn has(list: &[String], names: &[&str]) -> bool {
    list.iter().any(|c| names.iter().any(|n| c.contains(n)))
}

/// Read the keys from `<sys>/firmware/efi/efivars`.
pub fn read(sys: &Path) -> Keys {
    let dir = sys.join("firmware/efi/efivars");
    if !sys.join("firmware/efi").exists() {
        return Keys::default();
    }
    let flag = |name: &str| var(&dir, name, GLOBAL).and_then(|v| v.first().copied()).map(|b| b == 1);
    let certs = |name: &str, guid: &str| var(&dir, name, guid).map(|v| signature_lists(&v)).unwrap_or_default();
    let (pk, _) = certs("PK", GLOBAL);
    let (kek, _) = certs("KEK", GLOBAL);
    let (db, db_hashes) = certs("db", IMAGE_SECURITY);
    let (dbx_certs, dbx_hashes) = certs("dbx", IMAGE_SECURITY);
    let mut windows_entries: Vec<String> = fs::read_dir(&dir).into_iter().flatten().flatten().filter_map(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        let num = n.strip_prefix("Boot")?.strip_suffix(&format!("-{GLOBAL}"))?;
        (num.len() == 4 && num.chars().all(|c| c.is_ascii_hexdigit())).then_some(())?;
        windows_entry(fs::read(e.path()).ok()?.get(4..)?)
    }).collect();
    windows_entries.sort();
    windows_entries.dedup();
    let setup_mode = flag("SetupMode").or_else(|| dir.exists().then_some(pk.is_empty()));
    let mut k = Keys {
        uefi: true,
        secure_boot: flag("SecureBoot"),
        setup_mode,
        trusts_windows: has(&db, &WINDOWS_CAS),
        trusts_windows_2023: has(&db, &WINDOWS_CAS[1..]),
        trusts_third_party: has(&db, &THIRD_PARTY_CAS),
        pk, kek, db, db_hashes,
        dbx_entries: dbx_certs.len() + dbx_hashes,
        windows_entries,
        problems: vec![],
    };
    k.problems = problems(&k);
    k
}

const RESTORE: &str = "Restart, open the BIOS setup (ThinkPad: F1; Dell: F2; HP: F10), go to Security > Secure Boot \
and choose \"Restore Factory Keys\" (also called \"Reset to Default Keys\" or \"Install Default Keys\"), then save and exit. \
Never use \"Clear All Secure Boot Keys\" or \"Reset to Setup Mode\". Have your BitLocker recovery key ready first \
(aka.ms/myrecoverykey): Windows may ask for it once.";

fn problems(k: &Keys) -> Vec<Problem> {
    let mut out = Vec::new();
    if !k.uefi {
        return out;
    }
    let windows = !k.windows_entries.is_empty();
    if k.setup_mode == Some(true) {
        out.push(Problem {
            id: "secureboot-cleared".into(),
            title: "This PC's Secure Boot keys have been cleared".into(),
            detail: format!("{}Secure Boot can't protect this PC until its factory keys are back. {RESTORE}",
                if windows { "Windows may not start (the boot menu just returns). " } else { "" }),
        });
    } else if windows && !k.trusts_windows {
        out.push(Problem {
            id: "secureboot-no-windows-key".into(),
            title: "This PC no longer trusts Windows' boot key".into(),
            detail: format!("With Secure Boot on, Windows Boot Manager is refused. {RESTORE}"),
        });
    }
    out
}

/// Plain text for a person (nexa-doctor firmware-keys --text).
pub fn text(k: &Keys) -> String {
    if !k.uefi {
        return "This PC started in legacy BIOS mode: it has no Secure Boot keys to check.\n".into();
    }
    let yes = |b: bool| if b { "yes" } else { "NO" };
    let onoff = |b: Option<bool>| match b { Some(true) => "on", Some(false) => "off", None => "off (the firmware doesn't report it)" };
    let mut s = String::new();
    s += &format!("Secure Boot: {}\n", onoff(k.secure_boot));
    s += &format!("Keys: {}\n", match k.setup_mode {
        Some(true) => "CLEARED (setup mode: no platform key)",
        Some(false) => "installed (user mode)",
        None => "unknown",
    });
    s += &format!("Platform key: {}\n", if k.pk.is_empty() { "none".into() } else { k.pk.join(", ") });
    s += &format!("Key exchange keys: {}\n", if k.kek.is_empty() { "none".into() } else { k.kek.join(", ") });
    s += &format!("Trusted (db): {} certificate(s), {} file hash(es)\n", k.db.len(), k.db_hashes);
    for c in &k.db {
        s += &format!("  - {c}\n");
    }
    s += &format!("Forbidden (dbx): {} entries\n", k.dbx_entries);
    s += &format!("Trusts Windows: {}{}\n", yes(k.trusts_windows),
        if k.trusts_windows && !k.trusts_windows_2023 { " (2011 key only; not yet Windows UEFI CA 2023)" } else { "" });
    s += &format!("Trusts Microsoft's third-party CA (the NexaOS USB with Secure Boot on): {}\n", yes(k.trusts_third_party));
    s += &format!("Windows on this PC: {}\n", if k.windows_entries.is_empty() { "no boot entry found".into() } else { k.windows_entries.join(", ") });
    if k.problems.is_empty() {
        s += "\nNo problems found.\n";
    }
    for p in &k.problems {
        s += &format!("\nPROBLEM: {}\n{}\n", p.title, p.detail);
    }
    if !k.windows_entries.is_empty() && k.trusts_windows && !k.trusts_windows_2023 {
        s += "\nNote: if Windows still won't start with Secure Boot on after restoring the factory keys, its boot manager \
may be signed with the newer 2023 key: turn Secure Boot off, start Windows, install all updates (they add the 2023 key), \
then turn Secure Boot on again.\n";
    }
    if !k.trusts_third_party && k.setup_mode == Some(false) {
        s += "\nThe NexaOS USB can only start with Secure Boot on if the PC trusts Microsoft's third-party CA \
(ThinkPad: Security > Secure Boot > \"Allow Microsoft 3rd Party UEFI CA\"; on other PCs it may be called \
\"Third-party UEFI CA\"). Otherwise start it with Secure Boot off.\n";
    }
    s
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A DER TLV.
    fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![tag];
        if body.len() < 0x80 {
            v.push(body.len() as u8);
        } else {
            v.extend([0x82, (body.len() >> 8) as u8, body.len() as u8]);
        }
        v.extend(body);
        v
    }

    fn name(cn: &str) -> Vec<u8> {
        let atv = tlv(0x30, &[&[0x06, 0x03, 0x55, 0x04, 0x03][..], &tlv(0x0c, cn.as_bytes())].concat());
        tlv(0x30, &tlv(0x31, &atv))
    }

    /// A certificate shaped like a real one (no valid signature).
    pub fn cert(issuer: &str, subject: &str) -> Vec<u8> {
        let tbs = [
            tlv(0xa0, &tlv(0x02, &[2])),
            tlv(0x02, &[1, 2, 3, 4, 5, 6, 7, 8]),
            tlv(0x30, &tlv(0x06, &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b])),
            name(issuer),
            tlv(0x30, &[tlv(0x17, b"110101000000Z"), tlv(0x17, b"360101000000Z")].concat()),
            name(subject),
            tlv(0x30, &[0u8; 140]),
        ].concat();
        tlv(0x30, &[tlv(0x30, &tbs), tlv(0x30, &[0x05, 0x00]), tlv(0x03, &[0u8; 64])].concat())
    }

    /// An EFI_SIGNATURE_LIST holding one X.509 certificate.
    pub fn esl(cert: &[u8]) -> Vec<u8> {
        let size = 16 + cert.len();
        let mut v = X509.to_vec();
        v.extend(((28 + size) as u32).to_le_bytes());
        v.extend(0u32.to_le_bytes());
        v.extend((size as u32).to_le_bytes());
        v.extend([0x77u8; 16]); // owner
        v.extend(cert);
        v
    }

    fn sha256_esl(n: usize) -> Vec<u8> {
        let mut v = vec![0x26, 0x16, 0xc4, 0xc1, 0x4c, 0x50, 0x92, 0x40, 0xac, 0xa9, 0x41, 0xf9, 0x36, 0x93, 0x43, 0x28];
        v.extend(((28 + n * 48) as u32).to_le_bytes());
        v.extend(0u32.to_le_bytes());
        v.extend(48u32.to_le_bytes());
        v.extend(vec![0xabu8; n * 48]);
        v
    }

    pub fn load_option(desc: &str, path: &str) -> Vec<u8> {
        let mut v = 1u32.to_le_bytes().to_vec();
        let dp: Vec<u8> = [&[4u8, 4, 0, 0][..], &path.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()).collect::<Vec<_>>()].concat();
        v.extend((dp.len() as u16).to_le_bytes());
        v.extend(desc.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()));
        v.extend(dp);
        v
    }

    fn put(dir: &Path, name: &str, guid: &str, value: &[u8]) {
        fs::write(dir.join(format!("{name}-{guid}")), [&[7u8, 0, 0, 0][..], value].concat()).unwrap();
    }

    #[test]
    fn names_lists_and_entries() {
        let c = cert("Microsoft Root Certificate Authority 2010", "Microsoft Windows Production PCA 2011");
        assert_eq!(subject_name(&c).as_deref(), Some("Microsoft Windows Production PCA 2011"));
        let two = [esl(&c), sha256_esl(3), esl(&cert("Microsoft Corporation Third Party Marketplace Root", "Microsoft Corporation UEFI CA 2011"))].concat();
        let (names, hashes) = signature_lists(&two);
        assert_eq!(names, vec!["Microsoft Windows Production PCA 2011", "Microsoft Corporation UEFI CA 2011"]);
        assert_eq!(hashes, 3);
        assert_eq!(signature_lists(&[0u8; 10]), (vec![], 0), "too short: nothing, no panic");
        let mut broken = esl(&c);
        broken[16] = 0xff; // list size past the end
        assert_eq!(signature_lists(&broken), (vec![], 0));
        assert_eq!(windows_entry(&load_option("Windows Boot Manager", "\\EFI\\Microsoft\\Boot\\bootmgfw.efi")).as_deref(), Some("Windows Boot Manager"));
        assert_eq!(windows_entry(&load_option("NVMe0 TOSHIBA", "\\EFI\\Microsoft\\Boot\\BOOTMGFW.EFI")).as_deref(), Some("NVMe0 TOSHIBA"));
        assert_eq!(windows_entry(&load_option("NexaOS", "\\EFI\\nexaos\\grubx64.efi")), None);
    }

    #[test]
    fn cleared_keys_and_missing_windows_key() {
        let r = std::env::temp_dir().join(format!("nexa-doctor-sb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        assert!(!read(&r).uefi, "legacy BIOS");
        assert!(text(&read(&r)).contains("legacy"));
        let d = r.join("firmware/efi/efivars");
        fs::create_dir_all(&d).unwrap();
        put(&d, "Boot0001", GLOBAL, &load_option("Windows Boot Manager", "\\EFI\\Microsoft\\Boot\\bootmgfw.efi"));
        put(&d, "Boot0002", GLOBAL, &load_option("USB HDD", "\\EFI\\BOOT\\BOOTX64.EFI"));
        // What "Clear All Secure Boot Keys" leaves: setup mode, nothing in db.
        put(&d, "SecureBoot", GLOBAL, &[0]);
        put(&d, "SetupMode", GLOBAL, &[1]);
        let k = read(&r);
        assert_eq!(k.windows_entries, vec!["Windows Boot Manager"]);
        assert_eq!(k.problems.len(), 1);
        assert_eq!(k.problems[0].id, "secureboot-cleared");
        assert!(k.problems[0].detail.contains("Restore Factory Keys") && k.problems[0].detail.contains("Windows may not start"));
        assert!(text(&k).contains("CLEARED"));
        // Factory keys back: no problem; the 2023 note shows.
        put(&d, "SetupMode", GLOBAL, &[0]);
        put(&d, "SecureBoot", GLOBAL, &[1]);
        put(&d, "PK", GLOBAL, &esl(&cert("Lenovo Ltd. PK CA 2012", "Lenovo Ltd. PK CA 2012")));
        put(&d, "KEK", GLOBAL, &esl(&cert("Microsoft Root", "Microsoft Corporation KEK CA 2011")));
        put(&d, "db", IMAGE_SECURITY, &[esl(&cert("Microsoft Root Certificate Authority 2010", "Microsoft Windows Production PCA 2011")),
            esl(&cert("Microsoft Corporation Third Party Marketplace Root", "Microsoft Corporation UEFI CA 2011"))].concat());
        put(&d, "dbx", IMAGE_SECURITY, &sha256_esl(5));
        let k = read(&r);
        assert!(k.problems.is_empty(), "{:?}", k.problems);
        assert!(k.trusts_windows && k.trusts_third_party && !k.trusts_windows_2023);
        assert_eq!(k.dbx_entries, 5);
        assert!(text(&k).contains("2023 key"));
        // Someone's own keys, without Microsoft's Windows key: Windows is refused.
        put(&d, "db", IMAGE_SECURITY, &esl(&cert("NexaOS", "NexaOS Secure Boot Signing Key")));
        let k = read(&r);
        assert_eq!(k.problems[0].id, "secureboot-no-windows-key");
        assert!(text(&k).contains("Allow Microsoft 3rd Party UEFI CA"));
        // No Windows on the PC: own keys are fine.
        fs::remove_file(d.join(format!("Boot0001-{GLOBAL}"))).unwrap();
        assert!(read(&r).problems.is_empty());
        let _ = fs::remove_dir_all(&r);
    }
}
