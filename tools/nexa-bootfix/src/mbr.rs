//! Putting back the MBR partition list of a Windows installed in Legacy
//! BIOS mode, after it was deleted: which partitions, read from what's still
//! on the disk and checked against Windows' own boot settings (BCD), and the
//! 512-byte first sector to write.

use crate::lost::Found;
use serde::Serialize;

/// Boot code that starts the active partition (Syslinux mbr.bin, MIT; see
/// boot/README.md).
pub const BOOT: &[u8; 440] = include_bytes!("../boot/mbr.bin");

/// MBR partition types.
pub const NTFS: u8 = 0x07;
pub const FAT32_LBA: u8 = 0x0c;
pub const RECOVERY: u8 = 0x27;

fn u32at(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }
fn u64at(b: &[u8], o: usize) -> u64 { u64::from_le_bytes(b[o..o + 8].try_into().unwrap()) }

/// Partitions a BCD file names, MBR style: (start in bytes, disk signature).
/// BCD keeps a device as: type 6 (partition), flags, size, ..., the
/// partition's start (8 bytes) at +16, partition style 1 (MBR) at +36 and
/// the disk signature at +40.
pub fn bcd_mbr_devices(b: &[u8]) -> Vec<(u64, u32)> {
    let mut out = Vec::new();
    if b.get(..4) != Some(b"regf") {
        return out;
    }
    let mut p = 0;
    while p + 44 <= b.len() {
        if u32at(b, p) == 6 && (0x30..0x400).contains(&u32at(b, p + 8)) && u32at(b, p + 36) == 1 {
            let (off, sig) = (u64at(b, p + 16), u32at(b, p + 40));
            if off > 0 && off % 512 == 0 && sig != 0 && !out.contains(&(off, sig)) {
                out.push((off, sig));
            }
        }
        p += 1;
    }
    out
}

