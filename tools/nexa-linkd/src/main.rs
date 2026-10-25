//! nexa-linkd: Nexa Link. NexaOS devices you've paired find each other on
//! the local network and talk, encrypted, with no cloud, account or relay.
//! Nexa Drop lets a device receive from any nearby Nexa device for a few
//! minutes, each item asked first. See docs/NEXA_LINK.md.
//!
//!   nexa-linkd serve                     the background service (one per user)
//!   nexa-linkd id                        this device's id and name
//!   nexa-linkd name NAME                 rename this device
//!   nexa-linkd pair-offer [--yes] [--json]      show a pairing code and wait
//!   nexa-linkd pair-accept CODE [--yes] [--json] pair with the device showing CODE
//!   nexa-linkd devices | nearby | status
//!   nexa-linkd send DEVICE-ID|ADDRESS FILE [--json]
//!   nexa-linkd ping DEVICE-ID
//!   nexa-linkd drop MINUTES              receive from any Nexa device (0 = off)
//!   nexa-linkd allow DEVICE-ID ask|always|never   files from that device
//!   nexa-linkd forget DEVICE-ID
//!   nexa-linkd watch                     events (offers, received files) as JSON lines
//!   nexa-linkd answer OFFER yes|no
//!
//! The service talks to the UI over $XDG_RUNTIME_DIR/nexa-link.sock (0600,
//! this user only), JSON lines, like NexaOS's other services.

mod channel;
mod proto;
mod store;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use store::{Identity, Peer, Store};

const MAX_FILE: u64 = 16 << 30;
const ASK_TIMEOUT: Duration = Duration::from_secs(120);

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn port() -> u16 {
    std::env::var("NEXA_LINK_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(proto::PORT)
}

fn pair_port() -> u16 {
    port() + (proto::PAIR_PORT - proto::PORT)
}

fn run_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        // SAFETY: getuid has no failure mode.
        PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
    })
}

fn sock_path() -> PathBuf {
    run_dir().join("nexa-link.sock")
}

fn downloads() -> PathBuf {
    std::env::var_os("NEXA_LINK_DOWNLOADS").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join("Downloads")
    })
}

fn random16() -> [u8; 16] {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).expect("the kernel's random numbers");
    b
}

fn clean_name(v: &Value) -> String {
    let s: String = v.as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(40).collect();
    if s.trim().is_empty() { "Nexa device".into() } else { s.trim().to_string() }
}

/// This machine's IPv4 addresses and their broadcast addresses.
fn interfaces() -> (Vec<Ipv4Addr>, Vec<Ipv4Addr>) {
    let (mut own, mut bcast) = (Vec::new(), Vec::new());
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a list we free below; we only read it.
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return (own, vec![Ipv4Addr::BROADCAST]);
    }
    let mut p = ifap;
    while !p.is_null() {
        // SAFETY: p points into the list from getifaddrs.
        let ifa = unsafe { &*p };
        let v4 = |sa: *mut libc::sockaddr| -> Option<Ipv4Addr> {
            // SAFETY: a non-null sockaddr from getifaddrs; read as sockaddr_in only when AF_INET.
            if sa.is_null() || unsafe { (*sa).sa_family } as i32 != libc::AF_INET {
                return None;
            }
            let sin = unsafe { &*(sa as *const libc::sockaddr_in) };
            Some(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)))
        };
        if let Some(a) = v4(ifa.ifa_addr) {
            own.push(a);
            if ifa.ifa_flags & libc::IFF_BROADCAST as u32 != 0 && ifa.ifa_flags & libc::IFF_LOOPBACK as u32 == 0 {
                if let Some(b) = v4(ifa.ifa_ifu) {
                    bcast.push(b);
                }
            }
        }
        p = ifa.ifa_next;
    }
    // SAFETY: freeing the list getifaddrs gave us.
    unsafe { libc::freeifaddrs(ifap) };
    if bcast.is_empty() {
        bcast.push(Ipv4Addr::BROADCAST);
    }
    (own, bcast)
}

