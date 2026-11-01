//! Disks and partitions, read straight from the block devices (GPT and
//! filesystem signatures). Read-only.

use serde::Serialize;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const ESP: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
pub const MS_DATA: &str = "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7";
pub const MS_RECOVERY: &str = "de94bba4-06d1-4d40-a16a-bfd50179d6ac";
pub const MS_RESERVED: &str = "e3c9e316-0b5c-4db8-817d-f92df00215ae";

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Partition {
    /// e.g. nvme0n1p1
    pub dev: String,
    pub number: u32,
    pub type_guid: String,
    pub guid: String,
    pub name: String,
    pub first_lba: u64,
    pub last_lba: u64,
    /// "EFI system", "Windows data", "Windows recovery", ...
    pub kind: String,
    /// "vfat", "ntfs", "bitlocker", "exfat", ...
    pub fs: String,
    pub size_mb: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct Disk {
    pub dev: String,
    pub model: String,
    pub size_gb: f64,
    pub removable: bool,
    /// The NexaOS stick this system started from.
    pub boot_stick: bool,
    pub gpt: bool,
    pub sector: u64,
    pub partitions: Vec<Partition>,
}

/// A GUID as stored on disk (first three fields little endian) to text.
pub fn guid_text(b: &[u8]) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15])
}

/// Text GUID back to the on-disk bytes.
pub fn guid_bytes(s: &str) -> Option<[u8; 16]> {
    let h: String = s.chars().filter(|c| *c != '-').collect();
    if h.len() != 32 {
        return None;
    }
    let mut r = [0u8; 16];
    for i in 0..16 {
        r[i] = u8::from_str_radix(h.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    r[..4].reverse();
    r[4..6].reverse();
    r[6..8].reverse();
    Some(r)
}

fn kind_of(type_guid: &str) -> &'static str {
    match type_guid {
        ESP => "EFI system (boot files)",
        MS_DATA => "Windows / data",
        MS_RECOVERY => "Windows recovery",
        MS_RESERVED => "Microsoft reserved",
        "0fc63daf-8483-4772-8e79-3d69d8477de4" => "Linux",
        "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f" => "Linux swap",
        _ => "other",
    }
}

/// The filesystem from a partition's first 4 KB.
pub fn fs_of(head: &[u8]) -> &'static str {
    let at = |o: usize, s: &[u8]| head.get(o..o + s.len()) == Some(s);
    if at(3, b"-FVE-FS-") {
        "bitlocker"
    } else if at(3, b"NTFS    ") {
        "ntfs"
    } else if at(3, b"EXFAT   ") {
        "exfat"
    } else if at(0x52, b"FAT32   ") || at(0x36, b"FAT16   ") || at(0x36, b"FAT12   ") || at(0x36, b"FAT     ") {
        "vfat"
    } else if head.len() >= 0x43a && head[0x438] == 0x53 && head[0x439] == 0xef {
        "ext4"
    } else if head.len() > 0x8006 && &head[0x8001..0x8006] == b"CD001" {
        "iso9660"
    } else {
        ""
    }
}

/// Parse a GPT: header at LBA 1, entries after it.
pub fn parse_gpt(read_at: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>, sector: u64) -> Option<Vec<(u32, String, String, String, u64, u64)>> {
    let h = read_at(sector, 92)?;
    if &h[..8] != b"EFI PART" {
        return None;
    }
    let u32at = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u64at = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let entries_lba = u64at(&h, 72);
    let count = u32at(&h, 80).min(256);
    let size = u32at(&h, 84) as usize;
    if size < 128 || size > 4096 {
        return None;
    }
    let table = read_at(entries_lba * sector, count as usize * size)?;
    let mut out = Vec::new();
    for i in 0..count as usize {
        let e = &table[i * size..(i + 1) * size];
        if e[..16].iter().all(|&b| b == 0) {
            continue;
        }
        let name: Vec<u16> = e[56..128].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
        out.push((i as u32 + 1, guid_text(&e[..16]), guid_text(&e[16..32]), String::from_utf16_lossy(&name), u64at(e, 32), u64at(e, 40)));
    }
    Some(out)
}

