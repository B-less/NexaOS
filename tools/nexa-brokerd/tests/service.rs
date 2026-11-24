//! The real service on a test socket, asked by the real client as different
//! users (needs root, as in the build container; skipped otherwise).

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_nexa-brokerd");

struct Service {
    child: Child,
    dir: PathBuf,
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start() -> Option<Service> {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("not root: skipping the service test");
        return None;
    }
    // Unique per test: the tests run in parallel, so one shared dir would let
    // one test's setup delete another's socket (a flaky "connection refused").
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("nexa-broker-svc-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("etc")).unwrap();
    fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    fs::write(dir.join("etc/passwd"), "root:x:0:0::/root:/bin/sh\nuser:x:1000:1000::/home/user:/bin/sh\nguest:x:1001:1001::/home/guest:/bin/sh\n").unwrap();
    fs::write(dir.join("etc/group"), "root:x:0:\nwheel:x:10:user\nuser:x:1000:\nguest:x:1001:\n").unwrap();
    fs::write(dir.join("etc/shadow"), "root:!::0:::::\nuser:$6$nexasalt$MzZ40LeatUOJUqhCI2oVd1bgvOTb6sBY6vwLqVCZBgpJcOF1s2DBQ8Jftnj.UdYccj83QrC89CZuYVEbDt5Zg1:0:::::\nguest::0:::::\n").unwrap();
    let child = Command::new(BIN).arg("serve")
        .env("NEXA_BROKER_SOCKET", dir.join("sock")).env("NEXA_BROKER_ETC", dir.join("etc")).env("NEXA_BROKER_LOG", dir.join("log"))
        .env("NEXA_BROKER_TOKEN", dir.join("token"))
        .spawn().unwrap();
    for _ in 0..100 {
        if dir.join("sock").exists() { break; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // The token is written root:wheel 0640; the uid-1000 caller in these tests
    // doesn't carry the wheel group, so open it up for the test's reader (the
    // real guard for apps is Landlock denying the path, tested in nexa-run).
    for _ in 0..100 {
        if dir.join("token").exists() { break; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = fs::set_permissions(dir.join("token"), std::os::unix::fs::PermissionsExt::from_mode(0o644));
    Some(Service { child, dir })
}

/// A trusted caller: it can read the capability token (NexaOS's own apps).
fn ask(dir: &Path, uid: u32, action: &str, args: &str, password: Option<&str>) -> Value {
    ask_token(dir, uid, action, args, password, Some(dir.join("token")))
}

/// A confined app can't read the token; point the client at a path it can't
/// read so no token is sent, exactly as a sandboxed app ends up.
fn ask_no_token(dir: &Path, uid: u32, action: &str, args: &str) -> Value {
    ask_token(dir, uid, action, args, None, Some(dir.join("no-such-token")))
}

fn ask_token(dir: &Path, uid: u32, action: &str, args: &str, password: Option<&str>, token: Option<PathBuf>) -> Value {
    let mut c = Command::new(BIN);
    c.args(["call", action, args]).env("NEXA_BROKER_SOCKET", dir.join("sock")).uid(uid);
    match token {
        Some(t) => { c.env("NEXA_BROKER_TOKEN", t); }
        None => { c.env_remove("NEXA_BROKER_TOKEN"); }
    }
    if let Some(p) = password { c.env("NEXA_BROKER_PASSWORD", p); }
    let out = c.output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

/// Send raw bytes straight to the broker socket (bypassing the client, which
/// would only ever build clean JSON), read whatever comes back within a short
/// window, and drop the connection. Returns the reply bytes (may be empty).
fn raw(dir: &Path, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if let Ok(mut s) = UnixStream::connect(dir.join("sock")) {
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = s.write_all(bytes);
        let _ = s.read_to_end(&mut out);
    }
    out
}

/// Protocol fuzz (3.1): the live daemon must survive hostile raw framing and
/// auth input -- malformed/truncated/oversized frames, wrong types where it
/// expects a token or password -- without crashing or hanging. After each, the
/// broker must still answer a normal request (i.e. it's still running).
#[test]
fn the_service_survives_hostile_raw_requests() {
    let Some(s) = start() else { return };
    let d = &s.dir;
    let mut payloads: Vec<Vec<u8>> = vec![
        b"\xff\xfe\xfd\n".to_vec(),                       // not UTF-8, not JSON
        b"garbage not json at all\n".to_vec(),
        b"{\n".to_vec(),                                   // truncated JSON
        b"{\"action\":\n".to_vec(),
        b"{}".to_vec(),                                    // no trailing newline, then EOF
        b"\n".to_vec(),                                    // empty line
        b"{\"action\":123,\"args\":\"not-an-object\"}\n".to_vec(), // wrong types
        b"{\"action\":\"set_hostname\",\"args\":{\"name\":[1,2,3]},\"token\":{},\"password\":42}\n".to_vec(),
        b"{\"action\":\"re\x00boot\",\"args\":{}}\n".to_vec(), // embedded NUL in the action
        b"{\"action\":\"install_app\",\"args\":{\"path\":12345}}\n".to_vec(),
        format!("{}\n", "[".repeat(5000)).into_bytes(),    // deep nesting -> serde recursion limit
        {
            // A line far larger than the broker will read: it must bound the read
            // and reject, not buffer it all or hang.
            let mut v = b"{\"action\":\"".to_vec();
            v.extend(std::iter::repeat(b'a').take(5_000_000));
            v.extend_from_slice(b"\"}\n");
            v
        },
    ];
    for p in payloads.drain(..) {
        let _ = raw(d, &p); // must not crash or hang the broker (2s read timeout guards us)
    }

    // Still alive and serving? A normal admin request goes through to the worker.
    let r = ask(d, 1000, "set_governor", r#"{"governor": "powersave"}"#, None);
    assert!(r["error"].as_str().map(|e| !e.contains("isn't allowed")).unwrap_or(true),
            "broker should still be serving after hostile input, got {r}");
    assert_ne!(r, Value::Null, "broker died or stopped answering after hostile input");
}

#[test]
fn who_may_ask_and_passwords() {
    let Some(s) = start() else { return };
    let d = &s.dir;
    // Root: a request the policy refuses, and one the sandboxed worker runs
    // (no backlight in a container, so it reports that).
    assert!(ask(d, 0, "format_disk", "{}", None)["error"].as_str().unwrap().contains("isn't something"));
    let r = ask(d, 0, "set_brightness", r#"{"percent": 50}"#, None);
    assert!(r["error"].as_str().unwrap().contains("brightness can't be set"), "{r}");
    // Not an administrator.
    assert!(ask(d, 1001, "set_brightness", r#"{"percent": 50}"#, None)["error"].as_str().unwrap().contains("only administrators"));
    // An administrator: allowed actions go straight through...
    let r = ask(d, 1000, "set_governor", r#"{"governor": "powersave"}"#, None);
    assert_ne!(r["needs"], "password", "{r}");
    // ...password actions ask, check, and lock out after three wrong tries.
    let r = ask(d, 1000, "set_hostname", r#"{"name": "studio"}"#, None);
    assert_eq!(r["needs"], "password", "{r}");
    assert_eq!(r["what"], "Rename this computer to studio");
    let r = ask(d, 1000, "set_hostname", r#"{"name": "studio"}"#, Some("secret"));
    assert_ne!(r["needs"], "password", "right password goes through to the worker: {r}");
    // Root reads the package with the asker's permissions, so a root-only
    // file can't be "installed" (or read) through the broker.
    let secret = d.join("secret.nexa");
    fs::write(&secret, "root only").unwrap();
    fs::set_permissions(&secret, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let r = ask(d, 1000, "install_app", &format!(r#"{{"path": "{}"}}"#, secret.display()), Some("secret"));
    assert!(r["error"].as_str().unwrap_or("").contains("Permission denied"), "{r}");
    for _ in 0..3 {
        assert_eq!(ask(d, 1000, "set_hostname", r#"{"name": "x"}"#, Some("guess"))["error"], "wrong password");
    }
    let r = ask(d, 1000, "set_hostname", r#"{"name": "x"}"#, Some("secret"));
    assert!(r["error"].as_str().unwrap().contains("too many wrong passwords"), "locked out even with the right one: {r}");
    // Everything is in the audit log.
    let log = fs::read_to_string(d.join("log")).unwrap();
    assert!(log.contains("\"refused: not an administrator\"") && log.contains("wrong password") && log.contains("\"user\":\"user\""), "{log}");
}

/// L1: an administrator caller that can't present the capability token (a
/// nexa-run-confined app, which Landlock keeps from reading the token file)
/// is refused every action, with no password even tried. The matching
/// Landlock denial is tested in nexa-run's sandbox test.
#[test]
fn a_caller_without_the_token_cannot_change_anything() {
    let Some(s) = start() else { return };
    let d = &s.dir;

    // No token: an administrator is still refused an Allow action...
    let r = ask_no_token(d, 1000, "set_governor", r#"{"governor": "powersave"}"#);
    assert!(r["error"].as_str().unwrap().contains("isn't allowed to change the system"), "{r}");
    assert_ne!(r["needs"], "password", "it must not even reach the password step: {r}");
    // ...and a password action never gets to ask for the password.
    let r = ask_no_token(d, 1000, "set_hostname", r#"{"name": "evil"}"#);
    assert!(r["error"].as_str().unwrap().contains("isn't allowed to change the system"), "{r}");
    assert_ne!(r["needs"], "password", "{r}");

    // With the token the same admin action goes through to the worker.
    let r = ask(d, 1000, "set_governor", r#"{"governor": "powersave"}"#, None);
    assert!(r["error"].as_str().map(|e| !e.contains("isn't allowed")).unwrap_or(true), "token should let it through: {r}");

    // Root never needs the token (udev, the service itself).
    let r = ask_no_token(d, 0, "set_governor", r#"{"governor": "powersave"}"#);
    assert!(r["error"].as_str().map(|e| !e.contains("isn't allowed")).unwrap_or(true), "root is exempt: {r}");

    let log = fs::read_to_string(d.join("log")).unwrap();
    assert!(log.contains("\"refused: no system token\""), "{log}");
}
