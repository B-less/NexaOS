//! nexa-kerneld: Nexa Mind's link to the kernel. The AI itself stays a
//! normal sandboxed program (a model running inside the kernel would take
//! the whole computer down with it); this daemon gives it the kernel's view
//! and lets it steer the kernel's own controls:
//!
//! * sees: pressure stalls (PSI triggers, pushed by the kernel), every
//!   crash as it happens (netlink process connector), out-of-memory kills
//!   (/dev/kmsg), power drawn (RAPL, battery) and which programs draw it;
//! * steers: a CPU floor for the app in front (utilization clamping), and
//!   the AI's own share of the computer (its cgroup) - yielding while you
//!   work, capped under pressure, free when you're away.
//!
//!   nexa-kerneld serve            the service (root; OpenRC starts it)
//!   nexa-kerneld status|events|energy   ask it (prints JSON)
//!   nexa-kerneld focus-watch      in the desktop session: report focus
//!
//! Socket /run/nexa-kerneld.sock, one JSON line in, one out:
//!   {"op": "status"}  pressure, power, the AI's limit, the app in front
//!   {"op": "events", "limit": 20}  crashes, OOM kills, pressure spikes
//!   {"op": "energy"}  programs by CPU share and estimated watts
//!   {"op": "focus", "pid": N}  the caller's own process N is in front
//! Callers see their own programs' events and energy; administrators
//! (wheel) and root see everyone's. The service confines itself with
//! Landlock once its kernel interfaces are open.

mod focus;
mod kmsg;
mod power;
mod procev;
mod psi;
mod steer;

use nexa_sys::{peer, sandbox};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TICK: Duration = Duration::from_secs(2);
/// Someone is "using the computer" if focus moved within this long.
const ACTIVE_FOR: Duration = Duration::from_secs(180);
const MAX_EVENTS: usize = 300;

struct Paths {
    proc_root: PathBuf,
    sys: PathBuf,
    etc: PathBuf,
    socket: PathBuf,
    kmsg: PathBuf,
}

