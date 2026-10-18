//! Steering the kernel for the person at the keyboard.
//!
//! * The app in front gets a CPU floor with utilization clamping
//!   (sched_setattr, SCHED_FLAG_UTIL_CLAMP_MIN on each of its threads): the
//!   scheduler and the CPU frequency governor treat it as busy, so it stays
//!   responsive even in power-saving mode. Needs CONFIG_UCLAMP_TASK.
//! * The on-device AI (everything the nexa-mind account runs, i.e.
//!   llama-server) lives in its own cgroup: a small CPU share while you are
//!   working, a hard cap when the computer is under pressure, everything
//!   when you're away. Falls back to nice levels without cgroup v2.

use crate::psi::Pressure;
use serde::Serialize;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;

/// The kernel's struct sched_attr (SCHED_ATTR_SIZE_VER1, 56 bytes).
#[repr(C)]
#[derive(Default)]
struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,
    sched_nice: i32,
    sched_priority: u32,
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
    sched_util_min: u32,
    sched_util_max: u32,
}

/// The floor the app in front gets (of 1024): about 40% of a core's capacity.
pub const FOCUS_FLOOR: u32 = 410;

/// Set a thread's utilization floor, keeping its policy and priority.
pub fn set_floor(tid: i32, min: u32) -> io::Result<()> {
    let attr = SchedAttr {
        size: std::mem::size_of::<SchedAttr>() as u32,
        sched_flags: SCHED_FLAG_KEEP_POLICY | SCHED_FLAG_KEEP_PARAMS | SCHED_FLAG_UTIL_CLAMP_MIN,
        sched_util_min: min.min(1024),
        ..Default::default()
    };
    // SAFETY: attr is a valid sched_attr of the size it declares.
    let rc = unsafe { libc::syscall(libc::SYS_sched_setattr, tid, &attr as *const SchedAttr, 0u32) };
    if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Can this kernel clamp? (Setting our own floor to 0 changes nothing.)
pub fn uclamp_supported() -> bool {
    // SAFETY: gettid can't fail.
    set_floor(unsafe { libc::gettid() }, 0).is_ok()
}

pub fn threads(proc_root: &Path, pid: i32) -> Vec<i32> {
    fs::read_dir(proc_root.join(pid.to_string()).join("task")).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().to_string_lossy().parse().ok()).collect()
}

/// Floor every thread of a process (new threads inherit it on creation).
pub fn floor_process(proc_root: &Path, pid: i32, min: u32) -> io::Result<usize> {
    let mut n = 0;
    let mut last = None;
    for tid in threads(proc_root, pid) {
        match set_floor(tid, min) {
            Ok(()) => n += 1,
            Err(e) => last = Some(e),
        }
    }
    match (n, last) {
        (0, Some(e)) => Err(e),
        (0, None) => Err(io::Error::from_raw_os_error(libc::ESRCH)),
        _ => Ok(n),
    }
}

pub fn owner(proc_root: &Path, pid: i32) -> Option<u32> {
    fs::metadata(proc_root.join(pid.to_string())).ok().map(|m| m.uid())
}

/// How much the AI may use.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelLimit {
    /// You're away: all it wants.
    Free,
    /// You're working: it yields to you (low CPU weight).
    Background,
    /// The computer is struggling: hard-capped.
    Squeezed,
}

impl ModelLimit {
    pub fn words(self) -> &'static str {
        match self {
            ModelLimit::Free => "The AI may use the whole computer (you're away).",
            ModelLimit::Background => "The AI yields to your apps while you work.",
            ModelLimit::Squeezed => "The AI is held back: the computer is busy or short of memory.",
        }
    }
}

/// The policy, from pressure and whether someone is using the computer.
pub fn decide(cpu: &Pressure, memory: &Pressure, active: bool) -> ModelLimit {
    if memory.some.avg10 >= 10.0 || (active && cpu.some.avg10 >= 25.0) {
        ModelLimit::Squeezed
    } else if active {
        ModelLimit::Background
    } else {
        ModelLimit::Free
    }
}

/// The AI's cgroup (cgroup v2), or nice levels when there is none.
pub struct ModelGroup {
    pub cgroup_root: PathBuf,
    pub proc_root: PathBuf,
    pub uid: Option<u32>,
    pub applied: Option<ModelLimit>,
}

impl ModelGroup {
    fn dir(&self) -> PathBuf {
        self.cgroup_root.join("nexa-mind-model")
    }

    pub fn has_cgroup(&self) -> bool {
        self.cgroup_root.join("cgroup.controllers").exists()
    }

