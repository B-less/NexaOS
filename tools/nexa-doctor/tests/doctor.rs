//! The real nexa-doctor: restarting a part that stops, giving up on one
//! that keeps stopping, bringing volume above 100% back, and `check`.

use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn root(name: &str) -> PathBuf {
    let r = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("doctor-{name}"));
    let _ = fs::remove_dir_all(&r);
    fs::create_dir_all(r.join("bin")).unwrap();
    fs::create_dir_all(r.join("home")).unwrap();
    // A fake wpctl: volume 135%, and it records what it's asked.
    let wpctl = r.join("bin/wpctl");
    fs::write(&wpctl, format!("#!/bin/sh\necho \"$*\" >> {}/wpctl.log\ncase \"$1\" in get-volume) cat {}/volume 2>/dev/null || echo 'Volume: 1.35' ;; set-volume) echo \"Volume: $3\" > {}/volume ;; esac\n",
        r.display(), r.display(), r.display())).unwrap();
    fs::set_permissions(&wpctl, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(r.join("bin/notify-send"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(r.join("bin/notify-send"), fs::Permissions::from_mode(0o755)).unwrap();
    r
}

fn doctor(r: &PathBuf, args: &[&str], parts: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_nexa-doctor"));
    c.args(args).env("HOME", r.join("home")).env("NEXA_DOCTOR_PARTS", parts)
        .env("NEXA_DOCTOR_SYS", r.join("sys")).env("NEXA_DOCTOR_INTERVAL_MS", "200").env("NEXA_DOCTOR_HEALTH_S", "1")
        .env("PATH", format!("{}:/usr/bin:/bin", r.join("bin").display()));
    c
}

fn alive(tag: &str) -> Vec<u32> {
    let out = Command::new("pgrep").args(["-f", &format!("sleep {tag}")]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect()
}

fn wait_until(secs: u64, f: impl Fn() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn restarts_then_gives_up_and_fixes_volume() {
    let r = root("watch");
    let tag = format!("{}.4{}", std::process::id() % 1000, 7);
    let parts = format!(r#"[{{"name":"the test part","matches":["sleep {tag}"],"start":["sleep","{tag}"]}}]"#);
    let _first = Command::new("sleep").arg(&tag).spawn().unwrap();
    let _d = Kill(doctor(&r, &["watch"], &parts).spawn().unwrap());
    assert!(wait_until(3, || !alive(&tag).is_empty()));
    std::thread::sleep(Duration::from_millis(600));
    for round in 1..=3 {
        for pid in alive(&tag) {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        assert!(wait_until(4, || !alive(&tag).is_empty()), "restarted after stop #{round}");
    }
    for pid in alive(&tag) {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    std::thread::sleep(Duration::from_secs(2));
    assert!(alive(&tag).is_empty(), "the 4th stop within 5 minutes: it gives up instead of looping");
    let log = fs::read_to_string(r.join("home/.local/state/nexa-doctor/log.jsonl")).unwrap();
    assert!(log.contains("restarted it") && log.contains("gave up"), "{log}");
    assert!(wait_until(4, || fs::read_to_string(r.join("wpctl.log")).unwrap_or_default().contains("set-volume @DEFAULT_AUDIO_SINK@ 1.0")),
            "volume above 100% brought back");
    assert!(log.contains("volume") || fs::read_to_string(r.join("home/.local/state/nexa-doctor/log.jsonl")).unwrap().contains("volume was 135%"));
}

#[test]
fn check_reports_problems() {
    let r = root("check");
    let bl = r.join("sys/class/backlight/intel_backlight");
    fs::create_dir_all(&bl).unwrap();
    fs::write(bl.join("brightness"), "0\n").unwrap();
    fs::write(bl.join("max_brightness"), "100\n").unwrap();
    let out = doctor(&r, &["check"], "[]").output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<&str> = v["result"]["findings"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"volume") && ids.contains(&"brightness"), "{v}");
    let out = doctor(&r, &["fix", "volume"], "[]").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("Volume set back to 100%"));
    let out = doctor(&r, &["fix", "nonsense"], "[]").output().unwrap();
    assert!(!out.status.success());
}