/// The disk signature most of a BCD's devices use, and their partition starts.
pub fn bcd_disk(b: &[u8]) -> Option<(u32, Vec<u64>)> {
    let devs = bcd_mbr_devices(b);
    let sig = devs.iter().map(|d| d.1).max_by_key(|s| devs.iter().filter(|d| d.1 == *s).count())?;
    let mut offs: Vec<u64> = devs.iter().filter(|d| d.1 == sig).map(|d| d.0).collect();
    offs.sort();
    Some((sig, offs))
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Part {
    pub start_lba: u32,
    pub sectors: u32,
    pub kind: u8,
    pub active: bool,
    pub what: String,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Plan {
    pub signature: u32,
    pub parts: Vec<Part>,
}

/// What to write, or why it isn't safe to. Every partition must be one the
/// scan found whole, none may overlap, and Windows' BCD must name the same
/// disk (signature) and starts.
pub fn plan(found: &[Found], disk_bytes: u64, sector: u64) -> Result<Plan, String> {
    let boots: Vec<&Found> = found.iter().filter(|f| f.legacy_boot && f.bcd_signature.map_or(false, |s| s != 0)).collect();
    let [boot] = boots.as_slice() else {
        return Err(if boots.is_empty() { "no Windows Legacy BIOS boot partition with readable boot settings (BCD) was found".into() }
            else { "more than one Windows Legacy BIOS boot partition was found".into() });
    };
    if found.len() > 4 {
        return Err(format!("{} partitions were found; an MBR list holds 4", found.len()));
    }
    let mut parts: Vec<&Found> = found.iter().collect();
    parts.sort_by_key(|f| f.start);
    let mut end = 34 * sector; // the first sectors hold the partition list itself
    for f in &parts {
        if !matches!(f.fs.as_str(), "ntfs" | "vfat" | "exfat" | "bitlocker") {
            return Err(format!("the partition at {} MB isn't one Windows uses", f.start >> 20));
        }
        if f.size == 0 || f.start % sector != 0 || f.size % sector != 0 {
            return Err(format!("the size of the partition at {} MB is unknown", f.start >> 20));
        }
        if f.start < end {
            return Err(format!("the partition at {} MB overlaps the one before it", f.start >> 20));
        }
        end = f.start + f.size;
        if f.start / sector > u32::MAX as u64 || f.size / sector > u32::MAX as u64 {
            return Err("the disk is too big for an MBR partition list".into());
        }
    }
    if end > disk_bytes {
        return Err("the last partition runs past the end of the disk".into());
    }
    let starts: Vec<u64> = parts.iter().map(|f| f.start).collect();
    if boot.bcd_offsets.is_empty() || boot.bcd_offsets.iter().any(|o| !starts.contains(o)) {
        return Err("Windows' boot settings (BCD) name a partition that wasn't found".into());
    }
    Ok(Plan {
        signature: boot.bcd_signature.unwrap(),
        parts: parts.iter().map(|f| Part {
            start_lba: (f.start / sector) as u32,
            sectors: (f.size / sector) as u32,
            kind: if f.what == "Windows recovery" { RECOVERY } else if f.fs == "vfat" { FAT32_LBA } else { NTFS },
            active: std::ptr::eq(*f, *boot),
            what: f.what.clone(),
        }).collect(),
    })
}

/// Cylinder/head/sector of an LBA (255 heads, 63 sectors), as stored.
pub fn chs(lba: u32) -> [u8; 3] {
    let lba = lba as u64;
    let (c, h, s) = (lba / (255 * 63), (lba / 63) % 255, lba % 63 + 1);
    let (c, h, s) = if c > 1023 { (1023, 254, 63) } else { (c, h, s) };
    [h as u8, (s as u8 & 0x3f) | ((c >> 2) as u8 & 0xc0), c as u8]
}

/// The first sector: boot code, disk signature, the partitions, 55 AA.
pub fn build(p: &Plan) -> [u8; 512] {
    let mut b = [0u8; 512];
    b[..440].copy_from_slice(BOOT);
    b[440..444].copy_from_slice(&p.signature.to_le_bytes());
    for (i, part) in p.parts.iter().enumerate() {
        let e = &mut b[446 + 16 * i..446 + 16 * (i + 1)];
        e[0] = if part.active { 0x80 } else { 0 };
        e[1..4].copy_from_slice(&chs(part.start_lba));
        e[4] = part.kind;
        e[5..8].copy_from_slice(&chs(part.start_lba + part.sectors - 1));
        e[8..12].copy_from_slice(&part.start_lba.to_le_bytes());
        e[12..16].copy_from_slice(&part.sectors.to_le_bytes());
    }
    b[510] = 0x55;
    b[511] = 0xaa;
    b
}

/// An MBR's (signature, [(active, type, start LBA, sectors)]), the used entries.
pub fn parse(b: &[u8]) -> Option<(u32, Vec<(bool, u8, u32, u32)>)> {
    if b.len() < 512 || b[510] != 0x55 || b[511] != 0xaa {
        return None;
    }
    let parts = (0..4).map(|i| &b[446 + 16 * i..446 + 16 * (i + 1)]).filter(|e| e[4] != 0)
        .map(|e| (e[0] == 0x80, e[4], u32at(e, 8), u32at(e, 12))).collect();
    Some((u32at(b, 440), parts))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A BCD-like file naming these partitions of one disk.
    pub fn fake_bcd(sig: u32, offsets: &[u64]) -> Vec<u8> {
        let mut b = b"regf".to_vec();
        b.resize(4096, 0);
        for off in offsets {
            let mut d = vec![0u8; 0x48];
            d[0..4].copy_from_slice(&6u32.to_le_bytes());
            d[8..12].copy_from_slice(&0x48u32.to_le_bytes());
            d[16..24].copy_from_slice(&off.to_le_bytes());
            d[36..40].copy_from_slice(&1u32.to_le_bytes());
            d[40..44].copy_from_slice(&sig.to_le_bytes());
            b.extend(b"\xa0\xff\xff\xffent\0");
            b.extend(d);
        }
        b
    }

    /// The ThinkPad's partitions, as the scan found them.
    pub fn thinkpad() -> Vec<Found> {
        let f = |start: u64, size: u64, what: &str| Found { start, size, fs: "ntfs".into(), what: what.into(), from: "scan".into(), ..Default::default() };
        let mut boot = f(1048576, 104857600, "Windows boot (Legacy BIOS)");
        boot.legacy_boot = true;
        boot.bcd_signature = Some(0xa76f4909);
        boot.bcd_offsets = vec![1048576, 105906176, 255154192384];
        vec![boot, f(105906176, 255048286208, "Windows (C:)"), f(255154192384, 903872512, "Windows recovery")]
    }

    #[test]
    fn reads_bcd() {
        let b = fake_bcd(0xa76f4909, &[1048576, 105906176, 1048576]);
        assert_eq!(bcd_disk(&b), Some((0xa76f4909, vec![1048576, 105906176])));
        assert_eq!(bcd_disk(&[0u8; 100]), None);
        assert_eq!(bcd_disk(b"regf"), None);
    }

    #[test]
    fn thinkpad_plan_and_sector() {
        let p = plan(&thinkpad(), 256060514304, 512).unwrap();
        assert_eq!(p.signature, 0xa76f4909);
        assert_eq!(p.parts.iter().map(|x| (x.active, x.kind, x.start_lba, x.sectors)).collect::<Vec<_>>(),
            vec![(true, NTFS, 2048, 204800), (false, NTFS, 206848, 498141184), (false, RECOVERY, 498348032, 1765376)]);
        let b = build(&p);
        assert_eq!(&b[..440], &BOOT[..]);
        assert_eq!(&b[440..444], &[0x09, 0x49, 0x6f, 0xa7]);
        assert_eq!(&b[446..462], &[0x80, 0x20, 0x21, 0x00, 0x07, 0xdf, 0x13, 0x0c, 0x00, 0x08, 0, 0, 0x00, 0x20, 0x03, 0x00]);
        assert_eq!(parse(&b), Some((0xa76f4909, vec![(true, 7, 2048, 204800), (false, 7, 206848, 498141184), (false, 0x27, 498348032, 1765376)])));
    }

    #[test]
    fn refuses_unsafe_plans() {
        let disk = 256060514304;
        // BCD names a partition the scan didn't find.
        let mut f = thinkpad();
        f[0].bcd_offsets.push(5 << 30);
        assert!(plan(&f, disk, 512).unwrap_err().contains("BCD"));
        // Overlap.
        let mut f = thinkpad();
        f[1].size += 1 << 20;
        assert!(plan(&f, disk, 512).unwrap_err().contains("overlaps"));
        // No legacy boot partition (a UEFI Windows).
        let mut f = thinkpad();
        f[0].legacy_boot = false;
        assert!(plan(&f, disk, 512).is_err());
        // Past the end of the disk.
        assert!(plan(&thinkpad(), 200 << 30, 512).unwrap_err().contains("end of the disk"));
        // Unknown size (BitLocker without one).
        let mut f = thinkpad();
        f[1].size = 0;
        assert!(plan(&f, disk, 512).is_err());
        assert_eq!(chs(0), [0, 1, 0]);
        assert_eq!(chs(2048), [32, 33, 0]);
        assert_eq!(chs(u32::MAX), [254, 0xff, 0xff]);
    }
}