fn read_file(path: &Path, at: u64, len: usize) -> Option<Vec<u8>> {
    let mut f = fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(at)).ok()?;
    let mut b = vec![0u8; len];
    f.read_exact(&mut b).ok()?;
    Some(b)
}

fn sysread(p: &Path) -> String {
    fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Block devices holding the live medium: mounted as iso9660, or behind a
/// loop device (the initramfs mounts a DD-written stick through /dev/loop1).
fn live_devices(sys: &Path, proc_root: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_to_string(proc_root.join("mounts")).unwrap_or_default().lines().filter_map(|l| {
        let w: Vec<&str> = l.split_whitespace().collect();
        (w.len() > 3 && w[2] == "iso9660" && w[0].starts_with("/dev/") && !w[0].starts_with("/dev/loop"))
            .then(|| w[0].trim_start_matches("/dev/").to_string())
    }).collect();
    for e in fs::read_dir(sys.join("block")).into_iter().flatten().flatten() {
        let backing = sysread(&e.path().join("loop/backing_file"));
        if let Some(d) = backing.strip_prefix("/dev/") {
            out.push(d.to_string());
        }
    }
    out
}

/// All disks (no loop/ram/zram/optical), with their partitions.
pub fn disks(sys: &Path, dev: &Path, proc_root: &Path) -> Vec<Disk> {
    let live = live_devices(sys, proc_root);
    let mut out = Vec::new();
    let mut names: Vec<String> = fs::read_dir(sys.join("block")).into_iter().flatten().flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !["loop", "ram", "zram", "sr", "fd", "dm-", "md"].iter().any(|p| n.starts_with(p)))
        .collect();
    names.sort();
    for name in names {
        let b = sys.join("block").join(&name);
        let sector = sysread(&b.join("queue/logical_block_size")).parse().unwrap_or(512u64);
        let sectors: u64 = sysread(&b.join("size")).parse().unwrap_or(0); // always 512-byte units
        if sectors == 0 {
            continue;
        }
        let model = [sysread(&b.join("device/vendor")), sysread(&b.join("device/model"))].join(" ").trim().to_string();
        let path = dev.join(&name);
        let mut reader = |at: u64, len: usize| read_file(&path, at, len);
        let table = parse_gpt(&mut reader, sector);
        // Partition device names from sysfs: number -> name.
        let mut part_names: Vec<(u32, String)> = fs::read_dir(&b).into_iter().flatten().flatten().filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let num = sysread(&e.path().join("partition")).parse().ok()?;
            Some((num, n))
        }).collect();
        part_names.sort();
        let partitions: Vec<Partition> = table.clone().unwrap_or_default().into_iter().map(|(num, t, g, pname, first, last)| {
            let devname = part_names.iter().find(|(n, _)| *n == num).map(|(_, d)| d.clone())
                .unwrap_or_else(|| if name.chars().last().map_or(false, |c| c.is_ascii_digit()) { format!("{name}p{num}") } else { format!("{name}{num}") });
            let head = read_file(&path, first * sector, 0x8010).unwrap_or_default();
            Partition {
                dev: devname, number: num, kind: kind_of(&t).to_string(), type_guid: t, guid: g, name: pname,
                first_lba: first, last_lba: last, fs: fs_of(&head).to_string(),
                size_mb: (last + 1).saturating_sub(first) * sector / 1_048_576,
            }
        }).collect();
        // No GPT (an MBR disk, e.g. most USB drives): list what sysfs knows.
        let partitions = if table.is_some() { partitions } else {
            part_names.iter().map(|(num, pdev)| {
                let pb = b.join(pdev);
                let start: u64 = sysread(&pb.join("start")).parse().unwrap_or(0); // 512-byte units
                let size: u64 = sysread(&pb.join("size")).parse().unwrap_or(0);
                let head = read_file(&dev.join(pdev), 0, 0x8010).unwrap_or_default();
                Partition { dev: pdev.clone(), number: *num, type_guid: String::new(), guid: String::new(), name: String::new(),
                    first_lba: start * 512 / sector, last_lba: (start + size) * 512 / sector - 1, kind: "MBR partition".into(),
                    fs: fs_of(&head).to_string(), size_mb: size * 512 / 1_048_576 }
            }).collect()
        };
        // An ISO image written to the whole disk (CD001 at 32 KB) is a boot
        // stick too, NexaOS's or another one: never written to.
        let iso = read_file(&path, 0x8001, 5).map_or(false, |b| b == b"CD001") || partitions.iter().any(|p| p.fs == "iso9660");
        let boot_stick = iso || live.iter().any(|l| *l == name || partitions.iter().any(|p| p.dev == *l)
            || part_names.iter().any(|(_, n)| n == l));
        out.push(Disk {
            removable: sysread(&b.join("removable")) == "1" || fs::read_link(&b).map_or(false, |l| l.to_string_lossy().contains("/usb")),
            size_gb: (sectors as f64 * 512.0 / 1e9 * 10.0).round() / 10.0,
            dev: name, model, boot_stick, gpt: table.is_some(), sector, partitions,
        });
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A tiny GPT disk image in memory: (type, guid, name, first, last).
    pub fn gpt_image(parts: &[(&str, &str, &str, u64, u64)]) -> Vec<u8> {
        let mut img = vec![0u8; 34 * 512];
        let mut h = b"EFI PART".to_vec();
        h.extend([0, 0, 1, 0]);
        h.extend(92u32.to_le_bytes());
        h.resize(72, 0);
        h.extend(2u64.to_le_bytes()); // entries at LBA 2
        h.extend(128u32.to_le_bytes());
        h.extend(128u32.to_le_bytes());
        img[512..512 + h.len()].copy_from_slice(&h);
        for (i, (t, g, n, first, last)) in parts.iter().enumerate() {
            let at = 1024 + i * 128;
            img[at..at + 16].copy_from_slice(&guid_bytes(t).unwrap());
            img[at + 16..at + 32].copy_from_slice(&guid_bytes(g).unwrap());
            img[at + 32..at + 40].copy_from_slice(&first.to_le_bytes());
            img[at + 40..at + 48].copy_from_slice(&last.to_le_bytes());
            for (j, u) in n.encode_utf16().enumerate() {
                img[at + 56 + 2 * j..at + 58 + 2 * j].copy_from_slice(&u.to_le_bytes());
            }
        }
        img
    }

    #[test]
    fn gpt_and_filesystems() {
        let g = "6a2f1b33-0d1e-4c7a-9a11-1234567890ab";
        assert_eq!(guid_text(&guid_bytes(g).unwrap()), g);
        let img = gpt_image(&[(ESP, g, "EFI system partition", 2048, 206847), (MS_DATA, "11111111-2222-3333-4444-555555555555", "Basic data partition", 206848, 500000)]);
        let mut r = |at: u64, len: usize| img.get(at as usize..at as usize + len).map(|s| s.to_vec());
        let t = parse_gpt(&mut r, 512).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0], (1, ESP.to_string(), g.to_string(), "EFI system partition".to_string(), 2048, 206847));
        assert_eq!(t[1].0, 2);
        let mut none = |_: u64, len: usize| Some(vec![0u8; len]);
        assert!(parse_gpt(&mut none, 512).is_none(), "no GPT: MBR or blank");
        let mut boot = vec![0u8; 4096];
        boot[3..11].copy_from_slice(b"NTFS    ");
        assert_eq!(fs_of(&boot), "ntfs");
        boot[3..11].copy_from_slice(b"-FVE-FS-");
        assert_eq!(fs_of(&boot), "bitlocker");
        let mut fat = vec![0u8; 4096];
        fat[0x52..0x5a].copy_from_slice(b"FAT32   ");
        assert_eq!(fs_of(&fat), "vfat");
        assert_eq!(fs_of(&[0u8; 512]), "");
    }
}
