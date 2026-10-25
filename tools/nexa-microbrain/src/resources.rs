//! FutureLoad: what the computer has left, and where it's heading.
//!
//! Reads the kernel's own numbers (`/proc/meminfo`, `/proc/stat`,
//! `/proc/pressure/*`, `statvfs`, `/sys/class/power_supply`) and keeps
//! exponential moving averages and a short history, so it can say "memory
//! pressure is rising; at this rate it runs out in ~4 minutes (fairly
//! sure)". Levels change with hysteresis: a number wobbling around a
//! threshold never flips the level back and forth.
//!
//! Every reader takes the root it reads from, so tests use a fake /proc.

use serde::Serialize;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Sample {
    pub at: u64,
    pub mem_total_mb: u64,
    pub mem_avail_mb: u64,
    pub swap_total_mb: u64,
    pub swap_free_mb: u64,
    /// Share of the last 10 s some task waited for memory / CPU / I/O (PSI avg10, %).
    pub psi_mem: f64,
    pub psi_mem_full: f64,
    pub psi_cpu: f64,
    pub psi_io: f64,
    /// CPU busy since the previous sample, % of all cores.
    pub cpu_busy: f64,
    pub disk_free_mb: u64,
    pub disk_total_mb: u64,
    pub battery: Option<u8>,
    pub on_battery: bool,
    /// Programs the kernel has ended for lack of memory since boot (/proc/vmstat oom_kill).
    pub oom_kills: u64,
}

pub struct Paths {
    pub proc_root: PathBuf,
    pub sys_root: PathBuf,
    pub home: PathBuf,
}

impl Paths {
    /// The real system; NEXA_MICROBRAIN_PROC / _SYS point elsewhere (tests).
    pub fn real() -> Paths {
        let env = |k: &str, d: &str| std::env::var_os(k).map(PathBuf::from).unwrap_or_else(|| d.into());
        Paths {
            proc_root: env("NEXA_MICROBRAIN_PROC", "/proc"),
            sys_root: env("NEXA_MICROBRAIN_SYS", "/sys"),
            home: std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into()),
        }
    }
}

fn meminfo(text: &str, key: &str) -> u64 {
    text.lines()
        .find(|l| l.starts_with(key) && l[key.len()..].starts_with(':'))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        / 1024
}

