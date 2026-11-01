//! The firmware's boot entries (Boot####, BootOrder) through efivarfs.
//! Only boot entries are ever written here; never Secure Boot keys.

use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;

pub const GLOBAL: &str = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
/// NON_VOLATILE | BOOTSERVICE_ACCESS | RUNTIME_ACCESS
const ATTRS: u32 = 0x7;
pub const WINDOWS_PATH: &str = "\\EFI\\Microsoft\\Boot\\bootmgfw.efi";

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Entry {
    /// e.g. "0001"
    pub id: String,
    pub description: String,
    pub active: bool,
    /// The partition it points at (GPT unique GUID), if any.
    pub partition_guid: Option<String>,
    pub partition_number: Option<u32>,
    /// The file it starts, e.g. \EFI\Microsoft\Boot\bootmgfw.efi
    pub file: Option<String>,
    pub in_boot_order: bool,
}

impl Entry {
    pub fn is_windows(&self) -> bool {
        self.file.as_deref().map_or(false, |f| f.to_ascii_lowercase().ends_with("bootmgfw.efi"))
            || self.description.to_ascii_lowercase().contains("windows boot manager")
    }
}

fn u16s(b: &[u8]) -> Vec<u16> {
    b.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

/// Parse an EFI_LOAD_OPTION.
pub fn parse_option(id: &str, b: &[u8]) -> Option<Entry> {
    let attrs = u32::from_le_bytes(b.get(..4)?.try_into().ok()?);
    let path_len = u16::from_le_bytes(b.get(4..6)?.try_into().ok()?) as usize;
    let units = u16s(b.get(6..)?);
    let desc_units = units.iter().position(|&u| u == 0)?;
    let description = String::from_utf16_lossy(&units[..desc_units]);
    let dp_start = 6 + (desc_units + 1) * 2;
    let dp = b.get(dp_start..(dp_start + path_len).min(b.len()))?;
    let (mut guid, mut number, mut file) = (None, None, None);
    let mut at = 0;
    while at + 4 <= dp.len() {
        let (t, st) = (dp[at], dp[at + 1]);
        let len = u16::from_le_bytes([dp[at + 2], dp[at + 3]]) as usize;
        if len < 4 || at + len > dp.len() || t == 0x7f {
            break;
        }
        let node = &dp[at..at + len];
        if t == 4 && st == 1 && len >= 42 {
            // Hard drive: partition number, start, size, signature, format, signature type.
            number = Some(u32::from_le_bytes(node[4..8].try_into().ok()?));
            if node[41] == 2 {
                guid = Some(crate::disk::guid_text(&node[24..40]));
            }
        } else if t == 4 && st == 4 {
            let s = String::from_utf16_lossy(&u16s(&node[4..]).into_iter().take_while(|&u| u != 0).collect::<Vec<_>>());
            file = Some(match file {
                Some(f) => format!("{f}{s}"),
                None => s,
            });
        }
        at += len;
    }
    Some(Entry { id: id.into(), description, active: attrs & 1 == 1, partition_guid: guid, partition_number: number, file, in_boot_order: false })
}

/// Build a load option: a GPT hard drive node plus a file path.
pub fn build_option(description: &str, part_number: u32, first_lba: u64, sectors: u64, part_guid: &[u8; 16], file: &str) -> Vec<u8> {
    let mut hd = vec![4u8, 1, 42, 0];
    hd.extend(part_number.to_le_bytes());
    hd.extend(first_lba.to_le_bytes());
    hd.extend(sectors.to_le_bytes());
    hd.extend(part_guid);
    hd.extend([2u8, 2]); // GPT, GUID signature
    let path: Vec<u8> = file.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()).collect();
    let mut fp = vec![4u8, 4];
    fp.extend(((4 + path.len()) as u16).to_le_bytes());
    fp.extend(path);
    let dp: Vec<u8> = [hd, fp, vec![0x7f, 0xff, 4, 0]].concat();
    let mut v = 1u32.to_le_bytes().to_vec(); // active
    v.extend((dp.len() as u16).to_le_bytes());
    v.extend(description.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()));
    v.extend(dp);
    v
}

fn var(dir: &Path, name: &str) -> Option<Vec<u8>> {
    let b = fs::read(dir.join(format!("{name}-{GLOBAL}"))).ok()?;
    (b.len() >= 4).then(|| b[4..].to_vec())
}

