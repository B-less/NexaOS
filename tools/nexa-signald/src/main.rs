//! nexa-signald: NexaOS's notification service. Owns
//! org.freedesktop.Notifications on the session bus, keeps the Signals
//! (store.rs) and tells nexa-signals (C, the tabs on the bar's edge) what to
//! show over $XDG_RUNTIME_DIR/nexa-signals.sock: JSON lines, this user only.
//!
//!   nexa-signald            run (one per session)
//!
//! Control socket requests, one JSON object per line, each answered with
//! one line ({"ok": true, ...}):
//!   subscribe                 then a {"event": "state", ...} line on every change
//!   hold {on}                 the pointer is on the Signals: timers wait
//!   invoke {id, action}       an action (or "default": a click on it)
//!   dismiss {id} | dismiss_all
//!   list | history            makoctl's JSON shape
//!   mode | set_quiet {on}
//!   reload                    Settings > Notifications changed

use dbus::blocking::SyncConnection;
use dbus::channel::{MatchingReceiver, Sender};
use dbus::message::MatchRule;
use dbus::Message;
use dbus_crossroads::Crossroads;
use nexa_signald::store::{Incoming, Reason, Store};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

struct Shared {
    store: Mutex<Store>,
    subs: Mutex<Vec<UnixStream>>,
    conn: Arc<SyncConnection>,
    /// one byte here wakes the D-Bus loop, which then sends what was queued
    wake: std::os::unix::io::RawFd,
}

impl Shared {
    fn wake(&self) {
        // SAFETY: a write of one byte from a valid buffer to our own pipe;
        // a full pipe (EAGAIN) already means the loop will wake.
        unsafe { libc::write(self.wake, b"!".as_ptr() as *const libc::c_void, 1) };
    }

    fn emit_closed(&self, closed: &[(u32, Reason)]) {
        for (id, why) in closed {
            if let Ok(m) = Message::new_signal(PATH, NAME, "NotificationClosed") {
                let _ = self.conn.send(m.append2(*id, *why as u32));
            }
        }
        self.wake();
    }

    fn emit_action(&self, id: u32, key: &str) {
        if let Ok(m) = Message::new_signal(PATH, NAME, "ActionInvoked") {
            let _ = self.conn.send(m.append2(id, key));
        }
        self.wake();
    }

    /// Every subscriber gets the new state; one that went away is dropped.
    fn broadcast(&self) {
        let line = {
            let st = self.store.lock().unwrap();
            format!("{}\n", st.state(Instant::now()))
        };
        let mut subs = self.subs.lock().unwrap();
        subs.retain_mut(|s| s.write_all(line.as_bytes()).is_ok());
    }
}

fn hint_u64(hints: &dbus::arg::PropMap, key: &str) -> Option<u64> {
    hints.get(key).and_then(|v| v.0.as_u64())
}

fn hint_str(hints: &dbus::arg::PropMap, key: &str) -> Option<String> {
    hints.get(key).and_then(|v| v.0.as_str()).map(String::from)
}

