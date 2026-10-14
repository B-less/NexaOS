//! The real service on a test socket, asked by the real client as different
//! users (needs root, as in the build container; skipped otherwise).

use serde_json::Value;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

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
    let dir = std::env::temp_dir().join(format!("nexa-broker-svc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("etc")).unwrap();
    fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    fs::write(dir.join("etc/passwd"), "root:x:0:0::/root:/bin/sh\nuser:x:1000:1000::/home/user:/bin/sh\nguest:x:1001:1001::/home/guest:/bin/sh\n").unwrap();
    fs::write(dir.join("etc/group"), "root:x:0:\nwheel:x:10:user\nuser:x:1000:\nguest:x:1001:\n").unwrap();
    fs::write(dir.join("etc/shadow"), "root:!::0:::::\nuser:$6$nexasalt$MzZ40LeatUOJUqhCI2oVd1bgvOTb6sBY6vwLqVCZBgpJcOF1s2DBQ8Jftnj.UdYccj83QrC89CZuYVEbDt5Zg1:0:::::\nguest::0:::::\n").unwrap();
    let child = Command::new(BIN).arg("serve")
        .env("NEXA_BROKER_SOCKET", dir.join("sock")).env("NEXA_BROKER_ETC", dir.join("etc")).env("NEXA_BROKER_LOG", dir.join("log"))
        .spawn().unwrap();
    for _ in 0..100 {
        if dir.join("sock").exists() { break; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Some(Service { child, dir })
}

fn ask(dir: &Path, uid: u32, action: &str, args: &str, password: Option<&str>) -> Value {
    let mut c = Command::new(BIN);
    c.args(["call", action, args]).env("NEXA_BROKER_SOCKET", dir.join("sock")).uid(uid);
    if let Some(p) = password { c.env("NEXA_BROKER_PASSWORD", p); }
    let out = c.output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
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
