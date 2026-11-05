//! Nexa Signals: NexaOS's notifications. `nexa-signald` owns
//! org.freedesktop.Notifications and decides what shows; `nexa-signals`
//! (C, tools-c) draws it; `nexa-signalctl` is the CLI for Deck, the bar and
//! Settings. Design: TODOnexa_identity.md, section "Signals".

pub mod store;
pub mod text;

use std::path::PathBuf;

/// $XDG_RUNTIME_DIR/nexa-signals.sock (NEXA_SIGNALS_SOCKET in tests).
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("NEXA_SIGNALS_SOCKET") {
        return p.into();
    }
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        // SAFETY: getuid has no failure mode.
        PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
    });
    run.join("nexa-signals.sock")
}

/// Settings > Notifications (~/.config/nexa/notifications.json, written by
/// nexa_common.notifyconf): the default timeout and the muted apps.
pub fn prefs_path() -> PathBuf {
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    cfg.join("nexa").join("notifications.json")
}

pub fn load_prefs() -> store::Prefs {
    let v: serde_json::Value = std::fs::read_to_string(prefs_path()).ok()
        .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    store::Prefs::from_json(&v)
}
