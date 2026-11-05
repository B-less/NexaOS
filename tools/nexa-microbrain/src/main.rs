//! nexa-microbrain: Nexa MicroBrain, NexaOS's small intelligence service.
//! No AI model: rules, moving averages, trends and counts, in one process
//! per user, event-driven (the kernel's pressure triggers wake it; a slow
//! sample otherwise). See TODOnexa_inter.md and docs/MICROBRAIN.md.
//!
//!   nexa-microbrain serve              the service (starts with the desktop session)
//!   nexa-microbrain status             what it sees and what it allows right now
//!   nexa-microbrain resources          memory, processor, disk, forecasts
//!   nexa-microbrain mode [MODE]        AdaptivePulse: auto, balanced, low-resource,
//!                                      battery-saver, performance
//!   nexa-microbrain agents | decisions | jobs
//!   nexa-microbrain config [FEATURE on|off]
//!   nexa-microbrain ask WORDS...        ZeroSearch: what those words would do
//!   nexa-microbrain answer slow|battery
//!   nexa-microbrain why slow|crash|memory|network|stopped   Why: the evidence
//!   nexa-microbrain rewind [ID]        Rewind: settings changes; go back to before ID
//!   nexa-microbrain workspaces | suggestions
//!
//! Talks to Settings over $XDG_RUNTIME_DIR/nexa-microbrain.sock (0600,
//! this user only, checked by the kernel's peer credentials), one JSON
//! line each way. Anything that needs root goes through nexa-brokerd.

mod agents;
mod apps;
mod config;
mod habit;
mod magic;
mod pulse;
mod resources;
mod rewind;
mod scheduler;
mod features;
mod search;
mod store;
mod why;
mod workspace;

use agents::{Coordinator, Event, Notice};
use config::{Config, Profile};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use store::{Doc, Store};

/// Rewind's history, shared by its watcher thread and the requests.
pub static REWIND: std::sync::OnceLock<Arc<std::sync::Mutex<rewind::Rewind>>> = std::sync::OnceLock::new();

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn run_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        // SAFETY: getuid has no failure mode.
        PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
    })
}

fn sock_path() -> PathBuf {
    run_dir().join("nexa-microbrain.sock")
}

/// What reaches the coordinator thread: events, and requests from Settings.
enum Msg {
    Event(Event),
    Request(Value, mpsc::Sender<Value>),
}

/// The decisions kept across restarts (the newest few hundred).
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Decisions(Vec<agents::Decision>);

impl Doc for Decisions {
    const NAME: &'static str = "decisions";
    const VERSION: u32 = 1;
}

impl<'de> serde::Deserialize<'de> for agents::Decision {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Raw {
            at: u64,
            agent: String,
            what: String,
            why: String,
        }
        let r = Raw::deserialize(d)?;
        let agent: &'static str = match r.agent.as_str() {
            "performance" => "performance",
            "maintenance" => "maintenance",
            "learning" => "learning",
            "security" => "security",
            "file" => "file",
            _ => "other",
        };
        Ok(agents::Decision { at: r.at, agent, what: r.what, why: r.why })
    }
}

fn notify(n: &Notice) {
    // Signals shows it; nothing happens if there's no notification service.
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "Nexa MicroBrain", "-i", "nexa-pulse", &n.title, &n.body])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut c| c.wait());
}

/// MicroBrain's own footprint, from the kernel: RSS, PSS (MB) and CPU seconds.
fn self_usage() -> Value {
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    let field = |k: &str| rollup.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok());
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = stat.rsplit(')').next().unwrap_or("");
    let f: Vec<&str> = after.split_whitespace().collect();
    // SAFETY: sysconf has no failure mode that matters here.
    let tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let cpu = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    json!({"rss_mb": field("Rss:").map(|k| k / 1024.0), "pss_mb": field("Pss:").map(|k| k / 1024.0),
           "cpu_seconds": (cpu(11) + cpu(12)) / tck, "threads": f.get(17).and_then(|v| v.parse::<u64>().ok())})
}

/// A memory-pressure trigger: the kernel wakes us when tasks stall for
/// memory 100 ms within 2 s (unprivileged triggers need a 2 s window).
fn psi_trigger() -> Option<std::fs::File> {
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open("/proc/pressure/memory").ok()?;
    f.write_all(b"some 100000 2000000\0").ok()?;
    Some(f)
}

