//! AdaptivePulse: how much background work NexaOS allows itself right now.
//!
//! The person picks a mode (or Automatic); the conditions (memory and CPU
//! pressure from FutureLoad, battery) adjust it. The result is a budget
//! the scheduler and the agents follow. It never kills a process and never
//! touches CPU frequency; it only holds back NexaOS's own background work
//! (scans, indexing, preloading, maintenance) and says why.

use crate::config::Profile;
use crate::resources::Level;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Auto,
    Balanced,
    LowResource,
    BatterySaver,
    Performance,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
    pub fn label(self) -> &'static str {
        match self {
            Mode::Auto => "Automatic",
            Mode::Balanced => "Balanced",
            Mode::LowResource => "Low resource",
            Mode::BatterySaver => "Battery saver",
            Mode::Performance => "Performance",
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Budget {
    /// The mode in effect (Automatic resolves to one of the others).
    pub mode: Mode,
    /// Background jobs allowed at once (0 = all waiting).
    pub jobs: usize,
    pub scans: bool,
    pub indexing: bool,
    pub preload: bool,
    /// Seconds between resource samples.
    pub sample_secs: u64,
    pub why: String,
}

pub struct Conditions {
    pub memory: Level,
    pub cpu: Level,
    pub on_battery: bool,
    pub battery: Option<u8>,
}

fn budget(mode: Mode, profile: Profile, why: String) -> Budget {
    let calm = profile.calm_interval();
    match mode {
        Mode::Performance => Budget { mode, jobs: (profile.max_jobs() * 2).min(4), scans: true, indexing: true, preload: true, sample_secs: calm.min(10), why },
        Mode::Balanced | Mode::Auto => Budget { mode: Mode::Balanced, jobs: profile.max_jobs(), scans: true, indexing: true, preload: true, sample_secs: calm, why },
        Mode::LowResource => Budget { mode, jobs: 0, scans: false, indexing: false, preload: false, sample_secs: 10, why },
        Mode::BatterySaver => Budget { mode, jobs: 0, scans: false, indexing: false, preload: false, sample_secs: calm.max(30), why },
    }
}

/// The budget for `chosen` under these conditions.
pub fn decide(chosen: Mode, c: &Conditions, profile: Profile) -> Budget {
    let tight = c.memory >= Level::High || c.cpu >= Level::Critical;
    let low_battery = c.on_battery && c.battery.is_some_and(|b| b <= 20);
    if chosen == Mode::Auto {
        return if c.memory >= Level::High {
            budget(Mode::LowResource, profile, "memory is under pressure, so background work waits".into())
        } else if c.cpu >= Level::High {
            budget(Mode::LowResource, profile, "the processor is busy, so background work waits".into())
        } else if c.on_battery {
            budget(Mode::BatterySaver, profile, "on battery: avoidable work waits for the charger".into())
        } else if profile == Profile::Large && c.memory == Level::Normal && c.cpu == Level::Normal {
            budget(Mode::Performance, profile, "plenty of memory and processor free".into())
        } else {
            budget(Mode::Balanced, profile, "conditions are normal".into())
        };
    }
    let mut b = budget(chosen, profile, format!("{} mode (your choice)", chosen.label()));
    // Safety over preference: even in Performance mode, nothing extra runs
    // while the computer is short of memory or nearly out of battery.
    if tight || low_battery {
        b.jobs = 0;
        b.scans = false;
        b.indexing = false;
        b.preload = false;
        b.why = if tight {
            format!("{} mode, but memory or processor is under pressure, so background work waits", chosen.label())
        } else {
            format!("{} mode, but the battery is low, so background work waits", chosen.label())
        };
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calm() -> Conditions {
        Conditions { memory: Level::Normal, cpu: Level::Normal, on_battery: false, battery: None }
    }

    #[test]
    fn automatic_follows_conditions() {
        assert_eq!(decide(Mode::Auto, &calm(), Profile::Medium).mode, Mode::Balanced);
        assert_eq!(decide(Mode::Auto, &calm(), Profile::Large).mode, Mode::Performance);
        let busy = Conditions { memory: Level::High, ..calm() };
        let b = decide(Mode::Auto, &busy, Profile::Medium);
        assert_eq!((b.mode, b.jobs, b.scans), (Mode::LowResource, 0, false));
        assert!(b.why.contains("memory"));
        let battery = Conditions { on_battery: true, battery: Some(70), ..calm() };
        assert_eq!(decide(Mode::Auto, &battery, Profile::Medium).mode, Mode::BatterySaver);
    }

    #[test]
    fn a_chosen_mode_is_kept_but_safety_wins() {
        let perf = decide(Mode::Performance, &calm(), Profile::Medium);
        assert_eq!((perf.mode, perf.jobs), (Mode::Performance, 4));
        let short = Conditions { memory: Level::Critical, ..calm() };
        let b = decide(Mode::Performance, &short, Profile::Medium);
        assert_eq!((b.mode, b.jobs, b.scans), (Mode::Performance, 0, false));
        let low = Conditions { on_battery: true, battery: Some(12), ..calm() };
        assert_eq!(decide(Mode::Balanced, &low, Profile::Medium).jobs, 0);
    }

    #[test]
    fn small_machines_do_less() {
        assert_eq!(decide(Mode::Balanced, &calm(), Profile::Small).jobs, 1);
        assert!(decide(Mode::Balanced, &calm(), Profile::Small).sample_secs >= 30);
        assert_eq!(Mode::parse("low-resource"), Some(Mode::LowResource));
        assert_eq!(Mode::parse("turbo"), None);
    }
}
