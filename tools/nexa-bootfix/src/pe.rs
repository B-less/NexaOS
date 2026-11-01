//! Is a .efi file a whole, signed x64 program, and who signed it?
//! (PE/COFF header, the security directory, the certificate names in the
//! Authenticode signature.) It doesn't verify the signature: the firmware
//! does that; this tells which Microsoft key the firmware will be asked to
//! trust.

use serde::Serialize;

#[derive(Debug, Serialize, PartialEq, Default, Clone)]
pub struct Image {
    pub size: u64,
    /// MZ + PE headers present and x64 (or arm64).
    pub valid: bool,
    pub machine: String,
    pub signed: bool,
    /// Every name in the signature's certificates (signer, its CA, the root).
    pub names: Vec<String>,
    /// Why it isn't valid.
    pub problem: Option<String>,
}

fn u16at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// CN (2.5.4.3) values anywhere in DER data, in order, without repeats.
pub fn common_names(der: &[u8]) -> Vec<String> {
    const CN: [u8; 5] = [0x06, 0x03, 0x55, 0x04, 0x03];
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 7 < der.len() {
        if der[i..i + 5] == CN {
            let tag = der[i + 5];
            let len = der[i + 6] as usize;
            if len < 0x80 && matches!(tag, 0x0c | 0x13 | 0x14 | 0x16 | 0x1e) {
                if let Some(raw) = der.get(i + 7..i + 7 + len) {
                    let s = if tag == 0x1e {
                        String::from_utf16_lossy(&raw.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>())
                    } else {
                        String::from_utf8_lossy(raw).into_owned()
                    };
                    if !out.contains(&s) {
                        out.push(s);
                    }
                }
            }
            i += 7;
        } else {
            i += 1;
        }
    }
    out
}

pub fn inspect(b: &[u8]) -> Image {
    let mut img = Image { size: b.len() as u64, ..Default::default() };
    let fail = |mut img: Image, why: &str| {
        img.problem = Some(why.to_string());
        img
    };
    if b.is_empty() {
        return fail(img, "the file is empty (0 bytes)");
    }
    if b.get(..2) != Some(b"MZ") {
        return fail(img, "not a program (no MZ header): the file is damaged");
    }
    let Some(pe) = u32at(b, 0x3c).map(|v| v as usize) else { return fail(img, "cut short") };
    if b.get(pe..pe + 4) != Some(b"PE\0\0") {
        return fail(img, "not a program (no PE header): the file is damaged");
    }
    let machine = u16at(b, pe + 4).unwrap_or(0);
    img.machine = match machine {
        0x8664 => "x64", 0xaa64 => "arm64", 0x14c => "x86 (32-bit)", _ => "unknown",
    }.to_string();
    let opt = pe + 24;
    let magic = u16at(b, opt).unwrap_or(0);
    // The security directory is data directory 4.
    let dirs = match magic {
        0x20b => opt + 112,
        0x10b => opt + 96,
        _ => return fail(img, "unknown program format: the file is damaged"),
    };
    let size_of_image = u32at(b, opt + 56).unwrap_or(0) as u64;
    let headers = u32at(b, opt + 60).unwrap_or(0) as u64;
    if headers > img.size || size_of_image == 0 {
        return fail(img, "the file is cut short");
    }
    // The last section must end inside the file.
    let sections = u16at(b, pe + 6).unwrap_or(0) as usize;
    let sec_table = opt + u16at(b, pe + 20).unwrap_or(0) as usize;
    for s in 0..sections {
        let at = sec_table + s * 40;
        let (raw_size, raw_ptr) = (u32at(b, at + 16).unwrap_or(0) as u64, u32at(b, at + 20).unwrap_or(0) as u64);
        if raw_ptr + raw_size > img.size {
            return fail(img, "the file is cut short (a part of the program is missing)");
        }
    }
    if let (Some(off), Some(len)) = (u32at(b, dirs + 32), u32at(b, dirs + 36)) {
        let (off, len) = (off as usize, len as usize);
        if len > 0 {
            match b.get(off..off + len) {
                Some(sig) => {
                    img.signed = true;
                    img.names = common_names(sig);
                }
                None => return fail(img, "the signature is cut off: the file is damaged"),
            }
        }
    }
    img.valid = machine == 0x8664 || machine == 0xaa64;
    if !img.valid {
        return fail(img, "not a 64-bit PC program");
    }
    img
}

/// Which of Microsoft's boot keys signed it: "2023", "2011", "third-party", or "".
pub fn microsoft_ca(img: &Image) -> &'static str {
    let has = |n: &str| img.names.iter().any(|x| x.contains(n));
    if has("Windows UEFI CA 2023") {
        "2023"
    } else if has("Microsoft Windows Production PCA 2011") {
        "2011"
    } else if has("Microsoft Corporation UEFI CA 2011") || has("Microsoft UEFI CA 2023") {
        "third-party"
    } else {
        ""
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A minimal x64 PE with one section and a "signature" holding these names.
    pub fn fake_pe(names: &[&str]) -> Vec<u8> {
        let mut sig = vec![0x30, 0x82, 0, 0];
        for n in names {
            sig.extend([0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, n.len() as u8]);
            sig.extend(n.as_bytes());
        }
        let mut b = vec![0u8; 0x400];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        b[0x86..0x88].copy_from_slice(&1u16.to_le_bytes()); // sections
        b[0x94..0x96].copy_from_slice(&240u16.to_le_bytes()); // optional header size
        let opt = 0x98;
        b[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        b[opt + 56..opt + 60].copy_from_slice(&0x2000u32.to_le_bytes());
        b[opt + 60..opt + 64].copy_from_slice(&0x200u32.to_le_bytes());
        let sec = opt + 240;
        b[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes());
        b[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes());
        let dirs = opt + 112;
        b[dirs + 32..dirs + 36].copy_from_slice(&0x400u32.to_le_bytes());
        b[dirs + 36..dirs + 40].copy_from_slice(&(sig.len() as u32).to_le_bytes());
        b.extend(sig);
        b
    }

    #[test]
    fn whole_signed_and_broken() {
        let good = fake_pe(&["Microsoft Root Certificate Authority 2010", "Microsoft Windows Production PCA 2011", "Microsoft Windows"]);
        let i = inspect(&good);
        assert!(i.valid && i.signed, "{i:?}");
        assert_eq!(i.machine, "x64");
        assert_eq!(microsoft_ca(&i), "2011");
        assert_eq!(microsoft_ca(&inspect(&fake_pe(&["Windows UEFI CA 2023", "Microsoft Windows"]))), "2023");
        assert_eq!(inspect(&[]).problem.as_deref(), Some("the file is empty (0 bytes)"));
        assert!(inspect(&good[..0x300]).problem.unwrap().contains("cut short"));
        assert!(inspect(&vec![0u8; 4096]).problem.unwrap().contains("damaged"));
        let mut unsigned = good.clone();
        unsigned.truncate(0x400);
        let dirs = 0x98 + 112;
        unsigned[dirs + 32..dirs + 40].copy_from_slice(&[0; 8]);
        let u = inspect(&unsigned);
        assert!(u.valid && !u.signed);
    }
}
