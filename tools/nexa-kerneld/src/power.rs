//! Power: the processor package's energy counter (Intel RAPL, powercap
//! sysfs), the battery's own reading, and which programs are using it -
//! each process's share of CPU time since the last sample (/proc/<pid>/stat)
//! applied to the power being drawn.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn read_u64(p: &Path) -> Option<u64> {
    fs::read_to_string(p).ok()?.trim().parse().ok()
}

/// One RAPL package counter (microjoules, wraps at max_energy_range_uj).
struct Rapl {
    dir: PathBuf,
    last: Option<(u64, Instant)>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Battery {
    pub watts: Option<f64>,
    pub discharging: bool,
    pub percent: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub pid: i32,
    pub name: String,
    pub uid: u32,
    /// Share of all CPU time used in the last interval, percent.
    pub cpu_percent: f64,
    /// Estimated watts (when the computer can measure power).
    pub watts: Option<f64>,
}

pub struct Sampler {
    sys: PathBuf,
    proc_root: PathBuf,
    rapl: Vec<Rapl>,
    ticks: HashMap<i32, (u64, String, u32)>,
    pub package_watts: Option<f64>,
    pub battery: Battery,
    pub users: Vec<User>,
}

/// utime + stime and the name, from /proc/<pid>/stat (the name may contain
/// spaces and parentheses, so fields are counted after the last ')').
pub fn stat_ticks(stat: &str) -> Option<(u64, String)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_string();
    let rest: Vec<&str> = stat.get(close + 2..)?.split_whitespace().collect();
    // rest[0] is field 3 (state); utime and stime are fields 14 and 15.
    Some((rest.get(11)?.parse::<u64>().ok()? + rest.get(12)?.parse::<u64>().ok()?, name))
}

/// Energy delta across a counter that wraps at `range`.
pub fn energy_delta(before: u64, after: u64, range: u64) -> u64 {
    if after >= before { after - before } else { range.saturating_sub(before) + after }
}

