//! How many threads to give the model, from the real CPU layout.
//!
//! Measured on the reference laptop (i5-8365U, 4 cores / 8 threads): one
//! thread per *physical core* generates 23 % faster than using every
//! hyper-thread. Physical cores are counted from sysfs topology: each
//! distinct (package, core) pair among the online CPUs.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn read_trim(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// "0-3,5,7-8" -> [0,1,2,3,5,7,8]
pub fn parse_cpu_list(s: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) {
                    if a <= b && b - a < 4096 {
                        out.extend(a..=b);
                    }
                }
            }
            None => out.extend(part.parse::<u32>().ok()),
        }
    }
    out
}

pub fn physical_cores(sys_root: &Path) -> usize {
    let cpu_dir = sys_root.join("sys/devices/system/cpu");
    let online = read_trim(&cpu_dir.join("online")).map(|s| parse_cpu_list(&s)).unwrap_or_default();
    let mut cores = BTreeSet::new();
    for cpu in &online {
        let topo = cpu_dir.join(format!("cpu{cpu}/topology"));
        let pkg = read_trim(&topo.join("physical_package_id")).unwrap_or_default();
        let core = read_trim(&topo.join("core_id")).unwrap_or_else(|| cpu.to_string());
        cores.insert((pkg, core));
    }
    if cores.is_empty() {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
    } else {
        cores.len()
    }
}

pub fn logical_cpus(sys_root: &Path) -> usize {
    let online = read_trim(&sys_root.join("sys/devices/system/cpu/online")).map(|s| parse_cpu_list(&s)).unwrap_or_default();
    if online.is_empty() { physical_cores(sys_root) } else { online.len() }
}

/// MemAvailable in MB.
pub fn available_mb(sys_root: &Path) -> u64 {
    fs::read_to_string(sys_root.join("proc/meminfo")).ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("MemAvailable:"))
            .and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok()))
        .map(|kb| kb / 1024).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_sys(layout: &[(u32, &str, &str)], online: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("nexa-cpu-{}-{}", std::process::id(), online.replace(['-', ','], "_")));
        let cpu = root.join("sys/devices/system/cpu");
        fs::create_dir_all(&cpu).unwrap();
        fs::write(cpu.join("online"), online).unwrap();
        for (n, pkg, core) in layout {
            let t = cpu.join(format!("cpu{n}/topology"));
            fs::create_dir_all(&t).unwrap();
            fs::write(t.join("physical_package_id"), pkg).unwrap();
            fs::write(t.join("core_id"), core).unwrap();
        }
        root
    }

    #[test]
    fn cpu_lists() {
        assert_eq!(parse_cpu_list("0-3,5,7-8\n"), vec![0, 1, 2, 3, 5, 7, 8]);
        assert_eq!(parse_cpu_list(""), Vec::<u32>::new());
        assert_eq!(parse_cpu_list("3-1,x"), Vec::<u32>::new());
    }

    #[test]
    fn hyperthreads_count_once() {
        // i5-8365U: 8 logical CPUs, cpu N and N+4 share a core
        let layout: Vec<_> = (0..8).map(|n| (n, "0", ["0", "1", "2", "3"][(n % 4) as usize])).collect();
        let root = fake_sys(&layout, "0-7");
        assert_eq!((physical_cores(&root), logical_cpus(&root)), (4, 8));
    }

    #[test]
    fn two_sockets_and_offline_cpus() {
        let root = fake_sys(&[(0, "0", "0"), (1, "0", "1"), (2, "1", "0"), (3, "1", "1")], "0-2");
        assert_eq!(physical_cores(&root), 3, "cpu3 is offline; same core id on different packages is a different core");
    }
}