    /// The AI account's processes.
    pub fn pids(&self) -> Vec<i32> {
        let Some(uid) = self.uid else { return vec![] };
        fs::read_dir(&self.proc_root).into_iter().flatten().flatten()
            .filter_map(|e| e.file_name().to_string_lossy().parse::<i32>().ok())
            .filter(|pid| owner(&self.proc_root, *pid) == Some(uid)).collect()
    }

    /// Apply a limit (and pull in processes started since last time).
    pub fn apply(&mut self, limit: ModelLimit, cpus: usize) -> io::Result<()> {
        let pids = self.pids();
        if self.has_cgroup() {
            let dir = self.dir();
            if !dir.exists() {
                fs::create_dir(&dir)?;
            }
            // The cpu controller must be on for the root's children.
            let _ = fs::write(self.cgroup_root.join("cgroup.subtree_control"), "+cpu");
            let inside: Vec<i32> = fs::read_to_string(dir.join("cgroup.procs")).unwrap_or_default()
                .lines().filter_map(|l| l.trim().parse().ok()).collect();
            for pid in pids.iter().filter(|p| !inside.contains(p)) {
                let _ = fs::write(dir.join("cgroup.procs"), pid.to_string());
            }
            let (weight, max) = match limit {
                ModelLimit::Free => ("100".to_string(), "max 100000".to_string()),
                ModelLimit::Background => ("20".to_string(), "max 100000".to_string()),
                // At most a quarter of the processors (at least one).
                ModelLimit::Squeezed => ("10".to_string(), format!("{} 100000", 100000 * (cpus / 4).max(1))),
            };
            fs::write(dir.join("cpu.weight"), weight)?;
            fs::write(dir.join("cpu.max"), max)?;
        } else {
            let nice = match limit { ModelLimit::Free => 0, ModelLimit::Background => 10, ModelLimit::Squeezed => 19 };
            for pid in &pids {
                for tid in threads(&self.proc_root, *pid) {
                    // SAFETY: setpriority on a thread id; failures are ignored.
                    unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, nice) };
                }
            }
        }
        self.applied = Some(limit);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psi::parse;

    #[test]
    fn policy() {
        let calm = parse("some avg10=2.00 avg60=0 avg300=0 total=0");
        let busy = parse("some avg10=30.00 avg60=0 avg300=0 total=0");
        assert_eq!(decide(&calm, &calm, false), ModelLimit::Free);
        assert_eq!(decide(&calm, &calm, true), ModelLimit::Background);
        assert_eq!(decide(&busy, &calm, true), ModelLimit::Squeezed);
        assert_eq!(decide(&busy, &calm, false), ModelLimit::Free, "busy only with the AI itself: let it run");
        assert_eq!(decide(&calm, &busy, false), ModelLimit::Squeezed, "short of memory: always held back");
    }

    #[test]
    fn sched_attr_matches_the_kernel() {
        assert_eq!(std::mem::size_of::<SchedAttr>(), 56);
    }

    #[test]
    fn cgroup_limits() {
        let r = std::env::temp_dir().join(format!("nexa-kerneld-cg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        fs::create_dir_all(r.join("nexa-mind-model")).unwrap();
        fs::write(r.join("cgroup.controllers"), "cpu memory").unwrap();
        // SAFETY: getuid can't fail.
        let me = unsafe { libc::getuid() };
        let mut g = ModelGroup { cgroup_root: r.clone(), proc_root: "/proc".into(), uid: Some(me), applied: None };
        g.apply(ModelLimit::Squeezed, 8).unwrap();
        let read = |f: &str| fs::read_to_string(r.join("nexa-mind-model").join(f)).unwrap();
        assert_eq!((read("cpu.weight"), read("cpu.max")), ("10".into(), "200000 100000".into()));
        assert!(!read("cgroup.procs").is_empty(), "the account's processes were moved in");
        g.apply(ModelLimit::Background, 8).unwrap();
        assert_eq!((read("cpu.weight"), read("cpu.max")), ("20".into(), "max 100000".into()));
        assert_eq!(g.applied, Some(ModelLimit::Background));
    }

    #[test]
    fn floors_when_the_kernel_can() {
        // SAFETY: getpid can't fail.
        let me = unsafe { libc::getpid() };
        match floor_process(Path::new("/proc"), me, 100) {
            Ok(n) => {
                assert!(n >= 1);
                floor_process(Path::new("/proc"), me, 0).unwrap();
            }
            Err(e) => assert!(!uclamp_supported(), "only fails without uclamp: {e}"),
        }
    }
}
