//! nexa-preload: boot readahead for the live USB.
//!
//! The live system is one big image (nexa.img) read through a loop device,
//! and a boot reads a few hundred MB of it in small scattered pieces - slow
//! on a USB stick. This tool:
//!
//!   nexa-preload record IMAGE MAP [SECONDS]
//!       samples, every half second, which pages of IMAGE are in the page
//!       cache (mincore; nothing is read to find out) and when each first
//!       appeared, then writes MAP: the ranges a boot needs, in first-use
//!       order. Run once on a reference boot (make preload-map).
//!   nexa-preload replay IMAGE MAP
//!       reads those ranges ahead, in that order, as large sequential reads
//!       (readahead(2)). The initramfs starts it in the background right
//!       after it finds the image, so later reads are already in memory.
//!       Stops early if memory gets tight; does nothing if MAP was made for
//!       a different image.
//!   nexa-preload show MAP          what a map holds
//!
//! The map is plain text: a header with the image's size and fingerprint, then
//! "order first_page pages" lines.

use nexa_sys::preload;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

const MAGIC: &str = "nexa-preload 1";
const INTERVAL: Duration = Duration::from_millis(500);
/// Ranges of the same order closer than this are read as one (a few extra
/// pages cost less than a seek).
const MERGE_GAP: u64 = 32;
/// Never read ahead more than this.
const MAX_BYTES: u64 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    pub order: u32,
    pub page: u64,
    pub pages: u64,
}

/// Which image a map belongs to: its size and a fingerprint of its first
/// 64 KiB (where ext4 keeps the superblock, unique per build). Not the
/// mtime: copying the ISO onto a FAT stick (Rufus) doesn't keep it exactly.
fn identity(path: &Path) -> Option<(u64, u64)> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = vec![0u8; 65536.min(len as usize)];
    f.read_exact(&mut head).ok()?;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in head {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    Some((len, h))
}

/// From "first seen at sample k" per page (u32::MAX = never) to ordered,
/// merged extents.
pub fn extents(first_seen: &[u32]) -> Vec<Extent> {
    let mut out: Vec<Extent> = Vec::new();
    let mut i = 0usize;
    while i < first_seen.len() {
        let order = first_seen[i];
        if order == u32::MAX {
            i += 1;
            continue;
        }
        let start = i;
        while i < first_seen.len() && first_seen[i] == order {
            i += 1;
        }
        out.push(Extent { order, page: start as u64, pages: (i - start) as u64 });
    }
    out.sort_by_key(|e| (e.order, e.page));
    let mut merged: Vec<Extent> = Vec::new();
    for e in out {
        match merged.last_mut() {
            Some(m) if m.order == e.order && e.page <= m.page + m.pages + MERGE_GAP => {
                m.pages = e.page + e.pages - m.page;
            }
            _ => merged.push(e),
        }
    }
    merged
}

