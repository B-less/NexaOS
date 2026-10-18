//! The real daemon on a fake /sys (power meter, battery, cgroup tree) and the
//! real /proc, asked over its socket like the assistant asks it.

use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

struct Daemon {
    child: Child,
    root: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Daemon {
    fn start(name: &str) -> Daemon {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("kerneld-{name}"));
        let _ = fs::remove_dir_all(&root);
        let sys = root.join("sys");
        let pkg = sys.join("class/powercap/intel-rapl:0");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("energy_uj"), "1000").unwrap();
        fs::write(pkg.join("max_energy_range_uj"), "262143328850").unwrap();
        let bat = sys.join("class/power_supply/BAT0");
        fs::create_dir_all(&bat).unwrap();
        for (f, v) in [("type", "Battery"), ("status", "Discharging"), ("capacity", "80"), ("power_now", "9500000")] {
            fs::write(bat.join(f), v).unwrap();
        }
        let cg = sys.join("fs/cgroup");
        fs::create_dir_all(&cg).unwrap();
        fs::write(cg.join("cgroup.controllers"), "cpu memory io").unwrap();
        // The AI's account is "us", so its cgroup gets our (test) processes.
        let me = unsafe { libc::getuid() };
        let etc = root.join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::write(etc.join("passwd"), format!("root:x:0:0::/root:/bin/sh\nnexa-mind:x:{me}:{me}::/var/lib/nexa-mind:/sbin/nologin\n")).unwrap();
        fs::write(etc.join("group"), "wheel:x:10:root\n").unwrap();
        fs::write(root.join("kmsg"), "").unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_nexa-kerneld")).arg("serve")
            .env("NEXA_KERNELD_SYS", &sys).env("NEXA_KERNELD_ETC", &etc).env("NEXA_KERNELD_KMSG", root.join("kmsg"))
            .env("NEXA_KERNELD_SOCKET", root.join("k.sock")).spawn().unwrap();
        for _ in 0..100 {
            if root.join("k.sock").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(200));
        Daemon { child, root }
    }

    fn ask(&self, req: Value) -> Value {
        let mut s = UnixStream::connect(self.root.join("k.sock")).unwrap();
        writeln!(s, "{req}").unwrap();
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn ok(&self, req: Value) -> Value {
        let r = self.ask(req.clone());
        assert_eq!(r["ok"], true, "{req} -> {r}");
        r["result"].clone()
    }

    fn cg(&self, f: &str) -> String {
        fs::read_to_string(self.root.join("sys/fs/cgroup/nexa-mind-model").join(f)).unwrap_or_default()
    }
}

#[test]
fn sees_and_steers() {
    if unsafe { libc::getuid() } != 0 {
        eprintln!("note: needs root (the test container is root); skipped");
        return;
    }
    let d = Daemon::start("main");
    let s = d.ok(json!({"op": "status"}));
    assert_eq!(s["can"]["power_meter"], true, "{s}");
    assert_eq!(s["can"]["ai_cgroup"], true);
    if Path::new("/proc/pressure/cpu").exists() {
        assert_eq!(s["can"]["pressure"], true);
        assert!(s["pressure"]["summary"].as_str().unwrap().len() > 10);
        assert!(s["pressure"]["memory"]["some"]["avg10"].is_number());
    }
    assert_eq!(s["power"]["battery"]["watts"], 9.5);
    // Nobody at the keyboard yet: the AI may use everything.
    assert_eq!(s["ai"]["limit"], "free", "{s}");
    assert_eq!((d.cg("cpu.weight"), d.cg("cpu.max")), ("100".into(), "max 100000".into()));

    // An app comes to the front: it gets the boost (if the kernel can), and the AI yields.
    let mut app = Command::new("sleep").arg("30").spawn().unwrap();
    let f = d.ok(json!({"op": "focus", "pid": app.id()}));
    assert_eq!((f["pid"].as_u64(), f["name"].as_str()), (Some(app.id() as u64), Some("sleep")));
    assert_eq!(f["boosted"], s["can"]["app_boost"], "boosted exactly when the kernel supports uclamp");
    let s = d.ok(json!({"op": "status"}));
    assert_eq!(s["ai"]["limit"], "background");
    assert_eq!(s["focus"]["name"], "sleep");
    assert_eq!(d.cg("cpu.weight"), "20");
    let _ = app.kill();
    let _ = app.wait();

    // Energy: programs ranked, watts estimated from the battery's reading.
    fs::write(d.root.join("sys/class/powercap/intel-rapl:0/energy_uj"), "5000000").unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    let e = d.ok(json!({"op": "energy"}));
    assert_eq!(e["measured"], true, "{e}");
    assert!(e["summary"].as_str().unwrap().contains("using the most") || e["programs"].as_array().unwrap().is_empty(), "{e}");

    // Crashes, if this kernel lets us hear about them (host pid namespace + CAP_NET_ADMIN).
    if s["can"]["process_events"] == true {
        let _ = Command::new("sh").args(["-c", "ulimit -c 0; kill -SEGV $$"]).status();
        std::thread::sleep(Duration::from_millis(300));
        let ev = d.ok(json!({"op": "events"}));
        let crash = ev["events"].as_array().unwrap().iter().find(|e| e["kind"] == "crash");
        assert!(crash.is_some_and(|c| c["text"].as_str().unwrap().contains("crashed with a memory error")), "{ev}");
    } else {
        eprintln!("note: no process connector here (container pid namespace); crash detection checked by unit tests");
    }

    assert!(d.ask(json!({"op": "focus", "pid": 999_999_999}))["error"].as_str().unwrap().contains("isn't running"));
    assert!(d.ask(json!({"op": "nonsense"}))["error"].as_str().unwrap().contains("unknown op"));
    assert!(d.ask(json!("not an object"))["ok"] == false);
}
