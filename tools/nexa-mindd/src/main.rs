//! nexa-mindd: Nexa Mind, the on-device AI service of NexaOS.
//!
//!   nexa-mindd serve                   the service (OpenRC runs it as nexa-mind)
//!   nexa-mindd status | models         ask the running service
//!   nexa-mindd download <model>        fetch and verify a model
//!   nexa-mindd ask "question"          one question, answer printed
//!
//! It listens on /run/nexa-mind/mind.sock (group nexa-mind, 0660), checks
//! every caller's uid with SO_PEERCRED, confines itself with Landlock, and
//! supervises llama-server, which listens only on a private Unix socket.
//! See docs/NEXA_MIND.md.
//!
//! API (HTTP/1.1 over the socket):
//!   GET  /v1/status                    state, model, threads, memory, downloads
//!   GET  /v1/models                    catalogue with installed flags
//!   POST /v1/models/download {"name"}  start a verified download
//!   POST /v1/unload                    free the model's memory now
//!   POST /v1/warm                      load the default model ahead of use
//!   POST /v1/chat/completions          OpenAI-style chat (tools, streaming)

mod catalog;
mod config;
mod cpu;
mod http;
mod llm;
mod models;
mod peer;

use nexa_sys::sandbox;

use config::Config;
use serde_json::{json, Value};
use std::io::{BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::time::Duration;

struct Service {
    cfg: Config,
    sup: llm::Supervisor,
    downloads: models::Downloads,
    landlock: i64,
    own_uid: u32,
}

fn status(svc: &Service) -> Value {
    let (state, model) = svc.sup.status();
    json!({
        "state": state, "model": model,
        "default_model": catalog::DEFAULT_CHAT,
        "threads": svc.sup.threads, "threads_batch": svc.sup.threads_batch,
        "available_mb": cpu::available_mb(&svc.cfg.sys_root),
        "installed": models::installed(&svc.cfg),
        "downloads": *svc.downloads.lock().unwrap(),
        "landlock_abi": svc.landlock,
        "idle_unload_secs": svc.cfg.idle.as_secs(),
    })
}

fn models_list(svc: &Service) -> Value {
    Value::Array(catalog::MODELS.iter().map(|m| json!({
        "name": m.name, "title": m.title, "size": m.size, "licence": m.licence,
        "ram_mb": m.ram_mb, "role": format!("{:?}", m.role).to_lowercase(),
        "installed": models::is_installed(&svc.cfg, m),
    })).collect())
}

fn handle(svc: &Service, mut stream: UnixStream) {
    let peer = match peer::of(&stream) {
        Ok(p) => p,
        Err(_) => return,
    };
    if !peer::allowed(&peer, svc.own_uid) {
        eprintln!("nexa-mindd: refused uid {} (pid {})", peer.uid, peer.pid);
        let _ = http::respond_json(&mut stream, 403, &json!({"error": "not allowed"}));
        return;
    }
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let req = match http::read_request(&mut BufReader::new(stream.try_clone().unwrap())) {
        Ok(Some(r)) => r,
        Ok(None) => return,
        Err(e) => {
            let _ = http::respond_json(&mut stream, 400, &json!({"error": e.to_string()}));
            return;
        }
    };
    stream.set_read_timeout(None).ok();
    let result = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v1/status") => http::respond_json(&mut stream, 200, &status(svc)),
        ("GET", "/v1/models") => http::respond_json(&mut stream, 200, &models_list(svc)),
        ("POST", "/v1/unload") => {
            svc.sup.unload();
            http::respond_json(&mut stream, 200, &json!({"ok": true}))
        }
        ("POST", "/v1/models/download") => {
            let name = serde_json::from_slice::<Value>(&req.body).ok()
                .and_then(|v| v["name"].as_str().map(String::from)).unwrap_or_default();
            match catalog::find(&name) {
                None => http::respond_json(&mut stream, 404, &json!({"error": format!("unknown model '{name}'")})),
                Some(m) => {
                    let started = models::start_download(&svc.cfg, m, &svc.downloads);
                    http::respond_json(&mut stream, 202, &json!({"ok": true, "started": started}))
                }
            }
        }
        ("POST", "/v1/chat/completions") => chat(svc, req, &mut stream),
        // Nexa Patterns expects you to ask soon: load the default model now
        // (answers at once; loads in this connection's thread). It is
        // unloaded again after the usual idle time if nothing asks.
        ("POST", "/v1/warm") => match catalog::find(catalog::DEFAULT_CHAT) {
            Some(m) if models::is_installed(&svc.cfg, m) => {
                let r = http::respond_json(&mut stream, 202, &json!({"ok": true, "model": m.name}));
                drop(stream);
                if let Err(e) = svc.sup.acquire(m) {
                    eprintln!("nexa-mindd: warm-up: {e}");
                }
                r
            }
            _ => http::respond_json(&mut stream, 409, &json!({"error": "no model installed to warm up"})),
        },
        _ => http::respond_json(&mut stream, 404, &json!({"error": "no such endpoint"})),
    };
    if let Err(e) = result {
        eprintln!("nexa-mindd: uid {}: {e}", peer.uid);
    }
}