fn sampler(tx: SyncSender<Msg>, interval: Arc<AtomicU64>) {
    let paths = resources::Paths::real();
    let trigger = if paths.proc_root == std::path::Path::new("/proc") { psi_trigger() } else { None };
    // NEXA_MICROBRAIN_INTERVAL: a fixed sample interval (tests).
    let fixed = std::env::var("NEXA_MICROBRAIN_INTERVAL").ok().and_then(|v| v.parse::<u64>().ok());
    let mut prev = None;
    // The first sample at once (Settings asks for numbers right away), the
    // rest when woken or when the interval is up.
    let mut first = true;
    loop {
        if first {
            first = false;
            let s = resources::sample(&paths, &mut prev, now());
            if tx.try_send(Msg::Event(Event::Sample(s))).is_err() {
                return;
            }
        }
        let wait_ms = (fixed.unwrap_or_else(|| interval.load(Ordering::Relaxed).max(2)).max(1) * 1000) as i32;
        match &trigger {
            Some(f) => {
                let mut p = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLPRI, revents: 0 };
                // SAFETY: one valid pollfd.
                unsafe { libc::poll(&mut p, 1, wait_ms) };
            }
            None => std::thread::sleep(Duration::from_millis(wait_ms as u64)),
        }
        let s = resources::sample(&paths, &mut prev, now());
        // A full queue means the coordinator is behind: drop this sample.
        if let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(Msg::Event(Event::Sample(s))) {
            return;
        }
    }
}

fn run_job(name: &str, store: &Store, log: &[agents::Decision]) -> Result<String, String> {
    match name {
        "trim-history" => {
            store.save(&Decisions(log.to_vec())).map_err(|e| e.to_string())?;
            // Damaged files set aside more than 30 days ago go.
            let mut removed = 0;
            for e in std::fs::read_dir(&store.dir).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let old = e.metadata().and_then(|m| m.modified()).ok()
                    .and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(30 * 86400));
                if name.contains(".broken-") && old && std::fs::remove_file(e.path()).is_ok() {
                    removed += 1;
                }
            }
            Ok(format!("history kept to {} decisions, {removed} old damaged files removed", log.len()))
        }
        _ => Err(format!("no job called {name}")),
    }
}

fn reply_status(c: &Coordinator, cfg: &Config, store: &Store) -> Value {
    json!({
        "ok": true, "enabled": cfg.enabled, "profile": c.profile, "mode": c.mode, "budget": c.budget,
        "memory": c.monitor.mem_level, "cpu": c.monitor.cpu_level, "disk_low": c.monitor.disk_low,
        "sample": c.monitor.last, "forecast": {"memory": c.monitor.memory_forecast(), "disk": c.monitor.disk_forecast()},
        "jobs": {"running": c.scheduler.running().collect::<Vec<_>>(), "waiting": c.scheduler.waiting().collect::<Vec<_>>(),
                 "refused": c.scheduler.refused},
        "self": self_usage(), "store_bytes": store.size(), "features": cfg.features,
    })
}

fn handle_request(req: &Value, c: &mut Coordinator, cfg: &mut Config, store: &Store) -> (Value, Vec<Notice>) {
    let cmd = req["cmd"].as_str().unwrap_or("");
    match cmd {
        "status" | "resources" => (reply_status(c, cfg, store), vec![]),
        "agents" => (json!({"ok": true, "agents": c.agents_status()}), vec![]),
        "decisions" => {
            let n = req["limit"].as_u64().unwrap_or(50) as usize;
            let list: Vec<&agents::Decision> = c.log.iter().rev().take(n).collect();
            (json!({"ok": true, "decisions": list}), vec![])
        }
        "jobs" => {
            if let Some(id) = req["cancel"].as_u64() {
                return (json!({"ok": c.scheduler.cancel(id)}), vec![]);
            }
            (json!({"ok": true, "running": c.scheduler.running().collect::<Vec<_>>(),
                    "waiting": c.scheduler.waiting().collect::<Vec<_>>()}), vec![])
        }
        // Privacy: everything MicroBrain remembers about what it decided.
        "forget-history" => {
            c.log.clear();
            (json!({"ok": store.forget::<Decisions>().is_ok()}), vec![])
        }
        "mode" => match req["mode"].as_str() {
            None => (json!({"ok": true, "mode": c.mode, "budget": c.budget}), vec![]),
            Some(m) => match pulse::Mode::parse(m) {
                Some(mode) => {
                    cfg.mode = mode;
                    let saved = store.save(cfg);
                    let notices = c.set_mode(mode, now());
                    (json!({"ok": saved.is_ok(), "mode": mode, "budget": c.budget}), notices)
                }
                None => (json!({"ok": false, "error": "Modes: auto, balanced, low-resource, battery-saver, performance."}), vec![]),
            },
        },
        "config" => {
            if let (Some(feature), Some(on)) = (req["feature"].as_str(), req["on"].as_bool()) {
                let mut f = serde_json::to_value(&cfg.features).unwrap_or_default();
                if f.get(feature).is_none() {
                    return (json!({"ok": false, "error": format!("No feature called {feature}.")}), vec![]);
                }
                f[feature] = json!(on);
                cfg.features = serde_json::from_value(f).unwrap_or_default();
            }
            if let Some(on) = req["enabled"].as_bool() {
                cfg.enabled = on;
            }
            if let Some(p) = req["profile"].as_str() {
                if ["auto", "2gb", "4gb", "8gb"].contains(&p) {
                    cfg.profile = p.into();
                }
            }
            let saved = store.save(cfg);
            (json!({"ok": saved.is_ok(), "config": cfg}), vec![])
        }
        _ => (json!({"ok": false, "error": format!("unknown command {cmd:?}")}), vec![]),
    }
}