/// The address other devices on the network reach this one at.
fn primary_ip() -> Ipv4Addr {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|_| s)) // TEST-NET: nothing is sent
        .and_then(|s| s.local_addr())
        .ok()
        .and_then(|a| match a.ip() { IpAddr::V4(v) if !v.is_unspecified() => Some(v), _ => None })
        .unwrap_or(Ipv4Addr::LOCALHOST)
}

// ---- the service ----------------------------------------------------------

struct Nearby {
    addr: IpAddr,
    seen: Instant,
}

struct State {
    me: Identity,
    peers: Vec<Peer>,
    peers_mtime: Option<SystemTime>,
    nearby: HashMap<String, Nearby>,
    drop_until: Option<Instant>,
    pending: HashMap<u64, mpsc::Sender<bool>>,
    next_offer: u64,
    subscribers: Vec<UnixStream>,
}

type Shared = Arc<Mutex<State>>;

impl State {
    fn drop_open(&self) -> bool {
        self.drop_until.is_some_and(|t| Instant::now() < t)
    }
}

fn emit(state: &Shared, ev: Value) {
    let line = format!("{ev}\n");
    state.lock().unwrap().subscribers.retain_mut(|s| s.write_all(line.as_bytes()).is_ok());
}

fn reload_peers(state: &Shared, store: &Store) {
    let mtime = store.paired_mtime();
    let mut st = state.lock().unwrap();
    if st.peers_mtime != mtime {
        st.peers = store.peers();
        st.peers_mtime = mtime;
        let ids: Vec<String> = st.peers.iter().map(|p| p.id.clone()).collect();
        st.nearby.retain(|k, _| k.starts_with("drop:") || ids.contains(k));
    }
}

