//! nexa-patternd: Nexa Patterns, on the device. Runs as you inside the
//! desktop session, learns which apps you use when, where and after what,
//! and predicts what you'll open next (docs/NEXA_PATTERNS.md).
//!
//!   nexa-patternd serve                  the session service
//!   nexa-patternd card | status | apps   ask it (prints JSON)
//!   nexa-patternd replay [--synthetic DAYS | JOURNAL]   measure the models
//!   nexa-patternd synthetic DAYS FILE    write a synthetic journal (tests)
//!
//! Off until you switch it on (Settings > Privacy > Patterns). It records an
//! app only after it has been in front for 5 seconds, never private browser
//! windows or apps you excluded, keeps 14 days of raw records (older ones
//! become counts), and forgets on request. Socket
//! $XDG_RUNTIME_DIR/nexa-patterns.sock (yours only), one JSON line each way:
//!   {"op": "card"}  what the desktop card shows: apps (with why), a file
//!   {"op": "predict", "k": 3}   {"op": "why", "app": ...}
//!   {"op": "feedback", "app": ..., "action": "used" | "dismissed"}
//!   {"op": "forget", "app": ... | "since": unix_time | "all": true}
//!   {"op": "warm", "app": ...}  read an app into memory now (the
//!     assistant: load the AI model); done by itself every minute for the
//!     apps at least 30% likely to be next (phase 3, "faster starts")
//!   {"op": "apps"}  {"op": "status"}  {"op": "export"}  {"op": "reload"}

mod compositor;
mod config;
mod context;
mod journal;
mod model;
mod recent;
mod replay;
mod warm;

use journal::{Kind, Record};
use model::Model;
use nexa_sys::{peer, sandbox};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Raw records older than this become counts in the base model.
const KEEP_RAW: u64 = 14 * 86400;
/// In front this long before it counts as using the app.
fn dwell() -> Duration {
    // NEXA_PATTERNS_DWELL_MS: tests only.
    Duration::from_millis(std::env::var("NEXA_PATTERNS_DWELL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(5000))
}
/// Suggestions below this chance aren't shown.
const SHOW_MIN: f64 = 0.12;
/// Apps at least this likely to be next are read into memory ahead of time.
const WARM_MIN: f64 = 0.3;
/// ...at most once per this long each.
const WARM_AGAIN: u64 = 30 * 60;
/// The assistant's app id: warming it means loading the AI model.
const ASSISTANT: &str = "nexa-ai";

fn now_secs() -> u64 {
    // NEXA_PATTERNS_NOW: tests only (a fixed clock).
    std::env::var("NEXA_PATTERNS_NOW").ok().and_then(|v| v.parse().ok())
        .unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into())
}

fn data_dir(home: &Path) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/share")).join("nexa-patterns")
}

fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("NEXA_PATTERNS_SOCKET") {
        return p.into();
    }
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
        // SAFETY: getuid can't fail.
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    run.join("nexa-patterns.sock")
}

struct Pending {
    app: String,
    pid: i32,
    since: Instant,
    recorded: bool,
}

struct Daemon {
    home: PathBuf,
    data: PathBuf,
    cfg_path: PathBuf,
    cfg: config::Config,
    cfg_mtime: Option<SystemTime>,
    base: Model,
    records: Vec<Record>,
    live: Model,
    proc_root: PathBuf,
    sys: PathBuf,
    pending: Option<Pending>,
    launched: HashMap<String, HashSet<i32>>,
    /// app -> (when, bytes) of the last read-ahead.
    warmed: Arc<Mutex<HashMap<String, (u64, u64)>>>,
    /// The maps helper (see `maps_helper`): Landlock stops this confined
    /// process from reading other processes' /proc/<pid>/maps.
    capturer: Option<(std::process::ChildStdin, BufReader<std::process::ChildStdout>)>,
}

