//! Which app is in front, from the compositor (Sway's IPC or Hyprland's
//! event socket): its Wayland app id (or X11 class) and process id. The
//! window title is looked at only to recognise private browsing, and is
//! never kept.

use serde_json::Value;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Focus {
    pub app: String,
    pub pid: i32,
    /// A private or incognito browser window: never learned from.
    pub private: bool,
}

/// Titles browsers give private windows (NexaOS's own, Firefox, Chromium).
pub fn is_private(title: &str) -> bool {
    let t = title.to_lowercase();
    t.ends_with(" — private") || t.ends_with(" — tor") || t.contains("private browsing") || t.contains("incognito")
}

const MAGIC: &[u8; 6] = b"i3-ipc";

pub fn i3_message(kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut m = MAGIC.to_vec();
    m.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    m.extend_from_slice(&kind.to_ne_bytes());
    m.extend_from_slice(payload);
    m
}

pub fn i3_read(s: &mut impl Read) -> io::Result<(u32, Vec<u8>)> {
    let mut head = [0u8; 14];
    s.read_exact(&mut head)?;
    if &head[..6] != MAGIC {
        return Err(io::Error::other("not an i3-ipc message"));
    }
    let len = u32::from_ne_bytes(head[6..10].try_into().unwrap()) as usize;
    if len > 16 << 20 {
        return Err(io::Error::other("message too big"));
    }
    let kind = u32::from_ne_bytes(head[10..14].try_into().unwrap());
    let mut body = vec![0u8; len];
    s.read_exact(&mut body)?;
    Ok((kind, body))
}

/// A Sway "window" event that moved focus.
pub fn sway_focus(body: &[u8]) -> Option<Focus> {
    let v: Value = serde_json::from_slice(body).ok()?;
    if v["change"] != "focus" {
        return None;
    }
    let c = &v["container"];
    let app = c["app_id"].as_str().or_else(|| c["window_properties"]["class"].as_str())?.to_string();
    Some(Focus { app, pid: c["pid"].as_i64().unwrap_or(0) as i32, private: is_private(c["name"].as_str().unwrap_or("")) })
}

pub fn hyprland_focus(body: &[u8]) -> Option<Focus> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let app = v["class"].as_str().filter(|c| !c.is_empty())?.to_string();
    Some(Focus { app, pid: v["pid"].as_i64().unwrap_or(0) as i32, private: is_private(v["title"].as_str().unwrap_or("")) })
}

fn runtime_dir() -> PathBuf {
    // SAFETY: getuid can't fail.
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })))
}

fn sway_socket() -> Option<PathBuf> {
    if let Some(s) = std::env::var_os("SWAYSOCK") {
        return Some(s.into());
    }
    std::fs::read_dir(runtime_dir()).ok()?.flatten().map(|e| e.path())
        .find(|p| p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("sway-ipc.")))
}

fn watch_sway(path: &Path, on: &mut dyn FnMut(Focus)) -> io::Result<()> {
    let mut s = UnixStream::connect(path)?;
    s.write_all(&i3_message(2, br#"["window"]"#))?;
    let (_, reply) = i3_read(&mut s)?;
    if !String::from_utf8_lossy(&reply).contains("true") {
        return Err(io::Error::other("Sway refused the subscription"));
    }
    loop {
        let (kind, body) = i3_read(&mut s)?;
        if kind == 0x8000_0003 {
            if let Some(f) = sway_focus(&body) {
                on(f);
            }
        }
    }
}

fn watch_hyprland(sig: &str, on: &mut dyn FnMut(Focus)) -> io::Result<()> {
    let base = [runtime_dir().join("hypr").join(sig), PathBuf::from("/tmp/hypr").join(sig)]
        .into_iter().find(|d| d.join(".socket2.sock").exists()).ok_or_else(|| io::Error::other("no Hyprland socket"))?;
    for line in BufReader::new(UnixStream::connect(base.join(".socket2.sock"))?).lines() {
        if !line?.starts_with("activewindowv2>>") {
            continue;
        }
        let mut q = UnixStream::connect(base.join(".socket.sock"))?;
        q.write_all(b"j/activewindow")?;
        let mut body = Vec::new();
        q.read_to_end(&mut body)?;
        if let Some(f) = hyprland_focus(&body) {
            on(f);
        }
    }
    Ok(())
}

/// Report every focus change until the compositor goes away.
pub fn watch(on: &mut dyn FnMut(Focus)) -> io::Result<()> {
    if let Ok(sig) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        return watch_hyprland(&sig, on);
    }
    watch_sway(&sway_socket().ok_or_else(|| io::Error::other("no Sway or Hyprland session"))?, on)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_focus_and_spots_private_windows() {
        assert_eq!(sway_focus(br#"{"change":"focus","container":{"pid":42,"app_id":"nexa-files","name":"Files"}}"#),
                   Some(Focus { app: "nexa-files".into(), pid: 42, private: false }));
        assert_eq!(sway_focus(br#"{"change":"focus","container":{"pid":7,"app_id":null,"window_properties":{"class":"Gimp"},"name":"x"}}"#).unwrap().app, "Gimp");
        assert!(sway_focus(r#"{"change":"focus","container":{"pid":9,"app_id":"nexa-browser","name":"Bank — Private"}}"#.as_bytes()).unwrap().private);
        assert_eq!(sway_focus(br#"{"change":"title","container":{"pid":42,"app_id":"x"}}"#), None);
        assert!(hyprland_focus(br#"{"class":"firefox","title":"Mozilla Firefox Private Browsing","pid":5}"#).unwrap().private);
        assert_eq!(hyprland_focus(br#"{"class":"","pid":0}"#), None);
        assert!(!is_private("Privacy settings") && is_private("New Tab - Google Chrome (Incognito)"));
    }
}