fn serve_dbus(shared: Arc<Shared>, wake_r: std::os::unix::io::RawFd) -> Result<(), dbus::Error> {
    let conn = shared.conn.clone();
    let mut cr = Crossroads::new();
    let token = cr.register(NAME, |b| {
        b.method("GetCapabilities", (), ("capabilities",), |_, _: &mut Arc<Shared>, ()| {
            Ok((["actions", "body", "body-markup", "icon-static", "persistence"].map(String::from).to_vec(),))
        });
        b.method("GetServerInformation", (), ("name", "vendor", "version", "spec_version"), |_, _: &mut Arc<Shared>, ()| {
            Ok(("Nexa Signals".to_string(), "NexaOS".to_string(), env!("CARGO_PKG_VERSION").to_string(), "1.2".to_string()))
        });
        b.method(
            "Notify",
            ("app_name", "replaces_id", "app_icon", "summary", "body", "actions", "hints", "expire_timeout"),
            ("id",),
            |_, sh: &mut Arc<Shared>, (app, replaces, icon, summary, body, actions, hints, timeout): (String, u32, String, String, String, Vec<String>, dbus::arg::PropMap, i32)| {
                let icon = if icon.is_empty() { hint_str(&hints, "image-path").unwrap_or_default() } else { icon };
                let n = Incoming {
                    app, replaces, icon, summary, body, actions, timeout_ms: timeout,
                    urgency: hint_u64(&hints, "urgency").unwrap_or(1).min(2) as u8,
                    desktop: hint_str(&hints, "desktop-entry").unwrap_or_default(),
                };
                let mood = nexa_signald::layer::now();  /* asked outside the lock: the compositor may be slow */
                let (id, gone) = {
                    let mut st = sh.store.lock().unwrap();
                    st.mood = mood;
                    st.notify(n, Instant::now())
                };
                sh.emit_closed(&gone);
                sh.broadcast();
                Ok((id,))
            },
        );
        b.method("CloseNotification", ("id",), (), |_, sh: &mut Arc<Shared>, (id,): (u32,)| {
            let closed = sh.store.lock().unwrap().close(id, Reason::Closed);
            if let Some(c) = closed {
                sh.emit_closed(&[c]);
                sh.broadcast();
            }
            Ok(())
        });
        b.signal::<(u32, u32), _>("NotificationClosed", ("id", "reason"));
        b.signal::<(u32, String), _>("ActionInvoked", ("id", "action_key"));
    });
    cr.insert(PATH, &[token], shared.clone());
    let cr = Arc::new(Mutex::new(cr));
    conn.start_receive(MatchRule::new_method_call(), Box::new(move |msg, c| {
        let _ = cr.lock().unwrap().handle_message(msg, c);
        true
    }));
    let bus_fd = conn.channel().watch().fd;
    loop {
        // Sleep until the next Signal is due, a call comes in, or another
        // thread has something to send (the wake pipe). No polling.
        let wait = {
            let st = shared.store.lock().unwrap();
            match st.next_deadline() {
                Some(d) => d.saturating_duration_since(Instant::now()).clamp(Duration::from_millis(20), Duration::from_secs(1)),
                None if st.active.is_empty() => Duration::from_secs(30),
                // held, or waiting until dismissed: a hold can end at any moment
                None => Duration::from_secs(1),
            }
        };
        let mut fds = [libc::pollfd { fd: bus_fd, events: libc::POLLIN, revents: 0 },
                       libc::pollfd { fd: wake_r, events: libc::POLLIN, revents: 0 }];
        // SAFETY: two valid pollfd structs; poll only writes their revents.
        unsafe { libc::poll(fds.as_mut_ptr(), 2, wait.as_millis() as libc::c_int) };
        if fds[1].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            // SAFETY: reading into our own buffer from our own non-blocking pipe.
            while unsafe { libc::read(wake_r, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) } > 0 {}
        }
        // everything waiting: incoming calls handled, queued messages sent
        while conn.process(Duration::ZERO)? {}
        let gone = shared.store.lock().unwrap().expire(Instant::now());
        if !gone.is_empty() {
            shared.emit_closed(&gone);
            shared.broadcast();
        }
    }
}

fn reply(out: &mut UnixStream, v: Value) -> bool {
    out.write_all(format!("{v}\n").as_bytes()).is_ok()
}

fn handle(sh: &Arc<Shared>, req: &Value, out: &mut UnixStream) -> bool {
    let cmd = req.get("cmd").and_then(Value::as_str).unwrap_or("");
    let id = req.get("id").and_then(Value::as_u64).map(|v| v as u32);
    let on = req.get("on").and_then(Value::as_bool);
    let now = Instant::now();
    let mut changed = true;
    let answer = match (cmd, id, on) {
        ("subscribe", _, _) => {
            let first = format!("{}\n", sh.store.lock().unwrap().state(now));
            if let Ok(clone) = out.try_clone() {
                sh.subs.lock().unwrap().push(clone);
            }
            return out.write_all(first.as_bytes()).is_ok();
        }
        ("hold", _, Some(on)) => {
            sh.store.lock().unwrap().hold(on, now);
            json!({"ok": true})
        }
        ("invoke", Some(id), _) => {
            let key = req.get("action").and_then(Value::as_str).unwrap_or("default");
            let desktop = sh.store.lock().unwrap().desktop_of(id);
            let done = sh.store.lock().unwrap().invoke(id, key);
            if done.is_some() {
                if let Some(d) = desktop {
                    nexa_signald::layer::focus_app(&d);  // the person clicked it: its app comes forward
                }
            }
            match done {
                Some((send, closed)) => {
                    if let Some(k) = send {
                        sh.emit_action(id, &k);  // the spec: ActionInvoked, then closed
                    }
                    sh.emit_closed(&[closed]);
                    json!({"ok": true})
                }
                None => {
                    changed = false;
                    json!({"ok": false, "error": "no such Signal or action"})
                }
            }
        }
        ("dismiss", Some(id), _) => {
            let closed = sh.store.lock().unwrap().close(id, Reason::Dismissed);
            sh.emit_closed(&closed.into_iter().collect::<Vec<_>>());
            json!({"ok": true})
        }
        ("dismiss_all", _, _) => {
            let closed = sh.store.lock().unwrap().dismiss_all();
            sh.emit_closed(&closed);
            json!({"ok": true})
        }
        ("list", _, _) | ("history", _, _) => {
            changed = false;
            let st = sh.store.lock().unwrap();
            let r = if cmd == "list" { Store::mako_json(st.active.iter()) } else { Store::mako_json(st.history.iter()) };
            json!({"ok": true, "result": r})
        }
        ("mode", _, _) => {
            changed = false;
            json!({"ok": true, "quiet": sh.store.lock().unwrap().quiet})
        }
        ("set_quiet", _, Some(on)) => {
            sh.store.lock().unwrap().set_quiet(on);
            json!({"ok": true, "quiet": on})
        }
        ("reload", _, _) => {
            sh.store.lock().unwrap().set_prefs(nexa_signald::load_prefs());
            json!({"ok": true})
        }
        _ => {
            changed = false;
            json!({"ok": false, "error": "unknown request"})
        }
    };
    if changed {
        sh.broadcast();
    }
    reply(out, answer)
}

