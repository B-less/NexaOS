//! nexa-brokerd: the one place where NexaOS changes the system as root.
//!
//!   nexa-brokerd serve                  the service (OpenRC, as root)
//!   nexa-brokerd call ACTION [JSON]     ask it, e.g. call set_brightness '{"percent":40}'
//!   nexa-brokerd exec                   (internal) perform one action, sandboxed
//!   nexa-brokerd drive-added|drive-removed NAME   (udev, as root)
//!
//! Apps send one JSON line to /run/nexa-broker.sock:
//!   {"action": "set_brightness", "args": {"percent": 40}, "password": "..."}
//! and get one back: {"ok": true, "result": {...}} or
//! {"ok": false, "error": "...", "needs": "password"}.
//!
//! The broker checks who is asking (the kernel says, SO_PEERCRED; they must
//! be root or in the wheel group), that the action is one it knows with
//! valid arguments (policy.rs), and the password when the action needs one
//! (three wrong tries lock that person out for a minute). It then runs the
//! action in a separate worker confined with Landlock to just what that
//! action touches, one request at a time, and logs every request to
//! /var/log/nexa-broker.log. Setting changes return the previous value as
//! an "undo" request.

mod apps;
mod auth;
mod drives;
mod policy;
mod users;
mod vpn;
mod worker;

use nexa_sys::{peer, sandbox};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SOCKET: &str = "/run/nexa-broker.sock";
const LOG: &str = "/var/log/nexa-broker.log";
const MAX_REQUEST: u64 = 64 * 1024;
const MAX_FAILURES: u32 = 3;
const LOCKOUT: Duration = Duration::from_secs(60);

fn socket_path() -> String {
    std::env::var("NEXA_BROKER_SOCKET").unwrap_or_else(|_| SOCKET.into())
}

// ---- the service ---------------------------------------------------------

struct Broker {
    etc: PathBuf,
    log: PathBuf,
    failures: HashMap<u32, (u32, Instant)>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Broker {
    fn audit(&self, uid: u32, user: &str, action: &str, what: &str, outcome: &Value) {
        let line = json!({"time": now(), "uid": uid, "user": user, "action": action, "what": what,
                          "ok": outcome["ok"], "error": outcome.get("error")});
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.log) {
            let _ = writeln!(f, "{line}");
        }
    }

    fn handle(&mut self, peer: &peer::Peer, req: &Value) -> Value {
        let action = req.get("action").and_then(Value::as_str).unwrap_or("");
        let args = req.get("args").cloned().unwrap_or_else(|| json!({}));
        let user = auth::user_and_groups(peer.uid, &self.etc).map(|(u, _)| u).unwrap_or_else(|| peer.uid.to_string());
        if !auth::may_ask(peer.uid, &self.etc) {
            let out = json!({"ok": false, "error": "only administrators (the wheel group) can change the system"});
            self.audit(peer.uid, &user, action, "refused: not an administrator", &out);
            return out;
        }
        let parsed = match policy::parse(action, &args, Path::new("/")) {
            Ok(a) => a,
            Err(e) => {
                let out = json!({"ok": false, "error": e});
                self.audit(peer.uid, &user, action, "refused: invalid request", &out);
                return out;
            }
        };
        let what = parsed.describe();
        if parsed.level() == policy::Level::Password && peer.uid != 0 {
            if let Some((n, since)) = self.failures.get(&peer.uid) {
                if *n >= MAX_FAILURES && since.elapsed() < LOCKOUT {
                    let wait = (LOCKOUT - since.elapsed()).as_secs() + 1;
                    return json!({"ok": false, "error": format!("too many wrong passwords; try again in {wait} s"), "needs": "password"});
                }
            }
            match auth::check_password(&user, req.get("password").and_then(Value::as_str), &self.etc) {
                auth::Check::Ok | auth::Check::NoPassword => {
                    self.failures.remove(&peer.uid);
                }
                auth::Check::Locked => {
                    let out = json!({"ok": false, "error": "this account's password is locked"});
                    self.audit(peer.uid, &user, action, &what, &out);
                    return out;
                }
                auth::Check::Wrong => {
                    if req.get("password").is_none() {
                        return json!({"ok": false, "error": "needs your password", "needs": "password", "what": what});
                    }
                    let e = self.failures.entry(peer.uid).or_insert((0, Instant::now()));
                    if e.1.elapsed() >= LOCKOUT { *e = (0, Instant::now()); }
                    e.0 += 1;
                    e.1 = Instant::now();
                    let out = json!({"ok": false, "error": "wrong password", "needs": "password", "what": what});
                    self.audit(peer.uid, &user, action, &what, &out);
                    return out;
                }
            }
        }
        let out = run_worker(action, &args, parsed.access().timeout_s, peer.uid, &user);
        self.audit(peer.uid, &user, action, &what, &out);
        out
    }
}