impl Daemon {
    fn open(home: PathBuf) -> Daemon {
        let data = data_dir(&home);
        let cfg_path = config::path(&home);
        let (cfg, cfg_mtime) = config::load(&cfg_path);
        let base = fs::read_to_string(data.join("base.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let records = journal::read(&data.join("journal.bin"));
        let var = |k: &str, d: &str| PathBuf::from(std::env::var_os(k).unwrap_or_else(|| d.into()));
        let mut d = Daemon { home, data, cfg_path, cfg, cfg_mtime, base, records, live: Model::default(),
                             proc_root: var("NEXA_PATTERNS_PROC", "/proc"), sys: var("NEXA_PATTERNS_SYS", "/sys"),
                             pending: None, launched: HashMap::new(), warmed: Arc::new(Mutex::new(HashMap::new())), capturer: None };
        d.prune(now_secs());
        d
    }

    fn rebuild(&mut self) {
        let mut m = self.base.clone();
        for r in &self.records {
            m.apply(r);
        }
        self.live = m;
    }

    fn save_base(&self) {
        let _ = fs::create_dir_all(&self.data);
        if let Ok(t) = serde_json::to_string(&self.base) {
            let tmp = self.data.join("base.json.tmp");
            if fs::write(&tmp, t).is_ok() {
                let _ = fs::rename(&tmp, self.data.join("base.json"));
            }
        }
    }

    /// Fold records older than 14 days into the base model's counts.
    fn prune(&mut self, now: u64) {
        let cut = now.saturating_sub(KEEP_RAW);
        let (old, keep): (Vec<Record>, Vec<Record>) = self.records.drain(..).partition(|r| r.ts < cut);
        self.records = keep;
        if !old.is_empty() {
            for r in &old {
                self.base.apply(r);
            }
            self.save_base();
            let _ = journal::write(&self.data.join("journal.bin"), &self.records);
        }
        self.rebuild();
    }

    fn reload_config(&mut self) {
        let (cfg, mtime) = config::load(&self.cfg_path);
        self.cfg = cfg;
        self.cfg_mtime = mtime;
    }

    fn record(&mut self, kind: Kind, app: &str) {
        if !self.cfg.enabled || self.cfg.excluded_apps.iter().any(|a| a == app) {
            return;
        }
        let ts = now_secs();
        let r = Record { ts, kind, app: app.to_string(), place: context::place(&self.proc_root), plugged: context::plugged(&self.sys) };
        let _ = fs::create_dir_all(&self.data);
        if journal::append(&self.data.join("journal.bin"), &r).is_ok() {
            self.live.apply(&r);
            self.records.push(r);
        }
    }

    fn on_focus(&mut self, f: compositor::Focus) {
        self.pending = (!f.private).then(|| Pending { app: f.app, pid: f.pid, since: Instant::now(), recorded: false });
    }

    fn tick(&mut self) {
        let due = self.pending.as_ref().map_or(false, |p| !p.recorded && p.since.elapsed() >= dwell());
        if due {
            let (app, pid) = {
                let p = self.pending.as_mut().unwrap();
                p.recorded = true;
                (p.app.clone(), p.pid)
            };
            let first = self.launched.entry(app.clone()).or_default().insert(pid);
            self.record(if first { Kind::Launch } else { Kind::Switch }, &app);
            // Note what it loaded, for faster starts later (system files only).
            let dir = self.data.join("apps");
            if self.cfg.enabled && self.cfg.preload && pid > 0 && warm::needs_capture(&dir, &app) {
                let files = self.capture(pid);
                warm::save(&dir, &app, &files);
            }
        }
    }

    /// What a process has loaded, asked of the unconfined maps helper.
    fn capture(&mut self, pid: i32) -> Vec<PathBuf> {
        let Some((stdin, stdout)) = self.capturer.as_mut() else { return warm::capture(&self.proc_root, pid) };
        let mut line = String::new();
        if writeln!(stdin, "{pid}").is_err() || stdout.read_line(&mut line).is_err() {
            self.capturer = None;
            return vec![];
        }
        serde_json::from_str::<Vec<String>>(&line).unwrap_or_default().into_iter().map(PathBuf::from).collect()
    }

    fn running(&self, app: &str) -> bool {
        self.launched.get(app).map_or(false, |pids| pids.iter().any(|p| self.proc_root.join(p.to_string()).exists()))
    }

    /// Battery charge, or None on mains power / no battery.
    fn battery(&self) -> Option<u64> {
        if context::plugged(&self.sys) {
            return None;
        }
        fs::read_dir(self.sys.join("class/power_supply")).into_iter().flatten().flatten()
            .find(|e| fs::read_to_string(e.path().join("type")).map(|t| t.trim() == "Battery").unwrap_or(false))
            .and_then(|e| fs::read_to_string(e.path().join("capacity")).ok()).and_then(|c| c.trim().parse().ok())
    }

    /// Read one app into memory now (or, for the assistant, load the AI
    /// model). Returns what was done, in words.
    fn warm(&self, app: &str, now: u64) -> Result<String, String> {
        if app == ASSISTANT {
            if self.battery().map_or(false, |b| b < 50) {
                return Err("battery below 50%: not loading the AI ahead of time".into());
            }
            warm_mind()?;
            self.warmed.lock().unwrap().insert(app.to_string(), (now, 0));
            return Ok("asked Nexa Mind to load its model".into());
        }
        if self.battery().map_or(false, |b| b < 30) {
            return Err("battery below 30%: no reading ahead".into());
        }
        let files = warm::load(&self.data.join("apps"), app);
        if files.is_empty() {
            return Err(format!("nothing known yet about what {app} loads"));
        }
        let want = warm::size(&files);
        if !nexa_sys::preload::memory_to_spare(&self.proc_root, want) {
            return Err("not enough free memory to read ahead".into());
        }
        self.warmed.lock().unwrap().insert(app.to_string(), (now, want));
        let (warmed, app_s) = (self.warmed.clone(), app.to_string());
        std::thread::spawn(move || {
            let done = warm::preload(&files);
            warmed.lock().unwrap().insert(app_s, (now, done));
        });
        Ok(format!("reading {} MB of {app} ahead", want >> 20))
    }

    /// Every minute: get the likely next apps ready.
    fn warm_likely(&self, now: u64) {
        if !self.cfg.enabled || !self.cfg.preload || !self.live.ready() {
            return;
        }
        let ctx = self.ctx(now);
        for p in self.live.predict(&ctx, now, 3, &self.cfg.never_suggest).into_iter().filter(|p| p.p >= WARM_MIN) {
            let recent = self.warmed.lock().unwrap().get(&p.app).map_or(false, |(t, _)| now.saturating_sub(*t) < WARM_AGAIN);
            if !recent && !self.running(&p.app) {
                let _ = self.warm(&p.app, now);
            }
        }
    }

    fn ctx(&self, now: u64) -> context::Ctx {
        context::now(&self.proc_root, &self.sys, now)
    }

    fn suggestions(&self, now: u64, k: usize) -> Vec<Value> {
        let ctx = self.ctx(now);
        self.live.predict(&ctx, now, k, &self.cfg.never_suggest).into_iter().filter(|p| p.p >= SHOW_MIN).map(|p| {
            let why = model::explain(&self.records, &p.app, self.live.previous(), now);
            json!({"app": p.app, "p": (p.p * 100.0).round() / 100.0, "why": why.words(&p.app), "why_parts": why})
        }).collect()
    }

    fn handle(&mut self, req: &Value) -> Result<Value, String> {
        let now = now_secs();
        let app_arg = || req.get("app").and_then(Value::as_str).filter(|a| !a.is_empty() && a.len() < 256)
            .map(str::to_string).ok_or("missing 'app'".to_string());
        match req.get("op").and_then(Value::as_str).ok_or("missing 'op'")? {
            "status" => Ok(json!({
                "enabled": self.cfg.enabled, "ready": self.live.ready(), "records": self.records.len(),
                "days": self.live.last_ts.saturating_sub(self.live.first_ts) / 86400, "apps": self.live.apps().len(),
                "weights": self.live.weights, "bias": self.live.bias, "preload": self.cfg.preload,
                "warmed": self.warmed.lock().unwrap().iter().map(|(a, (t, b))| json!({"app": a, "at": t, "bytes": b})).collect::<Vec<_>>(),
            })),
            "warm" => {
                let app = app_arg()?;
                self.warm(&app, now).map(|done| json!({"done": done}))
            }
            "card" => {
                let hidden = self.cfg.hidden_until > now;
                let on = self.cfg.enabled && self.cfg.desktop_card && !hidden;
                let ready = self.live.ready();
                let apps = if on && ready { self.suggestions(now, 3) } else { vec![] };
                let file = if on && ready { recent::suggest(&recent::list(&self.home), now) } else { None };
                Ok(json!({"enabled": self.cfg.enabled, "show": on, "hidden": hidden, "learning": self.cfg.enabled && !ready,
                          "position": self.cfg.card_position, "apps": apps,
                          "file": file.map(|f| json!({"path": f.path, "app": f.app}))}))
            }
            "predict" => {
                let k = req.get("k").and_then(Value::as_u64).unwrap_or(3).min(20) as usize;
                let on = self.cfg.enabled && self.live.ready();
                Ok(json!({"ready": self.live.ready(), "apps": if on { self.suggestions(now, k) } else { vec![] }}))
            }
            "why" => {
                let app = app_arg()?;
                let why = model::explain(&self.records, &app, self.live.previous(), now);
                Ok(json!({"app": app, "why": why.words(&app), "why_parts": why}))
            }
            "feedback" => {
                let app = app_arg()?;
                let kind = match req.get("action").and_then(Value::as_str) {
                    Some("used") => Kind::Used,
                    Some("dismissed") => Kind::Dismiss,
                    _ => return Err("action is 'used' or 'dismissed'".into()),
                };
                self.record(kind, &app);
                Ok(json!({"done": true}))
            }
            "forget" => {
                if req.get("all").and_then(Value::as_bool) == Some(true) {
                    self.records.clear();
                    self.base = Model::default();
                    let _ = fs::remove_file(self.data.join("journal.bin"));
                    let _ = fs::remove_file(self.data.join("base.json"));
                    self.launched.clear();
                } else if let Some(since) = req.get("since").and_then(Value::as_u64) {
                    // Recent records are only ever in the journal (the base holds 14+ days old).
                    self.records.retain(|r| r.ts < since);
                    let _ = journal::write(&self.data.join("journal.bin"), &self.records);
                } else {
                    let app = app_arg()?;
                    self.records.retain(|r| r.app != app);
                    self.base.forget_app(&app);
                    self.save_base();
                    let _ = journal::write(&self.data.join("journal.bin"), &self.records);
                }
                self.rebuild();
                Ok(json!({"done": true, "records": self.records.len()}))
            }
            "apps" => {
                let mut v: Vec<Value> = self.live.apps().into_iter().map(|a| json!({
                    "app": a, "times": self.live.seen.get(&a), "last_used": self.live.last_used.get(&a),
                    "excluded": self.cfg.excluded_apps.contains(&a), "never_suggest": self.cfg.never_suggest.contains(&a),
                })).collect();
                v.sort_by(|a, b| b["times"].as_u64().cmp(&a["times"].as_u64()));
                Ok(json!({"apps": v}))
            }
            "export" => Ok(json!({
                "records": self.records.iter().map(|r| json!({"time": r.ts, "kind": format!("{:?}", r.kind).to_lowercase(),
                    "app": r.app, "place": r.place, "plugged": r.plugged})).collect::<Vec<_>>(),
                "learned_apps": self.live.apps(),
                "note": "Older history is kept only as fading counts per app, time of day, place and previous app.",
            })),
            "reload" => {
                self.reload_config();
                Ok(json!({"enabled": self.cfg.enabled}))
            }
            other => Err(format!("unknown op '{other}'")),
        }
    }
}

fn client(d: &mut Daemon, stream: UnixStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    // Only you: the socket sits in your runtime folder, and this checks again.
    // SAFETY: getuid can't fail.
    if peer::of(&stream).ok().map(|p| p.uid) != Some(unsafe { libc::getuid() }) {
        return;
    }
    let mut line = String::new();
    let _ = BufReader::new((&stream).take(65536)).read_line(&mut line);
    let reply = match serde_json::from_str::<Value>(&line).map_err(|e| format!("bad request: {e}")).and_then(|r| d.handle(&r)) {
        Ok(v) => json!({"ok": true, "result": v}),
        Err(e) => json!({"ok": false, "error": e}),
    };
    let _ = writeln!(&stream, "{reply}");
}

fn confine(d: &Daemon) {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let _ = fs::create_dir_all(&d.data);
    if let Some(dir) = d.cfg_path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let mut rules = vec![
        (s(&d.data), sandbox::READ_WRITE),
        (s(d.cfg_path.parent().unwrap_or(Path::new("/nonexistent"))), sandbox::READ),
        (s(&d.proc_root), sandbox::READ),
        // Faster starts read the system's own programs and libraries ahead.
        ("/usr".into(), sandbox::READ),
        ("/lib".into(), sandbox::READ),
        ("/opt".into(), sandbox::READ),
        ("/bin".into(), sandbox::READ),
        ("/sbin".into(), sandbox::READ),
        ("/etc/fonts".into(), sandbox::READ),
        (s(&d.sys.join("class/power_supply")), sandbox::READ),
        (s(&d.sys.join("devices")), sandbox::READ),
        ("/etc/localtime".into(), sandbox::READ),
        ("/usr/share/zoneinfo".into(), sandbox::READ),
    ];
    let xbel = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| d.home.join(".local/share")).join("recently-used.xbel");
    rules.push((s(&xbel), sandbox::READ));
    if let Err(e) = sandbox::restrict(&sandbox::Rules { fs: rules, connect_ports: vec![], any_network: false }) {
        eprintln!("nexa-patternd: couldn't confine myself: {e}");
        std::process::exit(1);
    }
}

fn serve() -> ! {
    let socket = socket_path();
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).unwrap_or_else(|e| {
        eprintln!("nexa-patternd: can't listen on {}: {e}", socket.display());
        std::process::exit(1);
    });
    let _ = fs::set_permissions(&socket, fs::Permissions::from_mode(0o600));
    listener.set_nonblocking(true).expect("nonblocking listener");
    let mut d = Daemon::open(home());
    // Started before confining, so it isn't: Landlock keeps a confined process
    // from reading other processes' /proc/<pid>/maps (a ptrace-style check).
    // It answers only "pid -> the system files it has loaded".
    d.capturer = std::env::current_exe().ok().and_then(|exe| {
        std::process::Command::new(exe).arg("maps-helper").stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped()).spawn().ok()
    }).and_then(|mut c| Some((c.stdin.take()?, BufReader::new(c.stdout.take()?))));
    confine(&d);

    // The compositor watcher (started after confining: threads inherit it).
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        let send = tx.clone();
        if let Err(e) = compositor::watch(&mut |f| { let _ = send.send(f); }) {
            eprintln!("nexa-patternd: {e}; retrying");
        }
        std::thread::sleep(Duration::from_secs(3));
    });

    let (mut last_cfg_check, mut last_prune, mut last_warm) = (Instant::now(), Instant::now(), Instant::now());
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(f) => d.on_focus(f),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => std::process::exit(1),
        }
        while let Ok((s, _)) = listener.accept() {
            client(&mut d, s);
        }
        d.tick();
        if last_cfg_check.elapsed() > Duration::from_secs(5) {
            last_cfg_check = Instant::now();
            if fs::metadata(&d.cfg_path).and_then(|m| m.modified()).ok() != d.cfg_mtime {
                d.reload_config();
            }
        }
        if last_warm.elapsed() > Duration::from_secs(60) {
            last_warm = Instant::now();
            d.warm_likely(now_secs());
        }
        if last_prune.elapsed() > Duration::from_secs(6 * 3600) {
            last_prune = Instant::now();
            d.prune(now_secs());
        }
    }
}

