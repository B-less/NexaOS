//! `nexa-kerneld focus-watch`: runs as you inside the desktop session,
//! follows which window has focus (Sway's IPC or Hyprland's event socket)
//! and tells the daemon its process id. The daemon (root) never connects to
//! your session; it only accepts a focus report for a process you own.

use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

const I3_MAGIC: &[u8; 6] = b"i3-ipc";
const SUBSCRIBE: u32 = 2;
const EVENT_WINDOW: u32 = 0x8000_0003;

/// An i3/Sway IPC message: magic, length, type, payload.
pub fn i3_message(kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut m = I3_MAGIC.to_vec();
    m.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    m.extend_from_slice(&kind.to_ne_bytes());
    m.extend_from_slice(payload);
    m
}

fn i3_read(s: &mut impl Read) -> io::Result<(u32, Vec<u8>)> {
    let mut head = [0u8; 14];
    s.read_exact(&mut head)?;
    if &head[..6] != I3_MAGIC {
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

/// The pid in a Sway "window" event, if focus moved to a window.
pub fn sway_focus_pid(body: &[u8]) -> Option<i32> {
    let v: Value = serde_json::from_slice(body).ok()?;
    if v["change"] != "focus" {
        return None;
    }
    v["container"]["pid"].as_i64().filter(|p| *p > 0).map(|p| p as i32)
}

fn report(socket: &Path, pid: i32) {
    if let Ok(mut s) = UnixStream::connect(socket) {
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = writeln!(s, "{}", json!({"op": "focus", "pid": pid}));
        let mut reply = String::new();
        let _ = BufReader::new(s).read_line(&mut reply);
    }
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })))
}

fn sway_socket() -> Option<PathBuf> {
    if let Some(s) = std::env::var_os("SWAYSOCK") {
        return Some(s.into());
    }
    std::fs::read_dir(runtime_dir()).ok()?.flatten().map(|e| e.path())
        .find(|p| p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("sway-ipc.")))
}

fn watch_sway(path: &Path, daemon: &Path) -> io::Result<()> {
    let mut s = UnixStream::connect(path)?;
    s.write_all(&i3_message(SUBSCRIBE, br#"["window"]"#))?;
    let (_, reply) = i3_read(&mut s)?;
    if !String::from_utf8_lossy(&reply).contains("true") {
        return Err(io::Error::other("Sway refused the subscription"));
    }
    let mut last = 0;
    loop {
        let (kind, body) = i3_read(&mut s)?;
        if kind == EVENT_WINDOW {
            if let Some(pid) = sway_focus_pid(&body).filter(|p| *p != last) {
                last = pid;
                report(daemon, pid);
            }
        }
    }
}

fn watch_hyprland(sig: &str, daemon: &Path) -> io::Result<()> {
    let base = [runtime_dir().join("hypr").join(sig), PathBuf::from("/tmp/hypr").join(sig)]
        .into_iter().find(|d| d.join(".socket2.sock").exists()).ok_or_else(|| io::Error::other("no Hyprland socket"))?;
    let events = BufReader::new(UnixStream::connect(base.join(".socket2.sock"))?);
    let mut last = 0;
    for line in events.lines() {
        if !line?.starts_with("activewindowv2>>") {
            continue;
        }
        // Ask which process the focused window belongs to.
        let mut q = UnixStream::connect(base.join(".socket.sock"))?;
        q.write_all(b"j/activewindow")?;
        let mut body = Vec::new();
        q.read_to_end(&mut body)?;
        let pid = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v["pid"].as_i64()).unwrap_or(0) as i32;
        if pid > 0 && pid != last {
            last = pid;
            report(daemon, pid);
        }
    }
    Ok(())
}

/// Follow focus until the compositor goes away.
pub fn watch(daemon: &Path) -> io::Result<()> {
    if let Ok(sig) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        return watch_hyprland(&sig, daemon);
    }
    let path = sway_socket().ok_or_else(|| io::Error::other("no Sway or Hyprland session found"))?;
    watch_sway(&path, daemon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sway_protocol() {
        let m = i3_message(SUBSCRIBE, b"[\"window\"]");
        assert_eq!(&m[..6], b"i3-ipc");
        assert_eq!(u32::from_ne_bytes(m[6..10].try_into().unwrap()), 10);
        let (kind, body) = i3_read(&mut &m[..]).unwrap();
        assert_eq!((kind, body.as_slice()), (SUBSCRIBE, &b"[\"window\"]"[..]));
        assert_eq!(sway_focus_pid(br#"{"change":"focus","container":{"pid":4242,"app_id":"foot"}}"#), Some(4242));
        assert_eq!(sway_focus_pid(br#"{"change":"title","container":{"pid":4242}}"#), None);
        assert_eq!(sway_focus_pid(br#"{"change":"focus","container":{"pid":0}}"#), None);
        assert!(i3_read(&mut &b"nope-ipc......"[..]).is_err());
    }

    #[test]
    fn follows_a_fake_sway() {
        // A fake compositor: accept, answer the subscription, send two focus events.
        let dir = std::env::temp_dir().join(format!("nexa-kerneld-sway-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (sway, daemon) = (dir.join("sway.sock"), dir.join("daemon.sock"));
        let sway_l = std::os::unix::net::UnixListener::bind(&sway).unwrap();
        let daemon_l = std::os::unix::net::UnixListener::bind(&daemon).unwrap();
        std::thread::spawn(move || {
            let (mut c, _) = sway_l.accept().unwrap();
            let _ = i3_read(&mut c).unwrap();
            c.write_all(&i3_message(SUBSCRIBE, br#"{"success":true}"#)).unwrap();
            for pid in [11, 11, 22] {
                c.write_all(&i3_message(EVENT_WINDOW, format!(r#"{{"change":"focus","container":{{"pid":{pid}}}}}"#).as_bytes())).unwrap();
            }
        });
        let d = daemon.clone();
        std::thread::spawn(move || {
            let _ = watch_sway(&sway, &d);
        });
        let mut got = Vec::new();
        for _ in 0..2 {
            let (c, _) = daemon_l.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&c).read_line(&mut line).unwrap();
            got.push(serde_json::from_str::<Value>(&line).unwrap()["pid"].as_i64().unwrap());
            let _ = (&c).write_all(b"{\"ok\":true}\n");
        }
        assert_eq!(got, vec![11, 22], "repeats are not re-sent");
    }
}