/// Every 3 s: one signal per paired device (only it can read it), plus an
/// open one while Nexa Drop is on.
fn beacon_loop(state: Shared, store: Store, sock: UdpSocket) {
    loop {
        reload_peers(&state, &store);
        let (packets, _) = {
            let st = state.lock().unwrap();
            let me = st.me.public();
            let mut pk: Vec<[u8; proto::BEACON_LEN]> =
                st.peers.iter().map(|p| proto::beacon(&p.beacon_key(), &me, now_secs(), &random16())).collect();
            if st.drop_open() {
                pk.push(proto::beacon(proto::drop_key(), proto::NO_SENDER, now_secs(), &random16()));
            }
            (pk, ())
        };
        let (_, targets) = interfaces();
        for pkt in &packets {
            for t in &targets {
                sock.send_to(pkt, SocketAddr::new(IpAddr::V4(*t), port())).ok();
            }
        }
        {
            let mut st = state.lock().unwrap();
            st.nearby.retain(|_, n| n.seen.elapsed() < Duration::from_secs(12));
            if st.drop_until.is_some() && !st.drop_open() {
                st.drop_until = None;
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn listen_loop(state: Shared, sock: UdpSocket) {
    let mut buf = [0u8; 64];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        if n != proto::BEACON_LEN {
            continue;
        }
        let pkt = &buf[..n];
        let (own, _) = interfaces();
        let mine = matches!(from.ip(), IpAddr::V4(v) if own.contains(&v));
        let mut st = state.lock().unwrap();
        let now = now_secs();
        let found = st.peers.iter().find(|p| proto::beacon_from(&p.beacon_key(), &p.public(), pkt, now)).map(|p| p.id.clone());
        if let Some(id) = found {
            st.nearby.insert(id, Nearby { addr: from.ip(), seen: Instant::now() });
        } else if !mine && proto::beacon_from(proto::drop_key(), proto::NO_SENDER, pkt, now) {
            st.nearby.insert(format!("drop:{}", from.ip()), Nearby { addr: from.ip(), seen: Instant::now() });
        }
        // Anything else: not for us. Dropped without a trace.
    }
}

/// Ask the person (through any open UI) whether to take an item.
fn ask_user(state: &Shared, mut ev: Value) -> bool {
    let (tx, rx) = mpsc::channel();
    {
        let mut st = state.lock().unwrap();
        if st.subscribers.is_empty() {
            return false; // nobody to ask: say no
        }
        st.next_offer += 1;
        let id = st.next_offer;
        st.pending.insert(id, tx);
        ev["offer"] = json!(id);
    }
    let id = ev["offer"].as_u64().unwrap_or(0);
    emit(state, ev);
    let answer = rx.recv_timeout(ASK_TIMEOUT).unwrap_or(false);
    state.lock().unwrap().pending.remove(&id);
    answer
}

fn receive_file(ch: &mut channel::Channel, state: &Shared, peer: Option<&Peer>, from: &str, msg: &Value) -> io::Result<()> {
    let name = match msg["name"].as_str().and_then(proto::safe_name) {
        Some(n) => n,
        None => return ch.send_json(&json!({"t": "decline", "why": "bad name"})),
    };
    let size = msg["size"].as_u64().unwrap_or(u64::MAX);
    let want = msg["sha256"].as_str().unwrap_or("").to_string();
    if size > MAX_FILE || want.len() != 64 {
        return ch.send_json(&json!({"t": "decline", "why": "too big or no checksum"}));
    }
    let policy = peer.map(|p| p.files.as_str()).unwrap_or("ask");
    let ok = match policy {
        "always" => true,
        "never" => false,
        _ => ask_user(state, json!({"event": "offer", "kind": "file", "from": from, "device": peer.map(|p| p.id.clone()),
                                     "paired": peer.is_some(), "name": name, "size": size})),
    };
    if !ok {
        return ch.send_json(&json!({"t": "decline"}));
    }
    ch.send_json(&json!({"t": "accept"}))?;
    let dir = downloads();
    fs::create_dir_all(&dir)?;
    let part = dir.join(format!(".{}.nexa-part-{}", name, proto::hex(&random16()[..4])));
    let result = (|| -> io::Result<PathBuf> {
        let mut f = fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut got = 0u64;
        ch.set_timeout(60);
        while got < size {
            let (kind, data) = ch.recv()?;
            if kind != proto::MSG_DATA || got + data.len() as u64 > size {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected data"));
            }
            hasher.update(&data);
            f.write_all(&data)?;
            got += data.len() as u64;
        }
        f.sync_all()?;
        if proto::hex(&hasher.finalize()) != want {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "checksum mismatch"));
        }
        let dest = proto::unique_path(&dir, &name);
        fs::rename(&part, &dest)?;
        Ok(dest)
    })();
    match result {
        Ok(dest) => {
            let saved = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ch.send_json(&json!({"t": "done", "saved": saved}))?;
            emit(state, json!({"event": "received", "from": from, "path": dest.to_string_lossy()}));
            Ok(())
        }
        Err(e) => {
            fs::remove_file(&part).ok();
            ch.send_json(&json!({"t": "error", "why": e.to_string()})).ok();
            Err(e)
        }
    }
}

fn handle_conn(stream: std::net::TcpStream, state: Shared) {
    let (secret, peers, drop_open, my_name) = {
        let st = state.lock().unwrap();
        (st.me.secret(), st.peers.clone(), st.drop_open(), st.me.name.clone())
    };
    let allowed = |k: &[u8]| drop_open || peers.iter().any(|p| p.public() == k);
    let Ok(mut ch) = channel::accept(stream, &secret, None, allowed) else { return };
    let peer = peers.iter().find(|p| p.public() == ch.remote_public).cloned();
    ch.set_timeout(30);
    let Ok(hello) = ch.recv_json() else { return };
    let from = peer.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| clean_name(&hello["name"]));
    if ch.send_json(&json!({"t": "hello", "name": my_name})).is_err() {
        return;
    }
    loop {
        let Ok(msg) = ch.recv_json() else { return };
        let r = match msg["t"].as_str().unwrap_or("") {
            "ping" => ch.send_json(&json!({"t": "pong"})),
            "offer" if msg["kind"] == "file" => receive_file(&mut ch, &state, peer.as_ref(), &from, &msg),
            "bye" => return,
            _ => ch.send_json(&json!({"t": "error", "why": "not understood"})),
        };
        if r.is_err() {
            return;
        }
    }
}

