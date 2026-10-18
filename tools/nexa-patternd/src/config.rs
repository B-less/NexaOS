//! Your choices for Nexa Patterns (~/.config/nexa/patterns.json), written by
//! Settings > Privacy and the desktop card, read by the daemon.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Learning is off until you switch it on.
    pub enabled: bool,
    pub desktop_card: bool,
    pub card_position: String,
    /// Apps never learned from.
    pub excluded_apps: Vec<String>,
    /// Apps never suggested ("Don't suggest this").
    pub never_suggest: Vec<String>,
    /// "Hide for today": the card stays empty until this Unix time.
    pub hidden_until: u64,
    /// Faster app starts: read likely apps into memory ahead of time.
    pub preload: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config { enabled: false, desktop_card: true, card_position: "top-right".into(), excluded_apps: vec![],
                 never_suggest: vec![], hidden_until: 0, preload: true }
    }
}

pub fn path(home: &Path) -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".config"));
    base.join("nexa").join("patterns.json")
}

pub fn load(path: &Path) -> (Config, Option<SystemTime>) {
    let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
    let cfg = fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    (cfg, mtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial_files() {
        let d = std::env::temp_dir().join(format!("nexa-patternd-cfg-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let p = d.join("patterns.json");
        assert_eq!(load(&p).0, Config::default());
        assert!(!Config::default().enabled, "off until switched on");
        fs::write(&p, r#"{"enabled": true, "never_suggest": ["foot"]}"#).unwrap();
        let (c, m) = load(&p);
        assert!(c.enabled && c.never_suggest == ["foot"] && c.desktop_card && m.is_some());
        fs::write(&p, "broken").unwrap();
        assert_eq!(load(&p).0, Config::default(), "a damaged file means off, not a crash");
    }
}