pub fn write_map(path: &Path, image: (u64, u64), ext: &[Extent]) -> std::io::Result<()> {
    let mut text = format!("{MAGIC} {} {}\n", image.0, image.1);
    for e in ext {
        text.push_str(&format!("{} {} {}\n", e.order, e.page, e.pages));
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

pub fn read_map(path: &Path) -> Option<((u64, u64), Vec<Extent>)> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next()?.strip_prefix(MAGIC)?.split_whitespace().collect();
    let image = (head.first()?.parse().ok()?, head.get(1)?.parse().ok()?);
    let ext = lines.filter_map(|l| {
        let f: Vec<u64> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        (f.len() == 3).then(|| Extent { order: f[0] as u32, page: f[1], pages: f[2] })
    }).collect();
    Some((image, ext))
}

fn record(image: &Path, map: &Path, seconds: u64) -> Result<(), String> {
    let f = File::open(image).map_err(|e| format!("{}: {e}", image.display()))?;
    let id = identity(image).ok_or("can't stat the image")?;
    let mut first_seen: Vec<u32> = Vec::new();
    let start = Instant::now();
    let mut k = 0u32;
    while start.elapsed() < Duration::from_secs(seconds) {
        let resident = preload::resident(&f).map_err(|e| format!("mincore: {e}"))?;
        if first_seen.is_empty() {
            first_seen = vec![u32::MAX; resident.len()];
        }
        for (slot, r) in first_seen.iter_mut().zip(resident) {
            if r && *slot == u32::MAX {
                *slot = k;
            }
        }
        k += 1;
        std::thread::sleep(INTERVAL);
    }
    let ext = extents(&first_seen);
    write_map(map, id, &ext).map_err(|e| format!("{}: {e}", map.display()))?;
    let pages: u64 = ext.iter().map(|e| e.pages).sum();
    eprintln!("nexa-preload: recorded {} MB in {} ranges over {k} samples", pages * preload::PAGE >> 20, ext.len());
    Ok(())
}

fn replay(image: &Path, map: &Path) -> Result<(), String> {
    let (id, ext) = read_map(map).ok_or("no usable map")?;
    if identity(image) != Some(id) {
        return Err("the map was made for a different image; skipping".into());
    }
    let f = File::open(image).map_err(|e| format!("{}: {e}", image.display()))?;
    // Decided once: /proc may vanish from view when the root switches.
    let budget = preload::spare_bytes(Path::new("/proc")).min(MAX_BYTES);
    let (mut done, start) = (0u64, Instant::now());
    for e in ext {
        let len = e.pages * preload::PAGE;
        if done + len > budget {
            break;
        }
        let _ = preload::read_ahead(&f, e.page * preload::PAGE, len);
        done += len;
    }
    eprintln!("nexa-preload: read {} MB ahead in {:.1} s", done >> 20, start.elapsed().as_secs_f64());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let a = |i: usize| args.get(i).map(String::as_str);
    let result = match (a(1), a(2), a(3)) {
        (Some("record"), Some(img), Some(map)) => record(Path::new(img), Path::new(map), a(4).and_then(|s| s.parse().ok()).unwrap_or(75)),
        (Some("replay"), Some(img), Some(map)) => replay(Path::new(img), Path::new(map)),
        (Some("show"), Some(map), _) => match read_map(Path::new(map)) {
            Some(((size, print), ext)) => {
                let pages: u64 = ext.iter().map(|e| e.pages).sum();
                let _ = writeln!(std::io::stdout(), "image {size} bytes (fingerprint {print:016x}): {} MB in {} ranges, {} steps",
                                 pages * preload::PAGE >> 20, ext.len(), ext.iter().map(|e| e.order).max().map_or(0, |o| o + 1));
                Ok(())
            }
            None => Err("not a nexa-preload map".into()),
        },
        _ => Err("usage: nexa-preload record IMAGE MAP [SECONDS] | replay IMAGE MAP | show MAP".into()),
    };
    if let Err(e) = result {
        eprintln!("nexa-preload: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn extents_in_first_use_order() {
        const N: u32 = u32::MAX;
        // Pages 0-2 and 30 first (merged: the gap is under MERGE_GAP), page 5 later.
        let mut seen = vec![N; 200];
        seen[0] = 0;
        seen[1] = 0;
        seen[2] = 0;
        seen[5] = 3;
        seen[30] = 0;
        let e = extents(&seen);
        assert_eq!(e, vec![Extent { order: 0, page: 0, pages: 31 }, Extent { order: 3, page: 5, pages: 1 }]);
        assert!(extents(&[N, N]).is_empty());
    }

    #[test]
    fn map_round_trip_and_replay() {
        let d = std::env::temp_dir().join(format!("nexa-preload-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let img = d.join("nexa.img");
        fs::write(&img, vec![1u8; 256 * 1024]).unwrap();
        let f = File::open(&img).unwrap();
        // Push the image out of the page cache (allowed for any file you can open).
        // SAFETY: plain fadvise on an open fd.
        unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
        let map = d.join("boot.map");
        write_map(&map, identity(&img).unwrap(), &[Extent { order: 0, page: 8, pages: 16 }]).unwrap();
        assert_eq!(read_map(&map).unwrap().1, vec![Extent { order: 0, page: 8, pages: 16 }]);
        replay(&img, &map).unwrap();
        let r = preload::resident(&f).unwrap();
        assert!(r[8..24].iter().all(|x| *x), "the mapped range is now in memory");
        // A different image (another build): the map is ignored.
        fs::write(&img, vec![1u8; 300 * 1024]).unwrap();
        assert!(replay(&img, &map).unwrap_err().contains("different image"));
    }

    #[test]
    fn records_what_was_read() {
        let d = std::env::temp_dir().join(format!("nexa-preload-rec-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let img = d.join("nexa.img");
        fs::write(&img, vec![2u8; 128 * 1024]).unwrap();
        let f = File::open(&img).unwrap();
        // SAFETY: plain fadvise on an open fd.
        unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
        preload::read_ahead(&f, 0, 16 * 1024).unwrap();
        record(&img, &d.join("m"), 1).unwrap();
        let (_, ext) = read_map(&d.join("m")).unwrap();
        assert!(ext.first().map_or(false, |e| e.page == 0 && e.pages >= 4), "{ext:?}");
    }
}