/// Run `nexa-brokerd exec` with a clean environment and a time limit.
fn run_worker(action: &str, args: &Value, timeout_s: u64, uid: u32, user: &str) -> Value {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return json!({"ok": false, "error": e.to_string()}),
    };
    let mut child = match Command::new(exe).arg("exec").env_clear().env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "error": format!("couldn't start the worker: {e}")}),
    };
    let req = json!({"action": action, "args": args, "caller": {"uid": uid, "user": user}});
    let _ = child.stdin.take().unwrap().write_all(req.to_string().as_bytes());
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return json!({"ok": false, "error": format!("took longer than {timeout_s} s and was stopped")});
            }
        }
    }
    let text = reader.join().unwrap_or_default();
    serde_json::from_str(text.trim()).unwrap_or_else(|_| json!({"ok": false, "error": "the worker gave no answer"}))
}

fn serve() -> ! {
    // The desktop's drive list exists from the start (drives plugged in at boot).
    drives::publish(&drives::Tree::real());
    // SAFETY: geteuid can't fail.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("nexa-brokerd: serve must run as root");
        std::process::exit(1);
    }
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap_or_else(|e| {
        eprintln!("nexa-brokerd: {path}: {e}");
        std::process::exit(1);
    });
    // Anyone may connect; who may *ask* is checked per request (SO_PEERCRED).
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666));
    // Tests point these elsewhere; OpenRC starts the service without them.
    let etc = std::env::var_os("NEXA_BROKER_ETC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/etc"));
    let log = std::env::var_os("NEXA_BROKER_LOG").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(LOG));
    let mut b = Broker { etc, log, failures: HashMap::new() };
    eprintln!("nexa-brokerd: listening on {path}");
    // One request at a time: system changes never race each other.
    for stream in listener.incoming().flatten() {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let Ok(p) = peer::of(&stream) else { continue };
        let mut line = String::new();
        let mut r = BufReader::new((&stream).take(MAX_REQUEST));
        let reply = match r.read_line(&mut line) {
            Ok(n) if n > 0 => match serde_json::from_str::<Value>(&line) {
                Ok(req) => b.handle(&p, &req),
                Err(e) => json!({"ok": false, "error": format!("bad request: {e}")}),
            },
            _ => continue,
        };
        let mut w = &stream;
        let _ = writeln!(w, "{reply}");
    }
    std::process::exit(0)
}

// ---- the worker ----------------------------------------------------------