/// Ask Nexa Mind (nexa-mindd) to load its model now.
fn warm_mind() -> Result<(), String> {
    let path = std::env::var_os("NEXA_PATTERNS_MIND_SOCKET").map(PathBuf::from).unwrap_or_else(|| "/run/nexa-mind/mind.sock".into());
    let mut s = UnixStream::connect(&path).map_err(|e| format!("Nexa Mind isn't running: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    s.write_all(b"POST /v1/warm HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut status = String::new();
    BufReader::new(s).read_line(&mut status).map_err(|e| e.to_string())?;
    if status.contains(" 202 ") || status.contains(" 200 ") { Ok(()) } else { Err(format!("Nexa Mind said: {}", status.trim())) }
}

fn ask(req: Value) -> Result<Value, String> {
    let socket = socket_path();
    let mut s = UnixStream::connect(&socket).map_err(|e| format!("nexa-patternd isn't running ({}): {e}", socket.display()))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    writeln!(s, "{req}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| e.to_string())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    let req = match arg(1) {
        Some("serve") => serve(),
        Some("maps-helper") => {
            // One pid per line in, one JSON list of system files out.
            let proc_root = PathBuf::from(std::env::var_os("NEXA_PATTERNS_PROC").unwrap_or_else(|| "/proc".into()));
            let mut out = std::io::stdout();
            for line in std::io::stdin().lines() {
                let Ok(line) = line else { break };
                let files: Vec<String> = line.trim().parse::<i32>().ok().filter(|p| *p > 0)
                    .map(|pid| warm::capture(&proc_root, pid)).unwrap_or_default()
                    .into_iter().filter_map(|p| p.to_str().map(str::to_string)).collect();
                if writeln!(out, "{}", serde_json::to_string(&files).unwrap_or_else(|_| "[]".into())).is_err() || out.flush().is_err() {
                    break;
                }
            }
            return;
        }
        Some("replay") => {
            let records = match (arg(2), arg(3)) {
                (Some("--synthetic"), days) => replay::synthetic(days.and_then(|d| d.parse().ok()).unwrap_or(35), 42),
                (Some(file), _) => journal::read(Path::new(file)),
                _ => journal::read(&data_dir(&home()).join("journal.bin")),
            };
            println!("{}", serde_json::to_string_pretty(&replay::evaluate(&records, 7)).unwrap_or_default());
            return;
        }
        Some("synthetic") => {
            let (Some(days), Some(file)) = (arg(2).and_then(|d| d.parse().ok()), arg(3)) else {
                eprintln!("usage: nexa-patternd synthetic DAYS FILE");
                std::process::exit(2);
            };
            // Shifted so the synthetic month ends today.
            let mut recs = replay::synthetic(days, 42);
            let shift = now_secs().saturating_sub(recs.last().map_or(0, |r| r.ts)) / 86400 * 86400;
            for r in &mut recs {
                r.ts += shift;
            }
            journal::write(Path::new(file), &recs).unwrap_or_else(|e| eprintln!("{e}"));
            return;
        }
        Some(op @ ("card" | "status" | "apps" | "export")) => json!({"op": op}),
        _ => {
            eprintln!("usage: nexa-patternd serve | card | status | apps | export | replay [--synthetic DAYS | FILE] | synthetic DAYS FILE");
            std::process::exit(2);
        }
    };
    match ask(req) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            std::process::exit(if v["ok"] == true { 0 } else { 1 });
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