fn control(req: &Value, state: &Shared, store: &Store) -> Value {
    let id = req["id"].as_str().unwrap_or("");
    match req["cmd"].as_str().unwrap_or("") {
        "status" => {
            let st = state.lock().unwrap();
            json!({"ok": true, "id": st.me.id(), "name": st.me.name, "paired": st.peers.len(), "drop_open": st.drop_open()})
        }
        "devices" => {
            let st = state.lock().unwrap();
            let list: Vec<Value> = st.peers.iter().map(|p| json!({"id": p.id, "name": p.name, "files": p.files,
                "online": st.nearby.contains_key(&p.id)})).collect();
            json!({"ok": true, "devices": list})
        }
        "nearby" => {
            let st = state.lock().unwrap();
            let list: Vec<Value> = st.nearby.iter().map(|(k, n)| {
                let peer = st.peers.iter().find(|p| &p.id == k);
                json!({"id": peer.map(|p| p.id.clone()), "name": peer.map(|p| p.name.clone()), "addr": n.addr.to_string(),
                       "paired": peer.is_some(), "open_drop": k.starts_with("drop:")})
            }).collect();
            json!({"ok": true, "nearby": list})
        }
        "where" => match state.lock().unwrap().nearby.get(id) {
            Some(n) => json!({"ok": true, "addr": n.addr.to_string()}),
            None => json!({"ok": false, "error": "That device isn't nearby right now."}),
        },
        "drop" => {
            let minutes = req["minutes"].as_u64().unwrap_or(0).min(60);
            let mut st = state.lock().unwrap();
            st.drop_until = (minutes > 0).then(|| Instant::now() + Duration::from_secs(minutes * 60));
            json!({"ok": true, "drop_open": st.drop_open()})
        }
        "answer" => {
            let offer = req["offer"].as_u64().unwrap_or(0);
            match state.lock().unwrap().pending.remove(&offer) {
                Some(tx) => {
                    tx.send(req["accept"].as_bool().unwrap_or(false)).ok();
                    json!({"ok": true})
                }
                None => json!({"ok": false, "error": "That request has gone."}),
            }
        }
        "forget" => match store.forget(id) {
            Ok(gone) => json!({"ok": gone}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        },
        "allow" => match store.set_files(id, req["files"].as_str().unwrap_or("")) {
            Ok(done) => json!({"ok": done}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        },
        _ => json!({"ok": false, "error": "unknown command"}),
    }
}

fn control_conn(stream: UnixStream, state: Shared, store: Store) {
    // SAFETY: getuid has no failure mode.
    let me = unsafe { libc::getuid() };
    if nexa_sys::peer::of(&stream).map(|p| p.uid != me).unwrap_or(true) {
        return;
    }
    let Ok(mut out) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        if req["cmd"] == "subscribe" {
            if let Ok(s) = out.try_clone() {
                state.lock().unwrap().subscribers.push(s);
            }
            if writeln!(out, "{}", json!({"ok": true})).is_err() {
                return;
            }
            continue;
        }
        let reply = control(&req, &state, &store);
        if req["cmd"] == "forget" || req["cmd"] == "allow" {
            reload_peers(&state, &store);
        }
        if writeln!(out, "{reply}").is_err() {
            return;
        }
    }
}

fn confine(store_dir: &Path) {
    use nexa_sys::sandbox::{self, READ, READ_EXEC, READ_WRITE, WRITE_FILE};
    let dl = downloads();
    fs::create_dir_all(&dl).ok();
    let mut rules: Vec<(String, u64)> = ["/usr", "/lib", "/bin"].iter().map(|p| (p.to_string(), READ_EXEC)).collect();
    for p in ["/etc", "/proc", "/sys", "/dev/urandom", "/dev/random"] {
        rules.push((p.into(), READ));
    }
    rules.push(("/dev/null".into(), READ | WRITE_FILE));
    for p in [store_dir.to_path_buf(), dl, run_dir()] {
        rules.push((p.to_string_lossy().into_owned(), READ_WRITE));
    }
    // The service only answers: no outgoing TCP at all (sending is done by
    // the `send` command, as the person, outside the service).
    if let Err(e) = sandbox::restrict(&sandbox::Rules { fs: rules, connect_ports: vec![], any_network: false }) {
        eprintln!("nexa-linkd: Landlock: {e} (running unconfined)");
    }
}

fn serve(store: Store) -> io::Result<()> {
    let me = store.identity()?;
    let udp = UdpSocket::bind(("0.0.0.0", port()))?;
    udp.set_broadcast(true)?;
    let tcp = TcpListener::bind(("0.0.0.0", port()))?;
    let sock = sock_path();
    fs::remove_file(&sock).ok();
    let ctl = UnixListener::bind(&sock)?;
    fs::set_permissions(&sock, fs::Permissions::from_mode(0o600))?;
    confine(&store.dir);
    eprintln!("nexa-linkd: {} ({}) on port {}", me.name, me.id(), port());
    let state: Shared = Arc::new(Mutex::new(State {
        me, peers: Vec::new(), peers_mtime: None, nearby: HashMap::new(), drop_until: None,
        pending: HashMap::new(), next_offer: 0, subscribers: Vec::new(),
    }));
    reload_peers(&state, &store);
    {
        let (st, s, u) = (state.clone(), Store { dir: store.dir.clone() }, udp.try_clone()?);
        std::thread::spawn(move || beacon_loop(st, s, u));
    }
    {
        let st = state.clone();
        std::thread::spawn(move || listen_loop(st, udp));
    }
    {
        let st = state.clone();
        std::thread::spawn(move || {
            for conn in tcp.incoming().flatten() {
                let st = st.clone();
                std::thread::spawn(move || handle_conn(conn, st));
            }
        });
    }
    for conn in ctl.incoming().flatten() {
        let (st, s) = (state.clone(), Store { dir: store.dir.clone() });
        std::thread::spawn(move || control_conn(conn, st, s));
    }
    Ok(())
}

// ---- commands -------------------------------------------------------------

fn ctl(req: Value) -> io::Result<Value> {
    let mut s = UnixStream::connect(sock_path()).map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "Nexa Link isn't running."))?;
    writeln!(s, "{req}")?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