/// avg10 of the "some" (or "full") line of a PSI file.
pub fn psi_avg10(text: &str, which: &str) -> f64 {
    text.lines()
        .find(|l| l.starts_with(which))
        .and_then(|l| l.split_whitespace().find_map(|f| f.strip_prefix("avg10=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0)
}

/// (busy, total) jiffies from the "cpu" line of /proc/stat.
pub fn cpu_jiffies(stat: &str) -> (u64, u64) {
    let v: Vec<u64> = stat.lines().next().unwrap_or("").split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let total: u64 = v.iter().take(8).sum();
    let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
    (total.saturating_sub(idle), total)
}

fn disk(path: &Path) -> (u64, u64) {
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else { return (0, 0) };
    // SAFETY: statvfs writes into the struct we pass; c is a valid C string.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return (0, 0);
    }
    let mb = |blocks: u64| blocks.saturating_mul(st.f_frsize as u64) / (1024 * 1024);
    (mb(st.f_bavail as u64), mb(st.f_blocks as u64))
}

fn battery(sys: &Path) -> (Option<u8>, bool) {
    let mut level = None;
    let mut on_battery = false;
    if let Ok(dir) = fs::read_dir(sys.join("class/power_supply")) {
        for e in dir.flatten() {
            let p = e.path();
            let kind = fs::read_to_string(p.join("type")).unwrap_or_default();
            if kind.trim() == "Battery" {
                level = fs::read_to_string(p.join("capacity")).ok().and_then(|c| c.trim().parse().ok());
                on_battery = fs::read_to_string(p.join("status")).map(|s| s.trim() == "Discharging").unwrap_or(false);
            }
        }
    }
    (level, on_battery)
}

/// Reads one sample. `prev_cpu` carries the previous jiffies between calls.
pub fn sample(p: &Paths, prev_cpu: &mut Option<(u64, u64)>, now: u64) -> Sample {
    let mem = fs::read_to_string(p.proc_root.join("meminfo")).unwrap_or_default();
    let psi = |f: &str| fs::read_to_string(p.proc_root.join("pressure").join(f)).unwrap_or_default();
    let (pm, pc, pi) = (psi("memory"), psi("cpu"), psi("io"));
    let (busy, total) = cpu_jiffies(&fs::read_to_string(p.proc_root.join("stat")).unwrap_or_default());
    let cpu_busy = match prev_cpu.replace((busy, total)) {
        Some((b0, t0)) if total > t0 => 100.0 * (busy.saturating_sub(b0)) as f64 / (total - t0) as f64,
        _ => 0.0,
    };
    let (disk_free_mb, disk_total_mb) = disk(&p.home);
    let (battery, on_battery) = battery(&p.sys_root);
    Sample {
        at: now,
        mem_total_mb: meminfo(&mem, "MemTotal"),
        mem_avail_mb: meminfo(&mem, "MemAvailable"),
        swap_total_mb: meminfo(&mem, "SwapTotal"),
        swap_free_mb: meminfo(&mem, "SwapFree"),
        psi_mem: psi_avg10(&pm, "some"),
        psi_mem_full: psi_avg10(&pm, "full"),
        psi_cpu: psi_avg10(&pc, "some"),
        psi_io: psi_avg10(&pi, "some"),
        cpu_busy,
        disk_free_mb,
        disk_total_mb,
        battery,
        on_battery,
        oom_kills: fs::read_to_string(p.proc_root.join("vmstat")).unwrap_or_default().lines()
            .find_map(|l| l.strip_prefix("oom_kill ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Level {
    Normal,
    Elevated,
    High,
    Critical,
}

/// Thresholds (enter, leave) per level: entering needs the higher number,
/// leaving needs the value to drop below the lower one (hysteresis).
#[derive(Clone, Copy, Debug)]
pub struct Bands {
    pub elevated: (f64, f64),
    pub high: (f64, f64),
    pub critical: (f64, f64),
}

pub const MEMORY_BANDS: Bands = Bands { elevated: (10.0, 5.0), high: (25.0, 15.0), critical: (50.0, 35.0) };
pub const CPU_BANDS: Bands = Bands { elevated: (30.0, 20.0), high: (60.0, 45.0), critical: (85.0, 70.0) };

pub fn next_level(current: Level, value: f64, b: &Bands) -> Level {
    let target = if value >= b.critical.0 {
        Level::Critical
    } else if value >= b.high.0 {
        Level::High
    } else if value >= b.elevated.0 {
        Level::Elevated
    } else {
        Level::Normal
    };
    if target >= current {
        return target;
    }
    // Going down only once below that level's leave threshold.
    let leave = match current {
        Level::Critical => b.critical.1,
        Level::High => b.high.1,
        Level::Elevated => b.elevated.1,
        Level::Normal => return Level::Normal,
    };
    if value < leave {
        // One step at a time, re-checked against the next band.
        next_level(match current { Level::Critical => Level::High, Level::High => Level::Elevated, _ => Level::Normal }, value, b)
    } else {
        current
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Forecast {
    /// What's being forecast ("memory", "disk").
    pub what: &'static str,
    /// Change per minute (MB/min, negative = shrinking).
    pub per_minute: f64,
    /// Minutes until it runs out at this rate, if it's shrinking.
    pub minutes_left: Option<f64>,
    /// 0..1: how well the recent points fit a straight line (R²), scaled by how many there are.
    pub confidence: f64,
}

/// A straight-line fit of (seconds, value): slope per second and R².
pub fn trend(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    if points.len() < 3 {
        return None;
    }
    let (sx, sy) = points.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    let (mx, my) = (sx / n, sy / n);
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in points {
        sxx += (x - mx) * (x - mx);
        sxy += (x - mx) * (y - my);
        syy += (y - my) * (y - my);
    }
    if sxx == 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    let r2 = if syy == 0.0 { 1.0 } else { (sxy * sxy) / (sxx * syy) };
    Some((slope, r2))
}

pub fn forecast(what: &'static str, points: &[(f64, f64)], floor: f64) -> Option<Forecast> {
    let (slope, r2) = trend(points)?;
    let per_minute = slope * 60.0;
    let last = points.last()?.1;
    let minutes_left = (slope < 0.0 && last > floor).then(|| (last - floor) / -per_minute);
    let confidence = (r2 * (points.len() as f64 / 12.0).min(1.0)).clamp(0.0, 1.0);
    Some(Forecast { what, per_minute, minutes_left, confidence })
}

pub struct Ema {
    pub value: Option<f64>,
    pub alpha: f64,
}

impl Ema {
    pub fn new(alpha: f64) -> Ema {
        Ema { value: None, alpha }
    }
    pub fn push(&mut self, x: f64) -> f64 {
        let v = match self.value {
            Some(v) => v + self.alpha * (x - v),
            None => x,
        };
        self.value = Some(v);
        v
    }
}

/// The running picture: smoothed values, levels and a short history.
pub struct Monitor {
    pub last: Option<Sample>,
    pub mem_level: Level,
    pub cpu_level: Level,
    pub disk_low: bool,
    pub psi_mem: Ema,
    pub cpu: Ema,
    history: VecDeque<Sample>,
    cap: usize,
    /// When the kernel ended programs for lack of memory: (after, by, how many);
    /// it happened between those two sample times. The last 20.
    pub ooms: VecDeque<(u64, u64, u64)>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Change {
    Memory(Level),
    Cpu(Level),
    DiskLow(bool),
}

impl Monitor {
    pub fn new(cap: usize) -> Monitor {
        Monitor {
            last: None, mem_level: Level::Normal, cpu_level: Level::Normal, disk_low: false,
            psi_mem: Ema::new(0.4), cpu: Ema::new(0.3), history: VecDeque::with_capacity(cap), cap: cap.max(4),
            ooms: VecDeque::new(),
        }
    }

    /// Takes a sample in; returns what changed (levels only move with hysteresis).
    pub fn push(&mut self, s: Sample) -> Vec<Change> {
        let mut changes = Vec::new();
        // Memory: PSI, but also little left (MemAvailable under 5 %) counts as high.
        let mut mem_signal = self.psi_mem.push(s.psi_mem);
        if s.mem_total_mb > 0 {
            let avail = 100.0 * s.mem_avail_mb as f64 / s.mem_total_mb as f64;
            if avail < 5.0 {
                mem_signal = mem_signal.max(MEMORY_BANDS.high.0);
            } else if avail < 10.0 {
                mem_signal = mem_signal.max(MEMORY_BANDS.elevated.0);
            }
        }
        let m = next_level(self.mem_level, mem_signal, &MEMORY_BANDS);
        if m != self.mem_level {
            self.mem_level = m;
            changes.push(Change::Memory(m));
        }
        let cpu_signal = self.cpu.push(s.psi_cpu.max(s.cpu_busy * 0.5));
        let c = next_level(self.cpu_level, cpu_signal, &CPU_BANDS);
        if c != self.cpu_level {
            self.cpu_level = c;
            changes.push(Change::Cpu(c));
        }
        if s.disk_total_mb > 0 {
            let free = 100.0 * s.disk_free_mb as f64 / s.disk_total_mb as f64;
            let low = if self.disk_low { free < 12.0 || s.disk_free_mb < 1500 } else { free < 8.0 || s.disk_free_mb < 1000 };
            if low != self.disk_low {
                self.disk_low = low;
                changes.push(Change::DiskLow(low));
            }
        }
        if self.history.len() == self.cap {
            self.history.pop_front();
        }
        if let Some(prev) = &self.last {
            if s.oom_kills > prev.oom_kills {
                if self.ooms.len() >= 20 {
                    self.ooms.pop_front();
                }
                self.ooms.push_back((prev.at, s.at, s.oom_kills - prev.oom_kills));
            }
        }
        self.history.push_back(s.clone());
        self.last = Some(s);
        changes
    }

    pub fn memory_forecast(&self) -> Option<Forecast> {
        let t0 = self.history.front()?.at as f64;
        let pts: Vec<(f64, f64)> = self.history.iter().map(|s| (s.at as f64 - t0, s.mem_avail_mb as f64)).collect();
        // "Runs out" = down to 3 % of RAM.
        let floor = self.last.as_ref().map(|s| s.mem_total_mb as f64 * 0.03).unwrap_or(0.0);
        forecast("memory", &pts, floor)
    }

    pub fn disk_forecast(&self) -> Option<Forecast> {
        let t0 = self.history.front()?.at as f64;
        let pts: Vec<(f64, f64)> = self.history.iter().map(|s| (s.at as f64 - t0, s.disk_free_mb as f64)).collect();
        forecast("disk", &pts, 200.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_kernels_numbers() {
        let mem = "MemTotal:        4028036 kB\nMemFree:  100 kB\nMemAvailable:    1024000 kB\nSwapTotal: 2097148 kB\nSwapFree: 2097148 kB\n";
        assert_eq!(meminfo(mem, "MemTotal"), 3933);
        assert_eq!(meminfo(mem, "MemAvailable"), 1000);
        assert_eq!(meminfo(mem, "Mem"), 0, "a key prefix isn't the key");
        let psi = "some avg10=12.50 avg60=3.00 avg300=1.00 total=1\nfull avg10=4.25 avg60=0 avg300=0 total=1\n";
        assert_eq!(psi_avg10(psi, "some"), 12.5);
        assert_eq!(psi_avg10(psi, "full"), 4.25);
        assert_eq!(psi_avg10("", "some"), 0.0, "no PSI: zero, not a crash");
        assert_eq!(cpu_jiffies("cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 2"), (200, 1000));
    }

    #[test]
    fn levels_change_with_hysteresis() {
        let b = &MEMORY_BANDS;
        assert_eq!(next_level(Level::Normal, 11.0, b), Level::Elevated);
        assert_eq!(next_level(Level::Elevated, 7.0, b), Level::Elevated, "between leave and enter: stays");
        assert_eq!(next_level(Level::Elevated, 4.0, b), Level::Normal);
        assert_eq!(next_level(Level::Normal, 60.0, b), Level::Critical, "jumps straight up");
        assert_eq!(next_level(Level::Critical, 20.0, b), Level::High, "comes down one band at a time");
    }

    #[test]
    fn a_wobbling_number_does_not_flap() {
        let mut m = Monitor::new(20);
        let mut changes = 0;
        for i in 0..40 {
            let psi = if i % 2 == 0 { 11.0 } else { 8.0 }; // around the 10 % line
            changes += m.push(Sample { at: i, psi_mem: psi, mem_total_mb: 4000, mem_avail_mb: 2000, ..Default::default() }).len();
        }
        assert!(changes <= 1, "{changes} level changes for a wobble");
    }

    #[test]
    fn falling_memory_is_forecast_with_confidence() {
        let mut m = Monitor::new(30);
        for i in 0..12 {
            m.push(Sample { at: i * 10, mem_total_mb: 4000, mem_avail_mb: 2000 - i * 50, ..Default::default() });
        }
        let f = m.memory_forecast().unwrap();
        assert!((f.per_minute + 300.0).abs() < 1.0, "{f:?}");
        let left = f.minutes_left.unwrap();
        assert!(left > 4.0 && left < 6.0, "{left}");
        assert!(f.confidence > 0.9);
        // Noise: low confidence, no strong claim.
        let mut n = Monitor::new(30);
        for (i, v) in [2000, 1500, 2100, 1600, 2050, 1550, 2000, 1500].iter().enumerate() {
            n.push(Sample { at: i as u64 * 10, mem_total_mb: 4000, mem_avail_mb: *v, ..Default::default() });
        }
        assert!(n.memory_forecast().unwrap().confidence < 0.3);
    }

    #[test]
    fn little_memory_left_is_high_even_without_psi() {
        let mut m = Monitor::new(10);
        let ch = m.push(Sample { at: 1, mem_total_mb: 4000, mem_avail_mb: 150, ..Default::default() });
        assert_eq!(ch, vec![Change::Memory(Level::High)]);
    }

    #[test]
    fn low_disk_with_hysteresis() {
        let mut m = Monitor::new(10);
        let s = |free| Sample { at: 1, disk_total_mb: 100_000, disk_free_mb: free, mem_total_mb: 4000, mem_avail_mb: 3000, ..Default::default() };
        assert_eq!(m.push(s(5_000)), vec![Change::DiskLow(true)]);
        assert!(m.push(s(9_000)).is_empty(), "9 % free: still low until 12 %");
        assert_eq!(m.push(s(13_000)), vec![Change::DiskLow(false)]);
    }

    #[test]
    fn reads_a_fake_system() {
        let root = std::env::temp_dir().join(format!("mb-res-{}", std::process::id()));
        let proc_ = root.join("proc");
        fs::create_dir_all(proc_.join("pressure")).unwrap();
        fs::write(proc_.join("meminfo"), "MemTotal: 2048000 kB\nMemAvailable: 512000 kB\n").unwrap();
        fs::write(proc_.join("pressure/memory"), "some avg10=20.00 avg60=0 avg300=0 total=0\nfull avg10=5.00 avg60=0 avg300=0 total=0\n").unwrap();
        fs::write(proc_.join("stat"), "cpu  10 0 10 80 0 0 0 0\n").unwrap();
        let bat = root.join("sys/class/power_supply/BAT0");
        fs::create_dir_all(&bat).unwrap();
        fs::write(bat.join("type"), "Battery\n").unwrap();
        fs::write(bat.join("capacity"), "42\n").unwrap();
        fs::write(bat.join("status"), "Discharging\n").unwrap();
        let p = Paths { proc_root: proc_, sys_root: root.join("sys"), home: root.clone() };
        let mut prev = None;
        let s = sample(&p, &mut prev, 7);
        assert_eq!((s.mem_total_mb, s.mem_avail_mb, s.psi_mem, s.psi_mem_full), (2000, 500, 20.0, 5.0));
        assert_eq!((s.battery, s.on_battery), (Some(42), true));
        assert!(s.disk_total_mb > 0);
        fs::remove_dir_all(root).ok();
    }
}