fn chat(svc: &Service, mut req: http::Request, stream: &mut UnixStream) -> std::io::Result<()> {
    let mut body: Value = match serde_json::from_slice(&req.body) {
        Ok(v @ Value::Object(_)) => v,
        _ => return http::respond_json(stream, 400, &json!({"error": "body must be a JSON object"})),
    };
    let name = body["model"].as_str().unwrap_or(catalog::DEFAULT_CHAT).to_string();
    let Some(m) = catalog::find(&name).or_else(|| catalog::find(catalog::DEFAULT_CHAT)) else {
        return http::respond_json(stream, 404, &json!({"error": "no chat model"}));
    };
    // Keep the shared prefix (system prompt + tools) cached between turns:
    // prompt reading is the slow part on a laptop CPU (docs/NEXA_MIND.md).
    body["cache_prompt"] = json!(true);
    body["model"] = json!(m.name);
    req.body = body.to_string().into_bytes();
    let _in_use = match svc.sup.acquire(m) {
        Ok(g) => g,
        Err(e) if e.contains("not installed") => {
            return http::respond_json(stream, 409, &json!({"error": e, "model": m.name, "hint": "POST /v1/models/download"}))
        }
        Err(e) => return http::respond_json(stream, 503, &json!({"error": e})),
    };
    http::proxy(&svc.cfg.llm_socket(), &req, stream).map(|_| ())
}

fn serve() -> ! {
    let cfg = Config::from_env();
    for dir in [cfg.state_dir.clone(), cfg.models_dir(), cfg.run_dir.clone()] {
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(&format!("{}: {e}", dir.display())));
    }
    std::fs::create_dir_all(cfg.private_dir()).ok();
    std::fs::set_permissions(cfg.private_dir(), std::fs::Permissions::from_mode(0o700)).ok();
    let landlock = match sandbox::restrict(&sandbox::service_rules(&cfg.state_dir.to_string_lossy(), &cfg.run_dir.to_string_lossy())) {
        Ok(0) => { eprintln!("nexa-mindd: kernel has no Landlock; running unconfined"); 0 }
        Ok(abi) => abi,
        Err(e) => die(&format!("Landlock: {e}")),
    };
    let sock = cfg.public_socket();
    std::fs::remove_file(&sock).ok();
    let listener = UnixListener::bind(&sock).unwrap_or_else(|e| die(&format!("{}: {e}", sock.display())));
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o660)).ok();
    // SAFETY: getuid can't fail.
    let own_uid = unsafe { libc::getuid() };
    let svc = Arc::new(Service { sup: llm::Supervisor::new(cfg.clone()), cfg, downloads: Default::default(), landlock, own_uid });
    eprintln!("nexa-mindd: listening on {} ({} threads, Landlock ABI {landlock})", sock.display(), svc.sup.threads);
    let reaper = svc.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(15));
        if reaper.sup.reap_if_idle() {
            eprintln!("nexa-mindd: model unloaded after {} s idle", reaper.cfg.idle.as_secs());
        }
    });
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let svc = svc.clone();
                std::thread::spawn(move || handle(&svc, stream));
            }
            Err(e) => eprintln!("nexa-mindd: accept: {e}"),
        }
    }
    exit(0)
}

fn exit(code: i32) -> ! {
    std::process::exit(code)
}

fn die(msg: &str) -> ! {
    eprintln!("nexa-mindd: {msg}");
    exit(1)
}

fn client(method: &str, path: &str, body: Value) -> (u16, Value) {
    let cfg = Config::from_env();
    let s = http::send(&cfg.public_socket(), method, path, body.to_string().as_bytes())
        .unwrap_or_else(|e| die(&format!("can't reach the service ({}): {e}", cfg.public_socket().display())));
    let (code, bytes) = http::read_response(s).unwrap_or_else(|e| die(&e.to_string()));
    (code, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(),
        Some("status") => println!("{}", serde_json::to_string_pretty(&client("GET", "/v1/status", Value::Null).1).unwrap()),
        Some("models") => println!("{}", serde_json::to_string_pretty(&client("GET", "/v1/models", Value::Null).1).unwrap()),
        Some("download") => {
            let name = args.get(1).map(String::as_str).unwrap_or(catalog::DEFAULT_CHAT);
            let (code, v) = client("POST", "/v1/models/download", json!({"name": name}));
            println!("{code} {v}");
        }
        Some("ask") => {
            let q = args[1..].join(" ");
            let (code, v) = client("POST", "/v1/chat/completions",
                                   json!({"messages": [{"role": "user", "content": q}], "max_tokens": 400}));
            if code != 200 {
                die(&format!("{code}: {v}"));
            }
            println!("{}", v["choices"][0]["message"]["content"].as_str().unwrap_or(""));
            let _ = std::io::stdout().flush();
        }
        _ => {
            eprintln!("usage: nexa-mindd serve | status | models | download [model] | ask \"question\"");
            exit(2)
        }
    }
}
