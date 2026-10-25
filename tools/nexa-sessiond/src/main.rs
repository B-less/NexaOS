//! nexa-sessiond: NexaOS's desktop sessions, one per person.
//!
//!   nexa-sessiond serve            the service (OpenRC "sway", as root)
//!   nexa-sessiond people           who can sign in, who is, who's on screen
//!   nexa-sessiond switch NAME      put NAME's desktop on screen
//!
//! At boot it starts the first person's desktop (uid 1000) on the VT that's
//! on screen. "Switch person" (the lock screen, Power, Settings > Users)
//! asks it for someone else: their desktop starts on a free VT (7-12) and
//! opens locked, or, if it's already running, its VT comes back on screen.
//! seatd pauses the desktop that leaves and resumes the one that arrives,
//! so everyone's windows stay as they were.
//!
//! Whenever a desktop leaves the screen (Switch person, or Ctrl+Alt+F-keys)
//! it's locked, so nobody walks into someone else's desktop. The one that
//! arrives is told (SIGUSR2 to its lock screen) to play its arrival.
//!
//! When a desktop ends: the first person's starts again (it opens locked,
//! as after "Log out"); anyone else's was a sign-out, and the screen goes
//! back to the desktop that was on it before. Whatever the person left
//! running is ended. Five ends within 10 s in a row stop the restarts.
//!
//! Requests are one line on /run/nexa-session.sock (anyone may connect;
//! the kernel says who did). Only the person whose desktop is on screen (or
//! root) may switch. Replies: "ok ..." or "error <why>"; "people" adds one
//! line per person: name, uid, display name, signed in (0/1), on screen (0/1),
//! tab-separated.

mod people;
mod vt;

use nexa_sys::peer;
use people::Person;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{PermissionsExt, chown};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const QUICK: Duration = Duration::from_secs(10);
const ARRIVAL_DELAY: Duration = Duration::from_millis(350);

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn socket_path() -> String {
    env_or("NEXA_SESSION_SOCKET", "/run/nexa-session.sock")
}