impl Paths {
    fn from_env() -> Paths {
        let var = |k: &str, d: &str| PathBuf::from(std::env::var_os(k).unwrap_or_else(|| d.into()));
        Paths {
            proc_root: var("NEXA_KERNELD_PROC", "/proc"),
            sys: var("NEXA_KERNELD_SYS", "/sys"),
            etc: var("NEXA_KERNELD_ETC", "/etc"),
            socket: var("NEXA_KERNELD_SOCKET", "/run/nexa-kerneld.sock"),
            kmsg: var("NEXA_KERNELD_KMSG", "/dev/kmsg"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct Event {
    time: u64,
    kind: &'static str,
    pid: i32,
    name: String,
    #[serde(skip)]
    uid: u32,
    text: String,
}

#[derive(Default, Serialize)]
struct Can {
    pressure: bool,
    pressure_triggers: bool,
    process_events: bool,
    oom_log: bool,
    power_meter: bool,
    app_boost: bool,
    ai_cgroup: bool,
}

struct State {
    paths: Paths,
    events: VecDeque<Event>,
    procs: HashMap<i32, (String, u32)>,
    sampler: power::Sampler,
    model: steer::ModelGroup,
    focus: Option<(i32, String, bool)>,
    last_active: Option<Instant>,
    forks: u64,
    forks_per_second: f64,
    last_pressure_event: HashMap<&'static str, Instant>,
    can: Can,
    cpus: usize,
    admins: Vec<String>,
    names: HashMap<u32, String>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The name and owner of a running process.
fn proc_info(proc_root: &Path, pid: i32) -> Option<(String, u32)> {
    let name = fs::read_to_string(proc_root.join(pid.to_string()).join("comm")).ok()?.trim().to_string();
    Some((name, steer::owner(proc_root, pid)?))
}

/// uid -> account name, and the wheel group's members.
fn accounts(etc: &Path) -> (HashMap<u32, String>, Vec<String>) {
    let names = fs::read_to_string(etc.join("passwd")).unwrap_or_default().lines().filter_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        Some((f.get(2)?.parse().ok()?, f.first()?.to_string()))
    }).collect();
    let admins = fs::read_to_string(etc.join("group")).unwrap_or_default().lines()
        .find(|l| l.starts_with("wheel:")).and_then(|l| l.split(':').nth(3))
        .map(|m| m.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    (names, admins)
}

impl State {
    fn new(paths: Paths) -> State {
        let (names, admins) = accounts(&paths.etc);
        let mind_uid = names.iter().find(|(_, n)| n.as_str() == "nexa-mind").map(|(u, _)| *u);
        let procs = fs::read_dir(&paths.proc_root).into_iter().flatten().flatten()
            .filter_map(|e| e.file_name().to_string_lossy().parse::<i32>().ok())
            .filter_map(|pid| proc_info(&paths.proc_root, pid).map(|i| (pid, i))).collect();
        let model = steer::ModelGroup { cgroup_root: paths.sys.join("fs/cgroup"), proc_root: paths.proc_root.clone(), uid: mind_uid, applied: None };
        let sampler = power::Sampler::new(&paths.sys, &paths.proc_root);
        let can = Can {
            pressure: psi::read(&paths.proc_root, "cpu").is_some(),
            power_meter: sampler.has_rapl(),
            app_boost: steer::uclamp_supported(),
            ai_cgroup: model.has_cgroup(),
            ..Default::default()
        };
        // SAFETY: sysconf can't fail for this name.
        let cpus = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) }.max(1) as usize;
        State { paths, events: VecDeque::new(), procs, sampler, model, focus: None, last_active: None, forks: 0,
                forks_per_second: 0.0, last_pressure_event: HashMap::new(), can, cpus, admins, names }
    }

    fn is_admin(&self, uid: u32) -> bool {
        uid == 0 || self.names.get(&uid).map_or(false, |n| self.admins.contains(n))
    }

    fn push(&mut self, kind: &'static str, pid: i32, name: String, uid: u32, text: String) {
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(Event { time: now_secs(), kind, pid, name, uid, text });
    }

    fn on_process(&mut self, e: procev::Event) {
        match e {
            procev::Event::Fork { .. } => self.forks += 1,
            procev::Event::Exec { pid } => {
                if let Some(info) = proc_info(&self.paths.proc_root, pid) {
                    self.procs.insert(pid, info);
                }
            }
            procev::Event::Exit { pid, signal, .. } => {
                let (name, uid) = self.procs.remove(&pid).unwrap_or_else(|| ("a program".into(), 0));
                if procev::is_crash(signal) {
                    let text = format!("{name} crashed with {}", procev::signal_name(signal));
                    self.push("crash", pid, name, uid, text);
                }
                if self.focus.as_ref().map_or(false, |f| f.0 == pid) {
                    self.focus = None;
                }
            }
            procev::Event::CoreDump { .. } => {}
        }
    }

    fn on_oom(&mut self, k: kmsg::OomKill) {
        let uid = k.uid.or_else(|| self.procs.get(&k.pid).map(|p| p.1)).unwrap_or(0);
        let why = if k.cgroup { "it reached its memory limit" } else { "the computer ran out of memory" };
        let text = format!("The kernel closed {} because {why}", k.name);
        self.push("oom_kill", k.pid, k.name, uid, text);
    }

    fn on_pressure(&mut self, res: &'static str) {
        let now = Instant::now();
        if self.last_pressure_event.get(res).map_or(false, |t| now.duration_since(*t) < Duration::from_secs(60)) {
            return;
        }
        self.last_pressure_event.insert(res, now);
        let p = psi::read(&self.paths.proc_root, res).unwrap_or_default();
        let what = match res { "cpu" => "waiting for the processor", "memory" => "short of memory", _ => "waiting for the disk" };
        self.push("pressure", 0, res.into(), 0, format!("Programs were held up {what} ({:.0}% of the last 10 s)", p.some.avg10));
        self.steer_ai();
    }

    fn active(&self) -> bool {
        self.last_active.map_or(false, |t| t.elapsed() < ACTIVE_FOR)
    }

    fn steer_ai(&mut self) {
        if self.model.uid.is_none() {
            return;
        }
        let cpu = psi::read(&self.paths.proc_root, "cpu").unwrap_or_default();
        let mem = psi::read(&self.paths.proc_root, "memory").unwrap_or_default();
        let limit = steer::decide(&cpu, &mem, self.active());
        if let Err(e) = self.model.apply(limit, self.cpus) {
            eprintln!("nexa-kerneld: couldn't limit the AI: {e}");
        }
    }

    fn tick(&mut self, since: Duration) {
        self.sampler.sample();
        self.forks_per_second = self.forks as f64 / since.as_secs_f64().max(0.001);
        self.forks = 0;
        self.steer_ai();
    }

    fn pressure(&self) -> Value {
        let r = |n| psi::read(&self.paths.proc_root, n).unwrap_or_default();
        let (c, m, i) = (r("cpu"), r("memory"), r("io"));
        json!({"summary": psi::summary(&c, &m, &i), "cpu": c, "memory": m, "io": i})
    }

    fn focus(&mut self, pid: i32, caller: u32) -> Result<Value, String> {
        let owner = steer::owner(&self.paths.proc_root, pid).ok_or("that program isn't running")?;
        if owner != caller && caller != 0 {
            return Err("you can only bring your own programs to the front".into());
        }
        self.last_active = Some(Instant::now());
        let name = proc_info(&self.paths.proc_root, pid).map(|i| i.0).unwrap_or_default();
        if self.focus.as_ref().map(|f| f.0) == Some(pid) {
            return Ok(json!({"pid": pid, "name": name, "boosted": self.focus.as_ref().unwrap().2}));
        }
        if let Some((old, _, true)) = self.focus.take() {
            let _ = steer::floor_process(&self.paths.proc_root, old, 0);
        }
        let boosted = self.can.app_boost && steer::floor_process(&self.paths.proc_root, pid, steer::FOCUS_FLOOR).is_ok();
        self.focus = Some((pid, name.clone(), boosted));
        self.steer_ai();
        Ok(json!({"pid": pid, "name": name, "boosted": boosted}))
    }

    fn handle(&mut self, req: &Value, caller: u32) -> Result<Value, String> {
        let admin = self.is_admin(caller);
        match req.get("op").and_then(Value::as_str).ok_or("missing 'op'")? {
            "status" => {
                let limit = self.model.applied;
                Ok(json!({
                    "pressure": self.pressure(),
                    "power": {"package_watts": self.sampler.package_watts, "battery": self.sampler.battery},
                    "ai": {"limit": limit, "words": limit.map(steer::ModelLimit::words), "you_are_active": self.active()},
                    "focus": self.focus.as_ref().map(|(pid, name, boosted)| json!({"pid": pid, "name": name, "boosted": boosted})),
                    "processes_started_per_second": (self.forks_per_second * 10.0).round() / 10.0,
                    "can": &self.can,
                }))
            }
            "events" => {
                let limit = req.get("limit").and_then(Value::as_u64).unwrap_or(20).min(MAX_EVENTS as u64) as usize;
                let list: Vec<&Event> = self.events.iter().rev().filter(|e| admin || e.uid == caller || e.kind == "pressure")
                    .take(limit).collect();
                let crashes = list.iter().filter(|e| e.kind != "pressure").count();
                Ok(json!({"events": list, "summary": if list.is_empty() {
                    "Nothing has crashed or been closed by the kernel since the computer started watching.".to_string()
                } else { format!("{crashes} crash or memory event(s), newest first.") }}))
            }
            "energy" => {
                let mut others = (0.0, 0.0);
                let mut mine = Vec::new();
                for u in &self.sampler.users {
                    if admin || u.uid == caller {
                        mine.push(json!({"pid": u.pid, "name": u.name, "cpu_percent": u.cpu_percent, "watts": u.watts,
                                         "account": self.names.get(&u.uid)}));
                    } else {
                        others.0 += u.cpu_percent;
                        others.1 += u.watts.unwrap_or(0.0);
                    }
                }
                let measured = self.sampler.package_watts.is_some() || (self.sampler.battery.discharging && self.sampler.battery.watts.is_some());
                let summary = match mine.first() {
                    Some(top) if measured => format!("{} is using the most: about {} W ({}% of the processor).", top["name"].as_str().unwrap_or("?"), top["watts"], top["cpu_percent"]),
                    Some(top) => format!("{} is using the most processor time ({}%); this computer can't measure watts right now.", top["name"].as_str().unwrap_or("?"), top["cpu_percent"]),
                    None => "Nothing is using noticeable power.".to_string(),
                };
                Ok(json!({"summary": summary, "measured": measured, "package_watts": self.sampler.package_watts,
                          "battery": self.sampler.battery, "programs": mine,
                          "other_accounts": {"cpu_percent": (others.0 * 10.0f64).round() / 10.0, "watts": (others.1 * 100.0f64).round() / 100.0}}))
            }
            "focus" => {
                let pid = req.get("pid").and_then(Value::as_i64).filter(|p| *p > 0 && *p < i32::MAX as i64).ok_or("missing 'pid'")? as i32;
                self.focus(pid, caller)
            }
            other => Err(format!("unknown op '{other}'")),
        }
    }
}

fn client(state: &mut State, stream: UnixStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let Ok(who) = peer::of(&stream) else { return };
    let mut line = String::new();
    let _ = BufReader::new((&stream).take(8192)).read_line(&mut line);
    let reply = match serde_json::from_str::<Value>(&line).map_err(|e| format!("bad request: {e}")).and_then(|r| state.handle(&r, who.uid)) {
        Ok(v) => json!({"ok": true, "result": v}),
        Err(e) => json!({"ok": false, "error": e}),
    };
    let _ = writeln!(&stream, "{reply}");
}

fn confine(p: &Paths) {
    let rules = vec![
        (p.proc_root.to_string_lossy().into_owned(), sandbox::READ),
        (p.sys.to_string_lossy().into_owned(), sandbox::READ),
        (p.sys.join("fs/cgroup").to_string_lossy().into_owned(), sandbox::READ_WRITE),
        (p.etc.join("passwd").to_string_lossy().into_owned(), sandbox::READ),
        (p.etc.join("group").to_string_lossy().into_owned(), sandbox::READ),
    ];
    match sandbox::restrict(&sandbox::Rules { fs: rules, connect_ports: vec![], any_network: false }) {
        Ok(0) => eprintln!("nexa-kerneld: this kernel has no Landlock; running unconfined"),
        Ok(_) => {}
        Err(e) => {
            eprintln!("nexa-kerneld: couldn't confine myself: {e}");
            std::process::exit(1);
        }
    }
}

fn serve() -> ! {
    let paths = Paths::from_env();
    let _ = fs::remove_file(&paths.socket);
    let listener = UnixListener::bind(&paths.socket).unwrap_or_else(|e| {
        eprintln!("nexa-kerneld: can't listen on {}: {e}", paths.socket.display());
        std::process::exit(1);
    });
    // Anyone may ask; what they see is decided per caller (SO_PEERCRED).
    let _ = fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o666));
    listener.set_nonblocking(true).expect("nonblocking listener");