pub fn boot_order(dir: &Path) -> Vec<u16> {
    var(dir, "BootOrder").map(|b| b.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()).unwrap_or_default()
}

pub fn boot_current(dir: &Path) -> Option<u16> {
    var(dir, "BootCurrent").and_then(|b| Some(u16::from_le_bytes(b.get(..2)?.try_into().ok()?)))
}

/// Every Boot#### entry, in id order.
pub fn entries(dir: &Path) -> Vec<Entry> {
    let order = boot_order(dir);
    let mut out: Vec<Entry> = fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        let id = n.strip_prefix("Boot")?.strip_suffix(&format!("-{GLOBAL}"))?.to_string();
        if id.len() != 4 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let b = fs::read(e.path()).ok()?;
        let mut entry = parse_option(&id, b.get(4..)?)?;
        let num = u16::from_str_radix(&id, 16).ok()?;
        entry.in_boot_order = order.contains(&num);
        Some(entry)
    }).collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Write a variable (4 attribute bytes, then the value) in one write, as efivarfs needs.
pub fn write_var(dir: &Path, name: &str, value: &[u8]) -> std::io::Result<()> {
    let path = dir.join(format!("{name}-{GLOBAL}"));
    let data = [&ATTRS.to_le_bytes()[..], value].concat();
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(false).open(&path)?;
    f.write_all(&data)
}

/// A free Boot#### number.
pub fn free_id(dir: &Path) -> u16 {
    let used: Vec<u16> = entries(dir).iter().filter_map(|e| u16::from_str_radix(&e.id, 16).ok()).collect();
    (0..0xffffu16).find(|n| !used.contains(n)).unwrap_or(0x1234)
}

/// Add (or replace) a boot entry and put it first in BootOrder.
pub fn put_first(dir: &Path, id: u16, option: &[u8]) -> std::io::Result<()> {
    write_var(dir, &format!("Boot{id:04X}"), option)?;
    let mut order = boot_order(dir);
    order.retain(|&n| n != id);
    order.insert(0, id);
    let bytes: Vec<u8> = order.iter().flat_map(|n| n.to_le_bytes()).collect();
    write_var(dir, "BootOrder", &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_round_trip() {
        let g = crate::disk::guid_bytes("6a2f1b33-0d1e-4c7a-9a11-1234567890ab").unwrap();
        let opt = build_option("Windows Boot Manager", 1, 2048, 204800, &g, WINDOWS_PATH);
        let e = parse_option("0001", &opt).unwrap();
        assert_eq!(e.description, "Windows Boot Manager");
        assert!(e.active && e.is_windows());
        assert_eq!(e.partition_guid.as_deref(), Some("6a2f1b33-0d1e-4c7a-9a11-1234567890ab"));
        assert_eq!(e.partition_number, Some(1));
        assert_eq!(e.file.as_deref(), Some(WINDOWS_PATH));
        // A firmware entry with no hard drive node (e.g. a vendor/BBS path).
        let mut fw = 1u32.to_le_bytes().to_vec();
        fw.extend(8u16.to_le_bytes());
        fw.extend("NVMe0 TOSHIBA".encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()));
        fw.extend([1u8, 4, 4, 0, 0x7f, 0xff, 4, 0]);
        let e = parse_option("0002", &fw).unwrap();
        assert_eq!((e.description.as_str(), e.file.clone(), e.partition_guid.clone()), ("NVMe0 TOSHIBA", None, None));
        assert!(parse_option("0003", &[1, 0]).is_none(), "short: nothing, no panic");
    }

    #[test]
    fn write_and_order() {
        let d = std::env::temp_dir().join(format!("nexa-bootfix-efi-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        write_var(&d, "BootOrder", &[2, 0, 0, 0]).unwrap();
        let g = [7u8; 16];
        put_first(&d, 5, &build_option("Windows Boot Manager", 1, 2048, 1000, &g, WINDOWS_PATH)).unwrap();
        assert_eq!(boot_order(&d), vec![5, 2, 0]);
        let es = entries(&d);
        assert_eq!(es.len(), 1);
        assert_eq!(es[0].id, "0005");
        assert!(es[0].in_boot_order);
        assert_eq!(free_id(&d), 0);
        let _ = fs::remove_dir_all(&d);
    }
}
