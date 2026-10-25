//! MicroBrain's settings (Settings > Intelligence writes them through the
//! socket) and the resource profile picked from how much RAM there is.

use crate::pulse::Mode;
use crate::store::Doc;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Features {
    /// HabitCore: learn which apps go together (opt-in, like Patterns).
    pub habits: bool,
    /// SmartFreeze: pause apps in the background under pressure (opt-in).
    pub freeze: bool,
    pub futureload: bool,
    pub workspaces: bool,
    pub skills: bool,
    pub storage: bool,
    pub guard: bool,
    pub offline: bool,
    pub recovery: bool,
    pub search: bool,
    pub pulse: bool,
}

impl Default for Features {
    fn default() -> Self {
        Features { habits: false, freeze: false, futureload: true, workspaces: true, skills: true, storage: true,
                   guard: true, offline: true, recovery: true, search: true, pulse: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    /// "auto", "2gb", "4gb" or "8gb".
    pub profile: String,
    pub mode: Mode,
    pub features: Features,
    /// App ids SmartFreeze must never pause (besides the built-in list).
    pub never_freeze: Vec<String>,
    /// Pop-up notices for MicroBrain's decisions.
    pub notify: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { enabled: true, profile: "auto".into(), mode: Mode::Auto, features: Features::default(),
                 never_freeze: Vec::new(), notify: true }
    }
}

impl Doc for Config {
    const NAME: &'static str = "config";
    const VERSION: u32 = 1;
}

/// How much MicroBrain allows itself, by RAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Profile {
    /// About 2 GB: the essentials, rarely, with little history.
    Small,
    /// About 4 GB: everything, conservatively. The main target.
    Medium,
    /// 8 GB or more: longer history, more at once.
    Large,
}

impl Profile {
    pub fn pick(setting: &str, mem_total_mb: u64) -> Profile {
        match setting {
            "2gb" => Profile::Small,
            "4gb" => Profile::Medium,
            "8gb" => Profile::Large,
            _ if mem_total_mb == 0 => Profile::Medium,
            _ if mem_total_mb < 3000 => Profile::Small,
            _ if mem_total_mb < 7000 => Profile::Medium,
            _ => Profile::Large,
        }
    }

    /// Samples of resource history kept (one per sample interval).
    pub fn history(self) -> usize {
        match self { Profile::Small => 30, Profile::Medium => 60, Profile::Large => 120 }
    }

    /// Background jobs MicroBrain runs at once when nothing holds it back.
    pub fn max_jobs(self) -> usize {
        match self { Profile::Small => 1, Profile::Medium => 2, Profile::Large => 4 }
    }

    /// Seconds between samples while things are calm.
    pub fn calm_interval(self) -> u64 {
        match self { Profile::Small => 30, Profile::Medium => 20, Profile::Large => 10 }
    }

    /// Decisions and events kept for the Overview / Agent pages.
    pub fn log_lines(self) -> usize {
        match self { Profile::Small => 100, Profile::Medium => 300, Profile::Large => 1000 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_follows_ram_unless_chosen() {
        assert_eq!(Profile::pick("auto", 1950), Profile::Small);
        assert_eq!(Profile::pick("auto", 3900), Profile::Medium);
        assert_eq!(Profile::pick("auto", 15_900), Profile::Large);
        assert_eq!(Profile::pick("2gb", 15_900), Profile::Small, "chosen by hand");
        assert!(Profile::Small.max_jobs() < Profile::Large.max_jobs());
    }

    #[test]
    fn learning_and_freezing_start_off() {
        let c = Config::default();
        assert!(!c.features.habits && !c.features.freeze, "opt-in");
        assert!(c.enabled && c.features.futureload);
        // An old or partial file keeps the rest at their defaults.
        let partial: Config = serde_json::from_str(r#"{"features": {"freeze": true}}"#).unwrap();
        assert!(partial.features.freeze && partial.features.search && partial.enabled);
    }
}