fn control_conn(sh: Arc<Shared>, stream: UnixStream) {
    // SAFETY: getuid has no failure mode.
    let me = unsafe { libc::getuid() };
    if nexa_sys::peer::of(&stream).map(|p| p.uid != me).unwrap_or(true) {
        return;
    }
    let Ok(mut out) = stream.try_clone() else { return };
    out.set_write_timeout(Some(Duration::from_secs(2))).ok();  // a stuck reader can't stall the service
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match (&mut reader).take(64 * 1024).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if !line.ends_with('\n') {
            return;  // longer than 64 KB: someone misbehaving
        }
        let req: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        if !handle(&sh, &req, &mut out) {
            return;
        }
    }
}

fn confine() {
    use nexa_sys::sandbox::{self, READ, READ_EXEC, WRITE_FILE};
    let mut rules: Vec<(String, u64)> = ["/usr", "/lib", "/bin"].iter().map(|p| (p.to_string(), READ_EXEC)).collect();
    for p in ["/etc", "/proc", "/sys", "/dev/urandom"] {
        rules.push((p.into(), READ));
    }
    rules.push(("/dev/null".into(), READ | WRITE_FILE));
    // Settings' file only; it never writes anything.
    if let Some(dir) = nexa_signald::prefs_path().parent() {
        rules.push((dir.to_string_lossy().into_owned(), READ));
    }
    if let Err(e) = sandbox::restrict(&sandbox::Rules { fs: rules, connect_ports: vec![], any_network: false }) {
        eprintln!("nexa-signald: Landlock: {e} (running unconfined)");
    }
}

fn main() {
    if std::env::args().len() > 1 {
        eprintln!("usage: nexa-signald   (the CLI is nexa-signalctl)");
        std::process::exit(2);
    }
    // The bus socket is watched by our own poll (with a wake pipe), so the
    // channel is made by hand with watch tracking on.
    let conn = match dbus::channel::Channel::get_private(dbus::channel::BusType::Session) {
        Ok(mut ch) => {
            ch.set_watch_enabled(true);
            Arc::new(SyncConnection::from(ch))
        }
        Err(e) => {
            eprintln!("nexa-signald: no session bus: {e}");
            std::process::exit(1);
        }
    };
    // Another notification service already has the name (mako): say so and stop.
    match conn.request_name(NAME, false, true, true) {
        Ok(dbus::blocking::stdintf::org_freedesktop_dbus::RequestNameReply::PrimaryOwner) => {}
        Ok(r) => {
            eprintln!("nexa-signald: {NAME} is taken by another program ({r:?})");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("nexa-signald: {e}");
            std::process::exit(1);
        }
    }
    let sock = nexa_signald::socket_path();
    std::fs::remove_file(&sock).ok();
    let listener = match UnixListener::bind(&sock) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nexa-signald: {}: {e}", sock.display());
            std::process::exit(1);
        }
    };
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600)).ok();
    confine();
    let mut pipe = [0; 2];
    // SAFETY: pipe2 fills two fds into our array.
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } != 0 {
        eprintln!("nexa-signald: no pipe: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let shared = Arc::new(Shared { store: Mutex::new(Store::new(nexa_signald::load_prefs())), subs: Mutex::new(vec![]), conn, wake: pipe[1] });
    {
        let sh = shared.clone();
        std::thread::spawn(move || {
            for c in listener.incoming().flatten() {
                let sh = sh.clone();
                std::thread::spawn(move || control_conn(sh, c));
            }
        });
    }
    if let Err(e) = serve_dbus(shared, pipe[0]) {
        eprintln!("nexa-signald: the session bus went away: {e}");
        std::process::exit(1);
    }
}
