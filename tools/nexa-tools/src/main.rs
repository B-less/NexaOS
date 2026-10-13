//! nexa-tools: the assistant's hands. One action per run, as the user:
//!
//!   echo '{"tool": "write_text", "args": {"path": "~/Notes/todo.txt", "content": "..."}}' | nexa-tools
//!
//! Each run checks the request (paths must be inside home, never keys or
//! Nexa Mind's own records), confines itself with Landlock to exactly the
//! folders the request needs (no network at all), performs it, records it
//! for undo and in the activity log, and prints one JSON object:
//!   {"ok": true, "result": {...}}  or  {"ok": false, "error": "..."}
//!
//! Tools: list_dir, read_text, search, find_duplicates, organize_plan
//! (read only); write_text (create / overwrite / append), make_dir, move,
//! copy, trash, apply_moves (journaled, undoable); undo, history.
//! See docs/NEXA_MIND.md (M3).

mod journal;
mod ops;
mod paths;

use nexa_sys::sandbox;
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

const MAX_REQUEST: u64 = 6 * 1024 * 1024;

fn home() -> Result<PathBuf, String> {
    // HOME of the user running us; the account is the trust boundary, not
    // this variable (a caller able to set it is already that user).
    let h = std::env::var_os("HOME").map(PathBuf::from).ok_or("HOME is not set")?;
    h.canonicalize().map_err(|e| format!("home folder: {e}"))
}

fn handle(input: &[u8]) -> Result<Value, String> {
    let req: Value = serde_json::from_slice(input).map_err(|e| format!("bad request: {e}"))?;
    let tool = req.get("tool").and_then(Value::as_str).ok_or("missing 'tool'")?.to_string();
    let args = req.get("args").cloned().unwrap_or_else(|| json!({}));
    let home = home()?;
    let j = journal::Journal { home: home.clone() };
    let (request, access) = ops::prepare(&home, &j, &tool, args)?;

    // Create what the sandbox rules need to point at, then lock down.
    std::fs::create_dir_all(j.mind_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(j.trash_dir()).map_err(|e| e.to_string())?;
    let mut fs_rules: Vec<(String, u64)> = ["/usr", "/lib"].iter().map(|p| (p.to_string(), sandbox::READ_EXEC)).collect();
    fs_rules.push(("/etc/localtime".into(), sandbox::READ));
    for p in &access.read {
        fs_rules.push((p.to_string_lossy().into(), sandbox::READ));
    }
    for p in &access.write {
        fs_rules.push((p.to_string_lossy().into(), sandbox::READ_WRITE));
    }
    let abi = sandbox::restrict(&sandbox::Rules { fs: fs_rules, connect_ports: vec![] })
        .map_err(|e| format!("couldn't enable the sandbox: {e}"))?;
    if abi == 0 && std::env::var_os("NEXA_TOOLS_ALLOW_UNSANDBOXED").is_none() {
        return Err("this kernel has no Landlock, so the assistant's file actions are disabled".into());
    }
    ops::run(&home, &j, request)
}

fn main() {
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().take(MAX_REQUEST).read_to_end(&mut input) {
        println!("{}", json!({"ok": false, "error": e.to_string()}));
        std::process::exit(1);
    }
    match handle(&input) {
        Ok(result) => println!("{}", json!({"ok": true, "result": result})),
        Err(error) => {
            println!("{}", json!({"ok": false, "error": error}));
            std::process::exit(1);
        }
    }
}