fn stamp() -> String {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r fills tm from t.
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{}-{:02}-{:02} {:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

struct Session {
    person: Person,
    vt: u32,
    child: Child,
    began: Instant,
}

struct Manager {
    etc: PathBuf,
    log: PathBuf,
    session_cmd: String,
    lock_cmd: String,
    vts: vt::Vts,
    primary: Person,
    primary_vt: u32,
    sessions: Vec<Session>,
    /// VTs most recently on screen first.
    history: Vec<u32>,
    last_vt: u32,
    quick: HashMap<String, u32>,
    given_up: HashSet<String>,
    restart_at: Option<Instant>,
    arrivals: Vec<(u32, Instant)>,
    helpers: Vec<Child>,
}

/// Every process of `uid` (never root's or a system account's).
fn processes_of(uid: u32) -> Vec<i32> {
    if uid < 1000 {
        return vec![];
    }
    let me = std::process::id() as i32;
    fs::read_dir("/proc").into_iter().flatten().flatten().filter_map(|e| {
        let pid: i32 = e.file_name().to_str()?.parse().ok()?;
        let status = fs::read_to_string(e.path().join("status")).ok()?;
        let real: u32 = status.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?.parse().ok()?;
        (real == uid && pid != me).then_some(pid)
    }).collect()
}

fn comm(pid: i32) -> String {
    fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default().trim().to_string()
}

/// The person's Wayland socket, if their desktop has one.
fn wayland_socket(run: &Path) -> Option<String> {
    let mut names: Vec<String> = fs::read_dir(run).ok()?.flatten().filter_map(|e| {
        let n = e.file_name().to_str()?.to_string();
        (n.starts_with("wayland-") && !n.ends_with(".lock")).then_some(n)
    }).collect();
    names.sort();
    names.into_iter().next()
}

impl Manager {
    fn say(&self, what: &str) {
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&self.log) {
            let _ = writeln!(f, "{} nexa-sessiond: {what}", stamp());
        }
    }

    fn session_on(&self, vt: u32) -> Option<&Session> {
        self.sessions.iter().find(|s| s.vt == vt)
    }

    fn session_of(&self, uid: u32) -> Option<&Session> {
        self.sessions.iter().find(|s| s.person.uid == uid)
    }

    fn start(&mut self, person: Person, vt: u32) -> Result<(), String> {
        let run = PathBuf::from(format!("/run/user/{}", person.uid));
        if fs::create_dir_all(&run).is_ok() {
            let _ = fs::set_permissions(&run, fs::Permissions::from_mode(0o700));
            let _ = chown(&run, Some(person.uid), Some(person.gid));
        }
        let log = fs::OpenOptions::new().create(true).append(true).open(&self.log).map_err(|e| e.to_string())?;
        let primary = person.uid == self.primary.uid;
        let mut cmd = Command::new(&self.session_cmd);
        cmd.arg("--wayland-only")
            .env("NEXA_SESSION_USER", &person.name)
            .env("XDG_RUNTIME_DIR", &run)
            .env("NEXA_SESSION_PRIMARY", if primary { "1" } else { "0" })
            .env("NEXA_SESSION_VT", vt.to_string())
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        // SAFETY: setsid is async-signal-safe; the desktop gets its own session.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| format!("couldn't start {}: {e}", self.session_cmd))?;
        self.say(&format!("started {}'s desktop on VT {vt} (pid {})", person.name, child.id()));
        self.sessions.push(Session { person, vt, child, began: Instant::now() });
        Ok(())
    }

    /// Ends whatever `uid` left running (their apps and services, pointed
    /// at a desktop that's gone, otherwise fail in a loop next to new ones).
    fn end_processes(&self, uid: u32) {
        let pids = processes_of(uid);
        for &p in &pids {
            // SAFETY: plain kill(2).
            unsafe { libc::kill(p, libc::SIGTERM) };
        }
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until && !processes_of(uid).is_empty() {
            std::thread::sleep(Duration::from_millis(50));
        }
        for p in processes_of(uid) {
            unsafe { libc::kill(p, libc::SIGKILL) };
        }
    }

    /// Locks a desktop that has left the screen (nexa-lock does nothing if
    /// it's locked already).
    fn lock(&mut self, uid: u32) {
        let Some(s) = self.session_of(uid) else { return };
        let p = s.person.clone();
        let run = PathBuf::from(format!("/run/user/{}", p.uid));
        let Some(display) = wayland_socket(&run) else { return };
        let child = Command::new(&self.lock_cmd)
            .env_clear()
            .env("PATH", env_or("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"))
            .env("HOME", &p.home)
            .env("USER", &p.name)
            .env("XDG_RUNTIME_DIR", &run)
            .env("WAYLAND_DISPLAY", display)
            .current_dir(if Path::new(&p.home).is_dir() { &p.home } else { "/" })
            .uid(p.uid)
            .gid(p.gid)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match child {
            Ok(c) => self.helpers.push(c),
            Err(e) => self.say(&format!("couldn't lock {}'s desktop: {e}", p.name)),
        }
    }

    fn arrive(&self, uid: u32) {
        for p in processes_of(uid) {
            if comm(p) == "nexa-locker" {
                // SAFETY: plain kill(2); the lock screen plays its arrival.
                unsafe { libc::kill(p, libc::SIGUSR2) };
            }
        }
    }

    fn people(&self) -> String {
        let active = self.vts.active();
        let mut out = String::from("ok\n");
        for p in people::people(&self.etc) {
            let s = self.session_of(p.uid);
            out += &format!("{}\t{}\t{}\t{}\t{}\n", p.name, p.uid, people::display_name(&p).replace('\t', " "),
                            s.is_some() as u8, s.is_some_and(|s| s.vt == active) as u8);
        }
        out
    }

    fn switch(&mut self, asker: u32, name: &str) -> Result<String, String> {
        let person = people::find(&self.etc, name)?;
        let active = self.vts.active();
        if asker != 0 && !self.session_on(active).is_some_and(|s| s.person.uid == asker) {
            return Err("only the person whose desktop is on screen can switch".into());
        }
        if let Some(s) = self.session_of(person.uid) {
            if s.vt == active {
                return Ok("already on screen".into());
            }
            let vt = s.vt;
            self.vts.activate(vt)?;
            self.say(&format!("switched to {} (VT {vt})", person.name));
            return Ok(format!("switched {vt}"));
        }
        let used: Vec<u32> = self.sessions.iter().map(|s| s.vt).collect();
        let vt = if person.uid == self.primary.uid {
            self.primary_vt
        } else {
            vt::free_vt(&used).ok_or("there are already six desktops open; someone needs to sign out first")?
        };
        self.given_up.remove(&person.name);
        self.quick.remove(&person.name);
        self.vts.prepare(vt, &format!("Starting {}'s desktop...", people::display_name(&person)));
        self.vts.activate(vt)?;
        self.start(person, vt)?;
        Ok(format!("started {vt}"))
    }

    fn handle(&mut self, asker: u32, line: &str) -> String {
        let mut words = line.split_whitespace();
        match (words.next(), words.next(), words.next()) {
            (Some("people"), None, _) => self.people(),
            (Some("switch"), Some(name), None) => match self.switch(asker, name) {
                Ok(what) => format!("ok {what}\n"),
                Err(why) => {
                    self.say(&format!("uid {asker} couldn't switch to {name}: {why}"));
                    format!("error {why}\n")
                }
            },
            _ => "error unknown request (people, switch NAME)\n".into(),
        }
    }

    fn ended(&mut self, s: Session, how: String) {
        let quick = s.began.elapsed() < QUICK;
        self.say(&format!("{}'s desktop ended after {} s ({how})", s.person.name, s.began.elapsed().as_secs()));
        self.end_processes(s.person.uid);
        let n = self.quick.entry(s.person.name.clone()).or_insert(0);
        *n = if quick { *n + 1 } else { 0 };
        if *n >= 5 {
            self.say(&format!("{}'s desktop ended 5 times in a row within 10 s; not starting it again", s.person.name));
            self.given_up.insert(s.person.name.clone());
        }
        if self.vts.active() != s.vt {
            return; // it ended off screen: nothing else changes
        }
        if s.person.uid == self.primary.uid {
            if !self.given_up.contains(&s.person.name) {
                self.restart_at = Some(Instant::now() + Duration::from_secs(1));
            }
            return;
        }
        // Someone else signed out: back to the desktop that was on screen before.
        let back = self.history.iter().copied().find(|v| *v != s.vt && self.session_on(*v).is_some());
        let result = match back {
            Some(v) => self.vts.activate(v),
            None if !self.given_up.contains(&self.primary.name) && self.session_of(self.primary.uid).is_none() => {
                let (p, v) = (self.primary.clone(), self.primary_vt);
                self.vts.activate(v).and_then(|_| self.start(p, v))
            }
            None => Ok(()),
        };
        if let Err(e) = result {
            self.say(&format!("couldn't go back after {} signed out: {e}", s.person.name));
        }
    }

    fn tick(&mut self) {
        let mut i = 0;
        while i < self.sessions.len() {
            match self.sessions[i].child.try_wait() {
                Ok(Some(status)) => {
                    let s = self.sessions.remove(i);
                    self.ended(s, status.to_string());
                }
                _ => i += 1,
            }
        }
        self.helpers.retain_mut(|c| !matches!(c.try_wait(), Ok(Some(_))));
        if self.restart_at.is_some_and(|t| Instant::now() >= t) {
            self.restart_at = None;
            if self.session_of(self.primary.uid).is_none() && self.vts.active() == self.primary_vt {
                let p = self.primary.clone();
                if let Err(e) = self.start(p, self.primary_vt) {
                    self.say(&e);
                }
            }
        }
        let now_vt = self.vts.active();
        if now_vt != 0 && now_vt != self.last_vt {
            let old = self.last_vt;
            self.last_vt = now_vt;
            self.history.retain(|v| *v != now_vt);
            self.history.insert(0, now_vt);
            self.history.truncate(8);
            if let Some(uid) = self.session_on(old).map(|s| s.person.uid) {
                self.lock(uid);
            }
            if let Some(s) = self.session_on(now_vt) {
                // A desktop that has only just started plays its own arrival.
                if s.began.elapsed() > Duration::from_secs(5) {
                    self.arrivals.push((s.person.uid, Instant::now() + ARRIVAL_DELAY));
                }
            }
        }
        let due: Vec<u32> = self.arrivals.iter().filter(|(_, t)| Instant::now() >= *t).map(|(u, _)| *u).collect();
        self.arrivals.retain(|(_, t)| Instant::now() < *t);
        for uid in due {
            self.arrive(uid);
        }
    }
}

fn serve() -> i32 {
    let etc = PathBuf::from(env_or("NEXA_SESSIOND_ETC", "/etc"));
    let vts = vt::Vts::from_env();
    let Some(primary) = people::primary(&etc) else {
        eprintln!("nexa-sessiond: nobody on this computer may use the desktop (no account in the seat group)");
        return 1;
    };
    let active = vts.active();
    let mut m = Manager {
        etc,
        log: env_or("NEXA_SESSIOND_LOG", "/var/log/nexa-session.log").into(),
        session_cmd: env_or("NEXA_SESSIOND_SESSION", "/usr/local/bin/nexa-session"),
        lock_cmd: env_or("NEXA_SESSIOND_LOCK", "nexa-lock"),
        primary_vt: if active > 0 { active } else { 1 },
        vts,
        primary,
        sessions: vec![],
        history: vec![],
        last_vt: active,
        quick: HashMap::new(),
        given_up: HashSet::new(),
        restart_at: None,
        arrivals: vec![],
        helpers: vec![],
    };
    m.history.push(m.primary_vt);

    let path = socket_path();
    let _ = fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nexa-sessiond: {path}: {e}");
            return 1;
        }
    };
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o666));
    let (tx, rx) = mpsc::channel::<(UnixStream, u32, String)>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let Ok(p) = peer::of(&stream) else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new((&stream).take(512)).read_line(&mut line).is_ok() && tx.send((stream, p.uid, line)).is_err() {
                break;
            }
        }
    });

    let p = m.primary.clone();
    let v = m.primary_vt;
    if let Err(e) = m.start(p, v) {
        m.say(&e);
    }
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok((mut stream, uid, line)) => {
                let reply = m.handle(uid, line.trim());
                let _ = stream.write_all(reply.as_bytes());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return 1,
        }
        m.tick();
    }
}

fn ask(line: &str) -> i32 {
    let path = socket_path();
    let mut s = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nexa-sessiond isn't running ({path}: {e})");
            return 1;
        }
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = s.write_all(format!("{line}\n").as_bytes());
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    let _ = s.read_to_string(&mut reply);
    print!("{reply}");
    if reply.starts_with("ok") { 0 } else { 1 }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    std::process::exit(match a.as_slice() {
        ["serve"] => serve(),
        ["people"] => ask("people"),
        ["switch", name] => ask(&format!("switch {name}")),
        _ => {
            eprintln!("usage: nexa-sessiond serve | people | switch NAME");
            2
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_wayland_socket() {
        let d = std::env::temp_dir().join(format!("nexa-run-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        assert_eq!(wayland_socket(&d), None);
        fs::write(d.join("wayland-1.lock"), "").unwrap();
        fs::write(d.join("wayland-1"), "").unwrap();
        assert_eq!(wayland_socket(&d).as_deref(), Some("wayland-1"));
        fs::remove_dir_all(d).ok();
    }

    #[test]
    fn never_ends_system_processes() {
        assert!(processes_of(0).is_empty() && processes_of(81).is_empty());
    }
}