/// The installed apps, rebuilt at most once a minute (Nexus asks per keystroke).
pub struct AppCache {
    apps: std::collections::BTreeMap<String, apps::App>,
    built: u64,
}

impl AppCache {
    pub fn get(&mut self, home: &std::path::Path) -> &std::collections::BTreeMap<String, apps::App> {
        if self.built == 0 || now().saturating_sub(self.built) >= 60 {
            self.apps = apps::index(home);
            self.built = now().max(1);
        }
        &self.apps
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into())
}

fn coordinator_loop(rx: Receiver<Msg>, mut c: Coordinator, mut cfg: Config, store: Store, interval: Arc<AtomicU64>,
                    job_tx: SyncSender<Msg>) {
    let saved: Decisions = store.load();
    c.log.extend(saved.0);
    let mut cache = AppCache { apps: Default::default(), built: 0 };
    for msg in rx {
        let (notices, reply) = match msg {
            Msg::Event(ev) => (if cfg.enabled { c.handle(ev, now()) } else { Vec::new() }, None),
            Msg::Request(req, tx) => {
                let (r, n) = match features::handle(&req, &mut c, &mut cfg, &store, &mut cache) {
                    Some(answer) => answer,
                    None => handle_request(&req, &mut c, &mut cfg, &store),
                };
                (n, Some((tx, r)))
            }
        };
        if let Some((tx, r)) = reply {
            let _ = tx.send(r);
        }
        interval.store(c.budget.sample_secs, Ordering::Relaxed);
        if cfg.notify {
            for n in &notices {
                notify(n);
            }
        }
        // Start what the budget allows; each job reports back as an event.
        for job in c.scheduler.next(&c.budget) {
            let log: Vec<agents::Decision> = c.log.iter().cloned().collect();
            let dir = store.dir.clone();
            let tx = job_tx.clone();
            std::thread::spawn(move || {
                let s = Store { dir };
                let (ok, note) = match run_job(&job.name, &s, &log) {
                    Ok(n) => (true, n),
                    Err(e) => (false, e),
                };
                let _ = tx.send(Msg::Event(Event::JobDone { id: job.id, name: job.name, ok, note }));
            });
        }
    }
}

fn control_conn(stream: UnixStream, tx: SyncSender<Msg>) {
    // SAFETY: getuid has no failure mode.
    let me = unsafe { libc::getuid() };
    if nexa_sys::peer::of(&stream).map(|p| p.uid != me).unwrap_or(true) {
        return;
    }
    stream.set_read_timeout(Some(Duration::from_secs(300))).ok();
    let Ok(mut out) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        // Requests are small; a huge line is someone misbehaving.
        match (&mut reader).take(64 * 1024).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) if req.is_object() => {
                let (rtx, rrx) = mpsc::channel();
                if tx.try_send(Msg::Request(req, rtx)).is_err() {
                    json!({"ok": false, "error": "MicroBrain is busy; try again in a moment."})
                } else {
                    rrx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| json!({"ok": false, "error": "no answer in time"}))
                }
            }
            _ => json!({"ok": false, "error": "not a request"}),
        };
        if writeln!(out, "{reply}").is_err() {
            return;
        }
    }
}


fn confine(store_dir: &std::path::Path) {
    use nexa_sys::sandbox::{self, READ, READ_EXEC, READ_WRITE, WRITE_FILE};
    let mut rules: Vec<(String, u64)> = ["/usr", "/lib", "/bin"].iter().map(|p| (p.to_string(), READ_EXEC)).collect();
    for p in ["/etc", "/proc", "/sys", "/dev/urandom"] {
        rules.push((p.into(), READ));
    }
    // PSI triggers are set up by writing to /proc/pressure/memory (done before this).
    rules.push(("/dev/null".into(), READ | WRITE_FILE));
    rules.push((store_dir.to_string_lossy().into_owned(), READ_WRITE));
    rules.push((run_dir().to_string_lossy().into_owned(), READ_WRITE));
    if let Some(home) = std::env::var_os("HOME") {
        rules.push((home.to_string_lossy().into_owned(), READ));
        // Rewind writes settings back: only the settings folders, never the rest of home.
        for d in [".config/nexa", ".config/sway/config.d"] {
            let p = std::path::Path::new(&home).join(d);
            if p.is_dir() {
                rules.push((p.to_string_lossy().into_owned(), READ_WRITE));
            }
        }
    }
    if let Err(e) = sandbox::restrict(&sandbox::Rules { fs: rules, connect_ports: vec![], any_network: false }) {
        eprintln!("nexa-microbrain: Landlock: {e} (running unconfined)");
    }
}

