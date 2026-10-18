//! Reading ahead: getting file data into the page cache before it's needed,
//! and finding out what is already there.
//!
//! * `resident` maps a file read-only and asks the kernel which of its pages
//!   are in memory (mincore(2)) - no data is read to find out.
//! * `read_ahead` pulls byte ranges into the page cache with readahead(2):
//!   large, in-order reads, which on a USB stick are many times faster than
//!   the small scattered reads programs make as they start.
//! * `memory_to_spare` says whether there's room to do so without pushing
//!   anything else out (MemAvailable, and memory pressure when PSI exists).

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

pub const PAGE: u64 = 4096;

/// Which pages of the file are in the page cache (one bool per 4 KiB page).
pub fn resident(file: &File) -> io::Result<Vec<bool>> {
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(Vec::new());
    }
    // SAFETY: a read-only shared mapping of an open file; never dereferenced,
    // only passed to mincore, and unmapped before returning.
    let addr = unsafe { libc::mmap(std::ptr::null_mut(), len as usize, libc::PROT_READ, libc::MAP_SHARED, file.as_raw_fd(), 0) };
    if addr == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    let pages = len.div_ceil(PAGE) as usize;
    let mut vec = vec![0u8; pages];
    // SAFETY: vec has one byte per page of the mapping, as mincore requires.
    let rc = unsafe { libc::mincore(addr, len as usize, vec.as_mut_ptr()) };
    let err = io::Error::last_os_error();
    // SAFETY: addr/len are exactly what mmap returned above.
    unsafe { libc::munmap(addr, len as usize) };
    if rc != 0 {
        return Err(err);
    }
    Ok(vec.into_iter().map(|b| b & 1 == 1).collect())
}

/// Runs of `true` pages as (first page, page count).
pub fn runs(pages: &[bool]) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &r) in pages.iter().enumerate() {
        match (r, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s as u64, (i - s) as u64));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s as u64, (pages.len() - s) as u64));
    }
    out
}

/// Read byte range [offset, offset+len) into the page cache. Blocks until
/// the data has been read.
pub fn read_ahead(file: &File, offset: u64, len: u64) -> io::Result<()> {
    // SAFETY: plain syscall on an open fd.
    let rc = unsafe { libc::readahead(file.as_raw_fd(), offset as libc::off64_t, len as usize) };
    if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Read a whole file ahead (for the files an app opens when it starts).
pub fn read_ahead_file(path: &Path) -> io::Result<u64> {
    let f = File::open(path)?;
    let len = f.metadata()?.len();
    if len > 0 {
        read_ahead(&f, 0, len)?;
    }
    Ok(len)
}

fn meminfo_kb(text: &str, key: &str) -> Option<u64> {
    text.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse().ok()
}

/// Is there memory to spare for reading ahead `want` bytes? At least a
/// quarter of RAM must stay available afterwards, and memory must not be
/// under pressure (some avg10 below 5%) when the kernel reports it.
pub fn memory_to_spare(proc_root: &Path, want: u64) -> bool {
    let info = std::fs::read_to_string(proc_root.join("meminfo")).unwrap_or_default();
    let (Some(total), Some(avail)) = (meminfo_kb(&info, "MemTotal:"), meminfo_kb(&info, "MemAvailable:")) else { return false };
    let calm = std::fs::read_to_string(proc_root.join("pressure/memory")).ok()
        .and_then(|t| t.lines().next().and_then(|l| l.split_whitespace().find_map(|w| w.strip_prefix("avg10="))).and_then(|v| v.parse::<f64>().ok()))
        .map_or(true, |avg10| avg10 < 5.0);
    calm && avail.saturating_sub(want / 1024) > total / 4
}

/// How many bytes can be read ahead now while keeping a quarter of RAM
/// available (0 under memory pressure). For a job that can't re-check later
/// (the boot readahead loses sight of /proc when the root switches).
pub fn spare_bytes(proc_root: &Path) -> u64 {
    let info = std::fs::read_to_string(proc_root.join("meminfo")).unwrap_or_default();
    let (Some(total), Some(avail)) = (meminfo_kb(&info, "MemTotal:"), meminfo_kb(&info, "MemAvailable:")) else { return 0 };
    if !memory_to_spare(proc_root, 0) {
        return 0;
    }
    avail.saturating_sub(total / 4) * 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn runs_of_pages() {
        assert_eq!(runs(&[true, true, false, true, false, false, true]), vec![(0, 2), (3, 1), (6, 1)]);
        assert_eq!(runs(&[]), vec![]);
    }

    #[test]
    fn reading_ahead_makes_pages_resident() {
        let p = std::env::temp_dir().join(format!("nexa-sys-preload-{}", std::process::id()));
        let mut f = File::create(&p).unwrap();
        f.write_all(&vec![7u8; 64 * 1024]).unwrap();
        drop(f);
        let f = File::open(&p).unwrap();
        // Freshly written pages are cached; after reading ahead they still are.
        read_ahead(&f, 0, 64 * 1024).unwrap();
        let r = resident(&f).unwrap();
        assert_eq!(r.len(), 16);
        assert!(r.iter().all(|x| *x));
        assert_eq!(read_ahead_file(&p).unwrap(), 64 * 1024);
    }

    #[test]
    fn memory_check() {
        let d = std::env::temp_dir().join(format!("nexa-sys-mem-{}", std::process::id()));
        std::fs::create_dir_all(d.join("pressure")).unwrap();
        std::fs::write(d.join("meminfo"), "MemTotal:       8000000 kB\nMemFree: 1 kB\nMemAvailable:   5000000 kB\n").unwrap();
        assert!(memory_to_spare(&d, 1 << 30), "5 GB free of 8: 1 GB is fine");
        assert!(!memory_to_spare(&d, 4 << 30), "would leave less than a quarter");
        assert_eq!(spare_bytes(&d), (5_000_000 - 2_000_000) * 1024);
        std::fs::write(d.join("pressure/memory"), "some avg10=12.00 avg60=0 avg300=0 total=1\n").unwrap();
        assert!(!memory_to_spare(&d, 1 << 20), "under pressure: no");
    }
}