struct Out {
    json: bool,
}

impl Out {
    fn say(&self, event: &str, data: Value, text: String) {
        if self.json {
            let mut v = data;
            v["event"] = json!(event);
            println!("{v}");
        } else {
            println!("{text}");
        }
        io::stdout().flush().ok();
    }
}

fn confirm(yes: bool) -> bool {
    if yes {
        return true;
    }
    let mut line = String::new();
    io::stdin().read_line(&mut line).ok();
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

fn finish_pairing(mut ch: channel::Channel, store: &Store, me: &Identity, yes: bool, out: &Out) -> io::Result<()> {
    ch.set_timeout(150);
    ch.send_json(&json!({"t": "hello", "name": me.name}))?;
    let hello = ch.recv_json()?;
    let name = clean_name(&hello["name"]);
    let id = proto::device_id(&ch.remote_public);
    let sas = proto::sas(&ch.handshake_hash);
    out.say("confirm", json!({"sas": sas, "name": name, "id": id}),
            format!("Pairing with \"{name}\". Check that it shows {sas}, then type yes:"));
    let ok = confirm(yes);
    ch.send_json(&json!({"t": "confirm", "ok": ok}))?;
    let theirs = ch.recv_json()?;
    if !(ok && theirs["ok"] == true) {
        out.say("cancelled", json!({}), "Pairing cancelled.".into());
        return Ok(());
    }
    store.add_peer(Peer {
        id: id.clone(), name: name.clone(), public: proto::hex(&ch.remote_public),
        beacon_key: proto::hex(&proto::beacon_key(&ch.handshake_hash)), files: "ask".into(), paired_at: now_secs(),
    })?;
    out.say("paired", json!({"id": id, "name": name}), format!("Paired with \"{name}\" ({id})."));
    Ok(())
}

fn pair_offer(store: &Store, yes: bool, out: &Out) -> io::Result<()> {
    let me = store.identity()?;
    let code = proto::PairCode { addr: primary_ip().to_string(), secret: random16() };
    let listener = TcpListener::bind(("0.0.0.0", pair_port()))?;
    listener.set_nonblocking(true)?;
    out.say("code", json!({"code": code.encode()}), format!("Pairing code (scan or type it on the other device):\n{}", code.encode()));
    let deadline = Instant::now() + Duration::from_secs(120);
    let psk = code.psk();
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((s, _)) => {
                s.set_nonblocking(false)?;
                // A wrong guess just fails; keep waiting for the right device.
                if let Ok(ch) = channel::accept(s, &me.secret(), Some(&psk), |_| true) {
                    return finish_pairing(ch, store, &me, yes, out);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e),
        }
    }
    out.say("expired", json!({}), "No device came to pair in 2 minutes.".into());
    Ok(())
}