impl Sampler {
    pub fn new(sys: &Path, proc_root: &Path) -> Sampler {
        let mut rapl = Vec::new();
        for e in fs::read_dir(sys.join("class/powercap")).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // Packages only ("intel-rapl:0"), not their sub-zones ("intel-rapl:0:0").
            if name.starts_with("intel-rapl:") && name.matches(':').count() == 1 && e.path().join("energy_uj").exists() {
                rapl.push(Rapl { dir: e.path(), last: None });
            }
        }
        Sampler { sys: sys.into(), proc_root: proc_root.into(), rapl, ticks: HashMap::new(), package_watts: None,
                  battery: Battery::default(), users: Vec::new() }
    }

    pub fn has_rapl(&self) -> bool {
        !self.rapl.is_empty()
    }

    fn sample_rapl(&mut self) {
        let now = Instant::now();
        let mut total = 0.0;
        let mut any = false;
        for z in &mut self.rapl {
            let Some(e) = read_u64(&z.dir.join("energy_uj")) else { continue };
            let range = read_u64(&z.dir.join("max_energy_range_uj")).unwrap_or(u64::MAX);
            if let Some((before, t)) = z.last {
                let secs = now.duration_since(t).as_secs_f64();
                if secs > 0.0 {
                    total += energy_delta(before, e, range) as f64 / 1e6 / secs;
                    any = true;
                }
            }
            z.last = Some((e, now));
        }
        self.package_watts = any.then_some(total);
    }

    fn sample_battery(&mut self) {
        let mut b = Battery::default();
        for e in fs::read_dir(self.sys.join("class/power_supply")).into_iter().flatten().flatten() {
            let d = e.path();
            if fs::read_to_string(d.join("type")).map(|t| t.trim() == "Battery").unwrap_or(false) {
                b.discharging = fs::read_to_string(d.join("status")).map(|s| s.trim() == "Discharging").unwrap_or(false);
                b.percent = read_u64(&d.join("capacity"));
                b.watts = read_u64(&d.join("power_now")).map(|uw| uw as f64 / 1e6).or_else(|| {
                    Some(read_u64(&d.join("current_now"))? as f64 * read_u64(&d.join("voltage_now"))? as f64 / 1e12)
                });
                break;
            }
        }
        self.battery = b;
    }

    fn sample_processes(&mut self) {
        let mut now: HashMap<i32, (u64, String, u32)> = HashMap::new();
        for e in fs::read_dir(&self.proc_root).into_iter().flatten().flatten() {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
            let Ok(stat) = fs::read_to_string(e.path().join("stat")) else { continue };
            let Some((ticks, name)) = stat_ticks(&stat) else { continue };
            let uid = fs::metadata(e.path()).map(|m| m.uid()).unwrap_or(0);
            now.insert(pid, (ticks, name, uid));
        }
        let mut deltas: Vec<(i32, u64)> = now.iter()
            .filter_map(|(pid, (t, _, _))| self.ticks.get(pid).map(|(b, _, _)| (*pid, t.saturating_sub(*b))))
            .filter(|(_, d)| *d > 0).collect();
        let total: u64 = deltas.iter().map(|(_, d)| d).sum();
        deltas.sort_by(|a, b| b.1.cmp(&a.1));
        let power = self.package_watts.or(if self.battery.discharging { self.battery.watts } else { None });
        self.users = deltas.into_iter().take(15).map(|(pid, d)| {
            let (_, name, uid) = &now[&pid];
            let share = d as f64 / total.max(1) as f64;
            User { pid, name: name.clone(), uid: *uid, cpu_percent: (share * 1000.0).round() / 10.0,
                   watts: power.map(|w| (w * share * 100.0).round() / 100.0) }
        }).collect();
        self.ticks = now;
    }

    pub fn sample(&mut self) {
        self.sample_rapl();
        self.sample_battery();
        self.sample_processes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields() {
        let s = "1234 (Web Content (x)) S 1 1234 1234 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 5 0 100 0 0";
        assert_eq!(stat_ticks(s), Some((300, "Web Content (x)".into())));
        assert_eq!(stat_ticks("garbage"), None);
    }

    #[test]
    fn counter_wraps() {
        assert_eq!(energy_delta(100, 400, 1000), 300);
        assert_eq!(energy_delta(900, 50, 1000), 150);
    }

    #[test]
    fn rapl_battery_and_shares() {
        let r = std::env::temp_dir().join(format!("nexa-kerneld-power-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let pkg = r.join("sys/class/powercap/intel-rapl:0");
        fs::create_dir_all(&pkg).unwrap();
        fs::create_dir_all(r.join("sys/class/powercap/intel-rapl:0:0")).unwrap();
        fs::write(pkg.join("energy_uj"), "1000000").unwrap();
        fs::write(pkg.join("max_energy_range_uj"), "262143328850").unwrap();
        let bat = r.join("sys/class/power_supply/BAT0");
        fs::create_dir_all(&bat).unwrap();
        for (f, v) in [("type", "Battery"), ("status", "Discharging"), ("capacity", "61"), ("current_now", "1500000"), ("voltage_now", "11400000")] {
            fs::write(bat.join(f), v).unwrap();
        }
        let mut s = Sampler::new(&r.join("sys"), Path::new("/proc"));
        assert!(s.has_rapl() && s.rapl.len() == 1, "packages only");
        s.sample();
        assert_eq!(s.package_watts, None, "needs two samples");
        std::thread::sleep(std::time::Duration::from_millis(200));
        fs::write(pkg.join("energy_uj"), "3000000").unwrap();
        // Burn a little CPU so this process shows up.
        let t = Instant::now();
        while t.elapsed().as_millis() < 50 {}
        s.sample();
        let w = s.package_watts.unwrap();
        assert!(w > 5.0 && w < 11.0, "2 J in ~0.2 s: {w}");
        assert!((s.battery.watts.unwrap() - 17.1).abs() < 0.01 && s.battery.discharging && s.battery.percent == Some(61));
        assert!(!s.users.is_empty() && s.users.iter().all(|u| u.watts.is_some()), "{:?}", s.users);
    }
}