fn exec() -> Value {
    let mut input = String::new();
    if std::io::stdin().take(MAX_REQUEST).read_to_string(&mut input).is_err() {
        return json!({"ok": false, "error": "no request"});
    }
    let req: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => return json!({"ok": false, "error": format!("bad request: {e}")}),
    };
    // Tests point the worker at a fake system tree; the service never sets
    // this (it starts workers with a clean environment).
    let root = std::env::var_os("NEXA_BROKER_ROOT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    let action = req.get("action").and_then(Value::as_str).unwrap_or("");
    let args = req.get("args").cloned().unwrap_or_else(|| json!({}));
    let a = match policy::parse(action, &args, &root) {
        Ok(a) => a,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let caller = worker::Caller {
        uid: req["caller"]["uid"].as_u64().unwrap_or(0) as u32,
        user: req["caller"]["user"].as_str().unwrap_or("root").to_string(),
    };
    let acc = a.access();
    if !acc.sandbox {
        return finish(worker::perform(&a, &root, &caller));
    }
    let mut fs: Vec<(String, u64)> = Vec::new();
    let base = if acc.exec { sandbox::READ_EXEC } else { sandbox::READ };
    for p in ["/usr", "/lib", "/bin", "/sbin", "/etc", "/proc", "/sys", "/run", "/var"] {
        fs.push((p.into(), base));
    }
    fs.push(("/dev/null".into(), sandbox::READ_WRITE));
    for w in &acc.write {
        let p = if root == Path::new("/") { PathBuf::from(w) } else { root.join(w.trim_start_matches('/')) };
        let rights = if *w == "/" { sandbox::ALL } else { sandbox::READ_WRITE | if acc.exec { sandbox::EXECUTE } else { 0 } };
        fs.push((p.to_string_lossy().into(), rights));
    }
    if root != Path::new("/") {
        fs.push((root.to_string_lossy().into(), sandbox::READ));
    }
    // The package being installed (on the real system "/" already covers it).
    if let policy::Action::InstallApp { path, .. } = &a {
        if let Some(dir) = Path::new(path).parent() {
            fs.push((dir.to_string_lossy().into(), sandbox::READ));
        }
    }
    if let Err(e) = sandbox::restrict(&sandbox::Rules { fs, connect_ports: acc.net.clone(), any_network: false }) {
        return json!({"ok": false, "error": format!("couldn't enable the sandbox: {e}")});
    }
    finish(worker::perform(&a, &root, &caller))
}

fn finish(r: Result<Value, String>) -> Value {
    match r {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(e) => match e.strip_prefix(apps::UNVERIFIED) {
            Some(why) => json!({"ok": false, "error": why, "unverified": true}),
            None => json!({"ok": false, "error": e}),
        },
    }
}

// ---- the client ----------------------------------------------------------

fn call(action: &str, args: &str) -> Value {
    let args: Value = serde_json::from_str(args).unwrap_or_else(|_| json!({}));
    let mut req = json!({"action": action, "args": args});
    if let Ok(pw) = std::env::var("NEXA_BROKER_PASSWORD") {
        req["password"] = json!(pw);
    }
    let mut s = match UnixStream::connect(socket_path()) {
        Ok(s) => s,
        Err(e) => return json!({"ok": false, "error": format!("the system broker isn't running ({e})")}),
    };
    let _ = writeln!(s, "{req}");
    let mut line = String::new();
    let _ = BufReader::new(s).read_line(&mut line);
    serde_json::from_str(&line).unwrap_or_else(|_| json!({"ok": false, "error": "no answer"}))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = match args.get(1).map(String::as_str) {
        Some("serve") => serve(),
        Some("exec") => exec(),
        Some("call") if args.len() >= 3 => call(&args[2], args.get(3).map(String::as_str).unwrap_or("{}")),
        // udev runs these as root when a USB/SD drive comes or goes.
        Some("drive-added") if args.len() == 3 => {
            let t = drives::Tree::real();
            let v = drives::added(&t, &args[2]);
            drives::publish(&t);
            v
        }
        Some("drive-removed") if args.len() == 3 => {
            let t = drives::Tree::real();
            let v = drives::removed(&t, &args[2]);
            drives::publish(&t);
            v
        }
        _ => {
            eprintln!("usage: nexa-brokerd serve | call ACTION [JSON-ARGS] | exec | drive-added NAME | drive-removed NAME");
            std::process::exit(2);
        }
    };
    println!("{out}");
    std::process::exit(if out["ok"] == false || out.get("error").is_some() && out["ok"] != true { 1 } else { 0 });
}