fn pair_accept(store: &Store, code: &str, yes: bool, out: &Out) -> io::Result<()> {
    let me = store.identity()?;
    let code = proto::PairCode::parse(code).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "That isn't a Nexa Link pairing code."))?;
    let addr = SocketAddr::new(code.addr.parse::<IpAddr>().map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?, pair_port());
    let ch = channel::connect(addr, &me.secret(), Some(&code.psk()), |_| true)?;
    finish_pairing(ch, store, &me, yes, out)
}

/// Connect to a paired device (by id, found through the service) or, for
/// Nexa Drop, to an address.
fn open(store: &Store, target: &str) -> io::Result<(channel::Channel, String)> {
    let me = store.identity()?;
    let peer = store.peers().into_iter().find(|p| p.id == target);
    let addr: IpAddr = match target.parse() {
        Ok(a) => a,
        Err(_) => {
            peer.as_ref().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No paired device has that id."))?;
            let r = ctl(json!({"cmd": "where", "id": target}))?;
            r["addr"].as_str().and_then(|a| a.parse().ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, r["error"].as_str().unwrap_or("not nearby").to_string()))?
        }
    };
    let want = peer.as_ref().map(|p| p.public());
    let mut ch = channel::connect(SocketAddr::new(addr, port()), &me.secret(), None, |k| want.as_deref().map_or(true, |w| w == k))?;
    ch.set_timeout(30);
    ch.send_json(&json!({"t": "hello", "name": me.name}))?;
    let hello = ch.recv_json()?;
    let name = peer.map(|p| p.name).unwrap_or_else(|| clean_name(&hello["name"]));
    Ok((ch, name))
}