    // Open every kernel interface before confining ourselves.
    let triggers: Vec<(&'static str, File)> = [("cpu", 500, 2000), ("memory", 300, 2000), ("io", 500, 2000)].into_iter()
        .filter_map(|(r, stall, window)| psi::trigger(&paths.proc_root, r, stall, window).ok().map(|f| (r, f))).collect();
    let conn = procev::Connector::open().map_err(|e| eprintln!("nexa-kerneld: no process events: {e}")).ok();
    let mut log = kmsg::Kmsg::open(&paths.kmsg).map_err(|e| eprintln!("nexa-kerneld: no kernel log: {e}")).ok();
    let mut state = State::new(paths);
    state.can.pressure_triggers = !triggers.is_empty();
    state.can.process_events = conn.is_some();
    state.can.oom_log = log.is_some();
    confine(&state.paths);
    eprintln!("nexa-kerneld: watching (pressure triggers: {}, process events: {}, OOM log: {}, power meter: {}, app boost: {}, AI cgroup: {})",
              triggers.len(), conn.is_some(), log.is_some(), state.can.power_meter, state.can.app_boost, state.can.ai_cgroup);

    let mut last_tick = Instant::now();
    state.tick(TICK);
    loop {
        let mut fds = vec![libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        for (_, f) in &triggers {
            fds.push(libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLPRI, revents: 0 });
        }
        let conn_at = conn.as_ref().map(|c| {
            fds.push(libc::pollfd { fd: c.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            fds.len() - 1
        });
        let log_at = log.as_ref().map(|k| {
            fds.push(libc::pollfd { fd: k.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            fds.len() - 1
        });
        let wait = TICK.saturating_sub(last_tick.elapsed()).as_millis() as i32;
        // SAFETY: fds is a valid array of fds.len() pollfds.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, wait.max(0)) };
        if n > 0 {
            if fds[0].revents & libc::POLLIN != 0 {
                while let Ok((s, _)) = listener.accept() {
                    client(&mut state, s);
                }
            }
            for (i, (res, _)) in triggers.iter().enumerate() {
                if fds[1 + i].revents & libc::POLLPRI != 0 {
                    state.on_pressure(res);
                }
            }
            if let (Some(c), Some(at)) = (&conn, conn_at) {
                if fds[at].revents & libc::POLLIN != 0 {
                    for e in c.drain() {
                        state.on_process(e);
                    }
                }
            }
            if let (Some(k), Some(at)) = (log.as_mut(), log_at) {
                if fds[at].revents & libc::POLLIN != 0 {
                    for o in k.drain() {
                        state.on_oom(o);
                    }
                }
            }
        }
        if last_tick.elapsed() >= TICK {
            let since = last_tick.elapsed();
            last_tick = Instant::now();
            state.tick(since);
        }
    }
}

fn ask(req: Value) -> Result<Value, String> {
    let socket = Paths::from_env().socket;
    let mut s = UnixStream::connect(&socket).map_err(|e| format!("nexa-kerneld isn't running ({}): {e}", socket.display()))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    writeln!(s, "{req}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| e.to_string())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let req = match args.get(1).map(String::as_str) {
        Some("serve") => serve(),
        Some("focus-watch") => {
            // The compositor may still be starting: retry for a while.
            let socket = Paths::from_env().socket;
            for _ in 0..30 {
                if let Err(e) = focus::watch(&socket) {
                    eprintln!("nexa-kerneld focus-watch: {e}");
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            return;
        }
        Some(op @ ("status" | "events" | "energy")) => json!({"op": op}),
        Some("focus") => json!({"op": "focus", "pid": args.get(2).and_then(|p| p.parse::<i64>().ok()).unwrap_or(0)}),
        _ => {
            eprintln!("usage: nexa-kerneld serve | status | events | energy | focus PID | focus-watch");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str) -> State {
        let r = std::env::temp_dir().join(format!("nexa-kerneld-state-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        fs::create_dir_all(r.join("etc")).unwrap();
        fs::write(r.join("etc/passwd"), "root:x:0:0::/root:/bin/sh\nann:x:1000:1000::/home/ann:/bin/sh\nbob:x:1001:1001::/home/bob:/bin/sh\n").unwrap();
        fs::write(r.join("etc/group"), "wheel:x:10:root,ann\n").unwrap();
        State::new(Paths { proc_root: "/proc".into(), sys: r.join("sys"), etc: r.join("etc"), socket: r.join("s"), kmsg: r.join("k") })
    }

    #[test]
    fn only_your_own_programs() {
        let mut s = state("own");
        // pid 1 belongs to root (or whoever runs init here), never to uid 4242.
        let e = s.focus(1, 4242).unwrap_err();
        assert!(e.contains("your own programs"), "{e}");
        assert!(s.is_admin(0) && s.is_admin(1000) && !s.is_admin(1001));
    }

    #[test]
    fn events_are_private() {
        let mut s = state("events");
        s.on_process(procev::Event::Exit { pid: 99_999_990, status: 0, signal: libc::SIGTERM });
        assert!(s.events.is_empty(), "being asked to stop isn't a crash");
        s.procs.insert(500, ("editor".into(), 1001));
        s.on_process(procev::Event::Exit { pid: 500, status: 0, signal: libc::SIGSEGV });
        s.on_oom(kmsg::OomKill { pid: 600, name: "firefox".into(), uid: Some(1000), cgroup: false });
        let mut seen = |uid| s.handle(&json!({"op": "events"}), uid).unwrap()["events"].as_array().unwrap()
            .iter().map(|e| e["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(seen(1001), vec!["editor"], "bob sees his own crash only");
        assert_eq!(seen(1000), vec!["firefox", "editor"], "ann is an administrator: everything, newest first");
        let ev = s.handle(&json!({"op": "events"}), 1001).unwrap();
        assert_eq!(ev["events"][0]["text"], "editor crashed with a memory error (SIGSEGV)");
        assert!(ev["events"][0].get("uid").is_none(), "uids aren't handed out");
    }
}
