//! Layer moods (Strata, TODOnexa_identity.md phase 5): each of the five
//! layers (workspaces 1 to 5) can hold notifications back. Which layer is on
//! screen is asked of the compositor when a notification comes in, so it is
//! always right, whatever changed the layer. The moods are in
//! ~/.config/nexa/strata.conf (Settings > Layers), read each time: a small
//! file, and a change applies at once.
//!
//!   layer.N.signals=all|quiet   quiet: only important ones pop up
//!   layer.N.mute=App,App        these apps' don't pop up on that layer

use serde_json::Value;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub const LAYERS: u32 = 5;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Mood {
    pub layer: u32,
    pub name: String,
    pub quiet: bool,
    pub muted: Vec<String>,
}

impl Mood {
    pub fn mutes(&self, app: &str) -> bool {
        !app.is_empty() && self.muted.iter().any(|m| m.eq_ignore_ascii_case(app) || m.to_lowercase() == app.to_lowercase())
    }
}

/// Layer n's mood from strata.conf's text (no mood: the default).
pub fn parse(text: &str, layer: u32) -> Mood {
    let mut m = Mood { layer, ..Default::default() };
    if !(1..=LAYERS).contains(&layer) {
        return m;
    }
    let prefix = format!("layer.{layer}.");
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let Some((k, v)) = line.split_once('=') else { continue };
        let Some(key) = k.strip_prefix(&prefix) else { continue };
        match key {
            "name" => m.name = v.chars().filter(|c| !c.is_control()).take(31).collect::<String>().trim().to_string(),
            "signals" => m.quiet = v.trim() == "quiet",
            "mute" => {
                m.muted = v.split(',').map(|a| a.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string())
                    .filter(|a| !a.is_empty()).take(50).collect()
            }
            _ => {}
        }
    }
    m
}

pub fn conf_path() -> PathBuf {
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    cfg.join("nexa").join("strata.conf")
}

/// The focused workspace's number from Sway's GET_WORKSPACES reply.
pub fn sway_focused(reply: &[u8]) -> Option<u32> {
    let v: Value = serde_json::from_slice(reply).ok()?;
    v.as_array()?.iter().find(|w| w["focused"].as_bool() == Some(true))?["num"].as_i64().filter(|n| *n > 0).map(|n| n as u32)
}

/// The active workspace's id from Hyprland's j/activeworkspace.
pub fn hypr_active(reply: &[u8]) -> Option<u32> {
    let v: Value = serde_json::from_slice(reply).ok()?;
    v["id"].as_i64().filter(|n| *n > 0).map(|n| n as u32)
}

fn ask_sway(sock: &std::ffi::OsStr) -> Option<Vec<u8>> {
    let mut s = UnixStream::connect(sock).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    let mut msg = b"i3-ipc".to_vec();
    msg.extend(0u32.to_ne_bytes());
    msg.extend(1u32.to_ne_bytes()); // GET_WORKSPACES
    s.write_all(&msg).ok()?;
    let mut head = [0u8; 14];
    s.read_exact(&mut head).ok()?;
    let len = u32::from_ne_bytes(head[6..10].try_into().ok()?) as usize;
    if len > 1 << 20 {
        return None;
    }
    let mut body = vec![0; len];
    s.read_exact(&mut body).ok()?;
    Some(body)
}

fn ask_hyprland(sig: &std::ffi::OsStr) -> Option<Vec<u8>> {
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    let sock = [run.join("hypr"), PathBuf::from("/tmp/hypr")].into_iter().map(|d| d.join(sig).join(".socket.sock"))
        .find(|p| p.exists())?;
    let mut s = UnixStream::connect(sock).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    s.write_all(b"j/activeworkspace").ok()?;
    let mut out = vec![];
    s.take(1 << 20).read_to_end(&mut out).ok()?;
    Some(out)
}

/// The layer on screen now: 1..5, or None (another workspace, no compositor).
pub fn current() -> Option<u32> {
    let n = if let Some(sig) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
        hypr_active(&ask_hyprland(&sig)?)
    } else {
        sway_focused(&ask_sway(&std::env::var_os("SWAYSOCK")?)?)
    };
    n.filter(|n| (1..=LAYERS).contains(n))
}

/// The mood of the layer on screen now (the default when there is none).
pub fn now() -> Mood {
    match current() {
        Some(n) => parse(&std::fs::read_to_string(conf_path()).unwrap_or_default(), n),
        None => Mood::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moods_from_the_file() {
        let t = "show=yes\nlayer.3.name=Work\nlayer.3.signals=quiet\nlayer.3.mute=Chat, Mail ,,\nlayer.30.name=no\n# layer.1.signals=quiet\n";
        let m = parse(t, 3);
        assert_eq!((m.name.as_str(), m.quiet, m.muted.clone()), ("Work", true, vec!["Chat".to_string(), "Mail".to_string()]));
        assert!(m.mutes("chat") && !m.mutes("Files") && !m.mutes(""));
        assert_eq!(parse(t, 1), Mood { layer: 1, ..Default::default() }, "no mood: all of them pop up");
        assert_eq!(parse(t, 9).name, "", "not a layer");
    }

    #[test]
    fn the_layer_from_the_compositor() {
        assert_eq!(sway_focused(br#"[{"num": 1, "focused": false}, {"num": 4, "focused": true}]"#), Some(4));
        assert_eq!(sway_focused(br#"[{"num": -1, "focused": true}]"#), None, "a named workspace isn't a layer");
        assert_eq!(sway_focused(b"garbage"), None);
        assert_eq!(hypr_active(br#"{"id": 2, "name": "2"}"#), Some(2));
        assert_eq!(hypr_active(br#"{"id": -98, "name": "special:minimized"}"#), None);
    }
}