fn serve() -> io::Result<()> {
    let store = Store::open(store::default_dir())?;
    let cfg: Config = store.load();
    let mem = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let total_mb = mem.lines().find(|l| l.starts_with("MemTotal:")).and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) / 1024;
    let profile = Profile::pick(&cfg.profile, total_mb);
    let sock = sock_path();
    std::fs::remove_file(&sock).ok();
    let listener = UnixListener::bind(&sock)?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;
    // SAFETY: getuid has no failure mode.
    let uid = unsafe { libc::getuid() };
    let agents: Vec<Box<dyn agents::Agent>> = vec![Box::<agents::Performance>::default(), Box::<agents::Maintenance>::default(),
                                                    Box::new(features::Learning::new(store.dir.clone(), home()))];
    let coordinator = Coordinator::new(profile, cfg.mode, agents, resources::Paths::real().proc_root, uid);
    let interval = Arc::new(AtomicU64::new(coordinator.budget.sample_secs));
    let (tx, rx) = mpsc::sync_channel::<Msg>(256);
    {
        let (tx, interval) = (tx.clone(), interval.clone());
        std::thread::spawn(move || sampler(tx, interval));
    }
    {
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(60));
            if let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(Msg::Event(Event::Minute)) {
                return;
            }
        });
    }
    {
        let r = Arc::new(std::sync::Mutex::new(rewind::Rewind::new(home(), &store.dir)));
        let _ = REWIND.set(r.clone());
        std::thread::spawn(move || rewind::watch(r));
    }
    confine(&store.dir);
    eprintln!("nexa-microbrain: profile {profile:?}, mode {:?}", cfg.mode);
    {
        let job_tx = tx.clone();
        std::thread::spawn(move || coordinator_loop(rx, coordinator, cfg, store, interval, job_tx));
    }
    for conn in listener.incoming().flatten() {
        let tx = tx.clone();
        std::thread::spawn(move || control_conn(conn, tx));
    }
    Ok(())
}

fn request(req: Value) -> io::Result<Value> {
    let mut s = UnixStream::connect(sock_path()).map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "MicroBrain isn't running."))?;
    writeln!(s, "{req}")?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn print(v: Value) -> io::Result<()> {
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    if v["ok"] == false {
        std::process::exit(1);
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match a.as_slice() {
        ["serve"] => serve(),
        ["status"] => request(json!({"cmd": "status"})).and_then(print),
        ["resources"] => request(json!({"cmd": "resources"})).and_then(print),
        ["agents"] => request(json!({"cmd": "agents"})).and_then(print),
        ["decisions"] => request(json!({"cmd": "decisions"})).and_then(print),
        ["jobs"] => request(json!({"cmd": "jobs"})).and_then(print),
        ["jobs", "cancel", id] => request(json!({"cmd": "jobs", "cancel": id.parse::<u64>().unwrap_or(0)})).and_then(print),
        ["forget-history"] => request(json!({"cmd": "forget-history"})).and_then(print),
        ["ask", words @ ..] if !words.is_empty() => request(json!({"cmd": "ask", "query": words.join(" ")})).and_then(print),
        ["answer", topic] => request(json!({"cmd": "answer", "topic": topic})).and_then(print),
        ["why", topic] => request(json!({"cmd": "why", "topic": topic})).and_then(print),
        ["rewind"] => request(json!({"cmd": "rewind"})).and_then(print),
        ["rewind", id] => request(json!({"cmd": "rewind-to", "id": id.parse::<u64>().unwrap_or(0)})).and_then(print),
        ["workspaces"] => request(json!({"cmd": "workspaces"})).and_then(print),
        ["suggestions"] => request(json!({"cmd": "suggestions"})).and_then(print),
        ["mode"] => request(json!({"cmd": "mode"})).and_then(print),
        ["mode", m] => request(json!({"cmd": "mode", "mode": m})).and_then(print),
        ["config"] => request(json!({"cmd": "config"})).and_then(print),
        ["config", feature, on @ ("on" | "off")] => request(json!({"cmd": "config", "feature": feature, "on": *on == "on"})).and_then(print),
        _ => {
            eprintln!("usage: nexa-microbrain serve | status | resources | mode [MODE] | agents | decisions | jobs [cancel ID] |\n       \
                       config [FEATURE on|off] | ask WORDS... | answer slow|battery | why slow|crash|memory|network|stopped | rewind [ID] | workspaces | suggestions | forget-history");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("nexa-microbrain: {e}");
        std::process::exit(1);
    }
}
