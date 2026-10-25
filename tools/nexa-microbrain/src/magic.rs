//! The Magic Layer's switches (~/.config/nexa/magic.json, written by
//! Settings through nexa_common.magic): each feature MicroBrain hosts asks
//! here whether it is on. Missing or damaged file: the defaults, which
//! keep learning and watching features off.

use std::path::{Path, PathBuf};

/// Off unless switched on (the same list as nexa_common.magic.OPT_IN).
pub const OPT_IN: &[&str] = &["thread", "tomorrow", "appetite", "clipboard", "surprise", "gravity", "gestures"];

pub fn path(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("nexa/magic.json")
}

pub fn enabled(home: &Path, feature: &str) -> bool {
    let default = !OPT_IN.contains(&feature);
    std::fs::read_to_string(path(home)).ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["features"][feature]["enabled"].as_bool())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_and_defaults() {
        let home = std::env::temp_dir().join(format!("magic-rs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert!(enabled(&home, "why"), "on by default");
        assert!(!enabled(&home, "clipboard"), "learning/watching features start off");
        std::fs::create_dir_all(home.join(".config/nexa")).unwrap();
        std::fs::write(home.join(".config/nexa/magic.json"), r#"{"features": {"why": {"enabled": false}, "clipboard": {"enabled": true}}}"#).unwrap();
        if std::env::var_os("XDG_CONFIG_HOME").is_none() {
            assert!(!enabled(&home, "why"));
            assert!(enabled(&home, "clipboard"));
        }
        std::fs::write(home.join(".config/nexa/magic.json"), "damaged").unwrap();
        assert!(enabled(&home, "why"));
    }
}