fn send_file(store: &Store, target: &str, path: &Path, out: &Out) -> io::Result<()> {
    let meta = fs::metadata(path)?;
    if !meta.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Only files can be sent."));
    }
    let mut hasher = Sha256::new();
    io::copy(&mut fs::File::open(path)?, &mut hasher)?;
    let sha = proto::hex(&hasher.finalize());
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (mut ch, to) = open(store, target)?;
    ch.send_json(&json!({"t": "offer", "kind": "file", "name": name, "size": meta.len(), "sha256": sha}))?;
    out.say("waiting", json!({"to": to}), format!("Waiting for \"{to}\" to accept {name}..."));
    ch.set_timeout(ASK_TIMEOUT.as_secs() + 10);
    let reply = ch.recv_json()?;
    if reply["t"] != "accept" {
        out.say("declined", json!({"to": to}), format!("\"{to}\" declined."));
        return Ok(());
    }
    let mut f = fs::File::open(path)?;
    let mut buf = vec![0u8; proto::CHUNK];
    let mut sent = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ch.send_data(&buf[..n])?;
        sent += n as u64;
    }
    ch.set_timeout(60);
    let done = ch.recv_json()?;
    ch.send_json(&json!({"t": "bye"})).ok();
    if done["t"] == "done" {
        out.say("sent", json!({"to": to, "bytes": sent, "saved": done["saved"]}), format!("Sent to \"{to}\" (saved as {}).", done["saved"].as_str().unwrap_or("?")));
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, done["why"].as_str().unwrap_or("the other device couldn't save it").to_string()))
    }
}

fn ping(store: &Store, target: &str) -> io::Result<()> {
    let started = Instant::now();
    let (mut ch, to) = open(store, target)?;
    ch.send_json(&json!({"t": "ping"}))?;
    let r = ch.recv_json()?;
    ch.send_json(&json!({"t": "bye"})).ok();
    if r["t"] == "pong" {
        println!("\"{to}\" answered in {} ms (encrypted).", started.elapsed().as_millis());
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, "no answer"))
    }
}

fn print(v: Value) -> io::Result<()> {
    println!("{v}");
    if v["ok"] == false {
        std::process::exit(1);
    }
    Ok(())
}

fn run(args: &[String]) -> io::Result<()> {
    let store = Store::open(store::default_dir())?;
    let flag = |f: &str| args.iter().any(|a| a == f);
    let pos: Vec<&str> = args.iter().skip(1).filter(|a| !a.starts_with("--")).map(String::as_str).collect();
    let out = Out { json: flag("--json") };
    match (args.first().map(String::as_str), pos.as_slice()) {
        (Some("serve"), _) => serve(store),
        (Some("id"), _) => {
            let me = store.identity()?;
            println!("{}", json!({"id": me.id(), "name": me.name}));
            Ok(())
        }
        (Some("name"), [name]) => store.set_name(name),
        (Some("pair-offer"), _) => pair_offer(&store, flag("--yes"), &out),
        (Some("pair-accept"), [code]) => pair_accept(&store, code, flag("--yes"), &out),
        (Some("send"), [target, file]) => send_file(&store, target, Path::new(file), &out),
        (Some("ping"), [target]) => ping(&store, target),
        (Some(cmd @ ("devices" | "nearby" | "status")), _) => print(ctl(json!({"cmd": cmd}))?),
        (Some("drop"), [minutes]) => print(ctl(json!({"cmd": "drop", "minutes": minutes.parse::<u64>().unwrap_or(0)}))?),
        (Some("allow"), [id, files]) => print(ctl(json!({"cmd": "allow", "id": id, "files": files}))?),
        (Some("forget"), [id]) => print(ctl(json!({"cmd": "forget", "id": id}))?),
        (Some("answer"), [offer, yes]) => print(ctl(json!({"cmd": "answer", "offer": offer.parse::<u64>().unwrap_or(0), "accept": *yes == "yes"}))?),
        (Some("watch"), _) => {
            let mut s = UnixStream::connect(sock_path())?;
            writeln!(s, "{}", json!({"cmd": "subscribe"}))?;
            for line in BufReader::new(s).lines() {
                println!("{}", line?);
                io::stdout().flush().ok();
            }
            Ok(())
        }
        _ => {
            eprintln!("usage: nexa-linkd serve | id | name NAME | pair-offer | pair-accept CODE | devices | nearby | status\n       \
                       send ID|ADDRESS FILE | ping ID | drop MINUTES | allow ID ask|always|never | forget ID | watch | answer OFFER yes|no");
            std::process::exit(2);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("nexa-linkd: {e}");
        std::process::exit(1);
    }
}
