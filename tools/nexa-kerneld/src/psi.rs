//! Pressure Stall Information (/proc/pressure/{cpu,memory,io}): the share of
//! time programs were held up waiting for the CPU, for memory, or for the
//! disk. Readings come from the files; *triggers* make the kernel wake us
//! (POLLPRI) the moment a stall threshold is crossed, so nothing polls.

use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
pub struct Line {
    /// Percent of the last 10 / 60 seconds.
    pub avg10: f64,
    pub avg60: f64,
    /// Total stall time in microseconds since boot.
    pub total: u64,
}

/// "some": at least one task waited; "full": everything runnable waited.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
pub struct Pressure {
    pub some: Line,
    pub full: Line,
}

pub fn parse(text: &str) -> Pressure {
    let mut p = Pressure::default();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let slot = match words.next() {
            Some("some") => &mut p.some,
            Some("full") => &mut p.full,
            _ => continue,
        };
        for w in words {
            match w.split_once('=') {
                Some(("avg10", v)) => slot.avg10 = v.parse().unwrap_or(0.0),
                Some(("avg60", v)) => slot.avg60 = v.parse().unwrap_or(0.0),
                Some(("total", v)) => slot.total = v.parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    p
}

pub fn read(proc_root: &Path, resource: &str) -> Option<Pressure> {
    fs::read_to_string(proc_root.join("pressure").join(resource)).ok().map(|t| parse(&t))
}

/// Ask the kernel to signal (POLLPRI on the returned file) whenever tasks
/// stall for `stall_ms` within any `window_ms`.
pub fn trigger(proc_root: &Path, resource: &str, stall_ms: u64, window_ms: u64) -> io::Result<File> {
    let mut f = OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK)
        .open(proc_root.join("pressure").join(resource))?;
    f.write_all(format!("some {} {}\0", stall_ms * 1000, window_ms * 1000).as_bytes())?;
    Ok(f)
}

/// The resource holding things up most, in plain words, or None if calm.
pub fn summary(cpu: &Pressure, memory: &Pressure, io: &Pressure) -> String {
    let worst = [("waiting for the processor", cpu.some.avg10), ("short of memory", memory.some.avg10),
                 ("waiting for the disk", io.some.avg10)]
        .into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    if worst.1 < 5.0 {
        "Nothing is holding the computer up right now.".into()
    } else {
        let how = if worst.1 >= 40.0 { "badly" } else if worst.1 >= 15.0 { "noticeably" } else { "a little" };
        format!("Programs are {how} held up: {} {:.0}% of the last 10 seconds.", worst.0, worst.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kernel_format() {
        let p = parse("some avg10=12.50 avg60=3.00 avg300=0.20 total=11877209\nfull avg10=1.25 avg60=0.00 avg300=0.00 total=99\n");
        assert_eq!(p.some, Line { avg10: 12.5, avg60: 3.0, total: 11877209 });
        assert_eq!((p.full.avg10, p.full.total), (1.25, 99));
        // cpu has no "full" line on older kernels.
        assert_eq!(parse("some avg10=0.00 avg60=0.00 avg300=0.00 total=5\n").full, Line::default());
    }

    #[test]
    fn plain_words() {
        let calm = parse("some avg10=1.00 avg60=0 avg300=0 total=0");
        let mem = parse("some avg10=22.00 avg60=0 avg300=0 total=0");
        assert_eq!(summary(&calm, &calm, &calm), "Nothing is holding the computer up right now.");
        assert_eq!(summary(&calm, &mem, &calm), "Programs are noticeably held up: short of memory 22% of the last 10 seconds.");
    }

    #[test]
    fn real_files_when_the_kernel_has_psi() {
        if let Some(p) = read(Path::new("/proc"), "cpu") {
            assert!(p.some.avg10 >= 0.0 && p.some.avg10 <= 100.0);
        }
    }
}
