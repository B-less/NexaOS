//! The real service in a fake session: a fake Sway sends focus changes, the
//! test asks over the socket like the desktop card and Settings do.

use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::mpsc;
use std::time::Duration;

fn i3(kind: u32, payload: &str) -> Vec<u8> {
    let mut m = b"i3-ipc".to_vec();
    m.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    m.extend_from_slice(&kind.to_ne_bytes());
    m.extend_from_slice(payload.as_bytes());
    m
}

struct Session {
    root: PathBuf,
    child: Child,
    focus: mpsc::Sender<String>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Session {
    fn start(name: &str, seed_days: Option<u64>) -> Session {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("patterns-{name}"));
        let _ = fs::remove_dir_all(&root);
        for d in ["home/.config/nexa", "home/.local/share/nexa-patterns", "run", "sys/class/power_supply"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        if let Some(days) = seed_days {
            let ok = Command::new(env!("CARGO_BIN_EXE_nexa-patternd"))
                .args(["synthetic", &days.to_string(), root.join("home/.local/share/nexa-patterns/journal.bin").to_str().unwrap()])
                .status().unwrap().success();
            assert!(ok);
        }
        // A fake Sway: answers the subscription, then forwards focus events.
        let sway = root.join("run/sway-ipc.sock");
        let listener = UnixListener::bind(&sway).unwrap();
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            let mut head = [0u8; 14];
            c.read_exact(&mut head).unwrap();
            let len = u32::from_ne_bytes(head[6..10].try_into().unwrap()) as usize;
            let mut body = vec![0u8; len];
            c.read_exact(&mut body).unwrap();
            c.write_all(&i3(2, r#"{"success":true}"#)).unwrap();
            for ev in rx {
                if c.write_all(&i3(0x8000_0003, &ev)).is_err() {
                    break;
                }
            }
        });
        let child = Command::new(env!("CARGO_BIN_EXE_nexa-patternd")).arg("serve")
            .env_clear().env("HOME", root.join("home")).env("XDG_RUNTIME_DIR", root.join("run")).env("SWAYSOCK", &sway)
            .env("NEXA_PATTERNS_SYS", root.join("sys")).env("NEXA_PATTERNS_DWELL_MS", "300").env("TZ", "UTC")
            .spawn().unwrap();
        for _ in 0..100 {
            if root.join("run/nexa-patterns.sock").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Session { root, child, focus: tx }
    }

    fn ask(&self, req: Value) -> Value {
        let mut s = UnixStream::connect(self.root.join("run/nexa-patterns.sock")).unwrap();
        writeln!(s, "{req}").unwrap();
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["ok"], true, "{req} -> {v}");
        v["result"].clone()
    }

    fn configure(&self, cfg: Value) {
        fs::write(self.root.join("home/.config/nexa/patterns.json"), cfg.to_string()).unwrap();
        self.ask(json!({"op": "reload"}));
    }

    /// Focus a window and keep it in front past the dwell time.
    fn use_app(&self, app: &str, pid: i32, title: &str) {
        self.focus.send(json!({"change": "focus", "container": {"app_id": app, "pid": pid, "name": title}}).to_string()).unwrap();
        std::thread::sleep(Duration::from_millis(700));
    }
}

#[test]
fn learns_only_when_allowed() {
    let s = Session::start("allowed", None);
    std::thread::sleep(Duration::from_millis(300));
    s.use_app("foot", 10, "Terminal");
    assert_eq!(s.ask(json!({"op": "status"}))["records"], 0, "off until switched on");
    assert_eq!(s.ask(json!({"op": "card"}))["show"], false);

    s.configure(json!({"enabled": true, "excluded_apps": ["nexa-pulse"]}));
    s.use_app("foot", 10, "Terminal");
    s.use_app("nexa-files", 11, "Files");
    s.use_app("nexa-browser", 12, "My bank — Private");
    s.use_app("nexa-pulse", 13, "Pulse");
    s.use_app("foot", 10, "Terminal");
    // A quick flick doesn't count: in front for less than the dwell time.
    s.focus.send(json!({"change": "focus", "container": {"app_id": "nexa-calc", "pid": 14, "name": "Calc"}}).to_string()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    s.use_app("nexa-files", 11, "Files");
    let e = s.ask(json!({"op": "export"}));
    let seen: Vec<(String, String)> = e["records"].as_array().unwrap().iter()
        .map(|r| (r["app"].as_str().unwrap().into(), r["kind"].as_str().unwrap().into())).collect();
    // Terminal was already open before learning was switched on: a switch.
    assert_eq!(seen, vec![("foot".into(), "switch".into()), ("nexa-files".into(), "launch".into()),
                          ("foot".into(), "switch".into()), ("nexa-files".into(), "switch".into())],
               "no private window, no excluded app, no flick; titles never kept");
    assert!(!e.to_string().contains("bank") && !e.to_string().contains("Terminal"));
    let journal = fs::read(s.root.join("home/.local/share/nexa-patterns/journal.bin")).unwrap();
    assert!(!String::from_utf8_lossy(&journal).contains("bank"));
    assert_eq!(s.ask(json!({"op": "card"}))["learning"], true, "too new to suggest anything");

    s.ask(json!({"op": "forget", "app": "foot"}));
    assert_eq!(s.ask(json!({"op": "status"}))["records"], 2);
    s.ask(json!({"op": "forget", "all": true}));
    assert_eq!(s.ask(json!({"op": "status"}))["records"], 0);
    assert!(!s.root.join("home/.local/share/nexa-patterns/journal.bin").exists(), "forgetting deletes the file");
}

#[test]
fn a_month_in_suggestions() {
    let s = Session::start("month", Some(30));
    s.configure(json!({"enabled": true}));
    let st = s.ask(json!({"op": "status"}));
    assert_eq!(st["ready"], true, "{st}");
    let p = s.ask(json!({"op": "predict", "k": 5}));
    let apps = p["apps"].as_array().unwrap();
    assert!(!apps.is_empty(), "{p}");
    for a in apps {
        assert!(a["p"].as_f64().unwrap() >= 0.12 && a["why"].as_str().unwrap().len() > 10, "{a}");
    }
    let top = apps[0]["app"].as_str().unwrap().to_string();

    let card = s.ask(json!({"op": "card"}));
    assert_eq!((card["show"].as_bool(), card["position"].as_str()), (Some(true), Some("top-right")));
    assert!(card["apps"].as_array().unwrap().len() <= 3);

    // "Don't suggest this" (settings) and "Hide for today".
    s.configure(json!({"enabled": true, "never_suggest": [top]}));
    assert!(s.ask(json!({"op": "predict", "k": 10}))["apps"].as_array().unwrap().iter().all(|a| a["app"] != top));
    s.configure(json!({"enabled": true, "hidden_until": 4_000_000_000u64}));
    let card = s.ask(json!({"op": "card"}));
    assert!(card["hidden"] == true && card["apps"].as_array().unwrap().is_empty());

    // Dismissals push an app down; apps lists what was learned.
    s.configure(json!({"enabled": true}));
    let before = s.ask(json!({"op": "predict", "k": 10}))["apps"].as_array().unwrap().iter()
        .find(|a| a["app"] == top).map(|a| a["p"].as_f64().unwrap()).unwrap();
    for _ in 0..3 {
        s.ask(json!({"op": "feedback", "app": top, "action": "dismissed"}));
    }
    let after = s.ask(json!({"op": "predict", "k": 10}))["apps"].as_array().unwrap().iter()
        .find(|a| a["app"] == top).map_or(0.0, |a| a["p"].as_f64().unwrap());
    assert!(after < before, "{after} < {before}");
    let learned = s.ask(json!({"op": "apps"}));
    assert!(learned["apps"].as_array().unwrap().len() >= 6, "{learned}");
}
