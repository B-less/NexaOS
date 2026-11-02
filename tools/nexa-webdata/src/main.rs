//! nexa-webdata: the browser's data work, one request per run, as the user.
//!
//!   echo '{"op": "reader", "html": "...", "url": "https://..."}' | nexa-webdata
//!
//! Ops:
//! - import_bookmarks (path): bookmarks from another browser's HTML export:
//!   {items: [{title, url, folder, added}], skipped}.
//! - export_bookmarks (path, items): writes such a file.
//! - reader (html, url): the article in a page, rewritten with safe tags:
//!   {title, byline, html, words, minutes}; code "no_article" if there
//!   isn't one. Runs with no access to any file.
//! - vault_status; vault_list (primary); vault_find (origin, primary);
//!   vault_save (origin, username, password, primary); vault_delete (id,
//!   primary); vault_never (origin, add, primary); vault_primary (primary,
//!   new): saved passwords, encrypted (vault.rs), in
//!   $XDG_DATA_HOME/nexa-browser. The primary password, when there is one,
//!   comes in the request (standard input), never on the command line.
//!
//! Reply: {"ok": true, "result": ...} or {"ok": false, "error": "...",
//! "code": "..."}. Landlock confines each op to the files it needs.

mod bookmarks;
mod html;
mod reader;
mod vault;

use nexa_sys::sandbox;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

type Res = Result<Value, vault::Fail>;

fn err(code: &'static str, e: impl Into<String>) -> vault::Fail {
    vault::Fail { error: e.into(), code }
}

fn confine(rules: Vec<(&Path, u64)>) -> Result<(), vault::Fail> {
    let fs = rules.into_iter().map(|(p, r)| (p.to_string_lossy().into_owned(), r)).collect();
    sandbox::restrict(&sandbox::Rules { fs, connect_ports: vec![], any_network: false })
        .map(|_| ())
        .map_err(|e| err("confine", format!("couldn't confine the browser's data engine: {e}")))
}

fn abs(req: &Value, key: &str) -> Result<PathBuf, vault::Fail> {
    let s = req.get(key).and_then(Value::as_str).ok_or(err("request", format!("missing '{key}'")))?;
    let p = Path::new(s);
    if !p.is_absolute() || s.contains('\0') || p.components().any(|c| c == Component::ParentDir) {
        return Err(err("request", format!("not a full path: {s}")));
    }
    Ok(p.components().collect())
}

fn text<'a>(req: &'a Value, key: &str) -> Result<&'a str, vault::Fail> {
    req.get(key).and_then(Value::as_str).ok_or(err("request", format!("missing '{key}'")))
}

/// "https://host[:port]" only.
fn origin(req: &Value) -> Result<String, vault::Fail> {
    let o = text(req, "origin")?.trim().to_ascii_lowercase();
    let rest = o.strip_prefix("https://").or_else(|| o.strip_prefix("http://")).ok_or(err("request", "not a web address"))?;
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', ' ']) || o.len() > 300 {
        return Err(err("request", "not a site's address"));
    }
    Ok(o)
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join(".local/share"))
        .join("nexa-browser")
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn vault_op(op: &str, req: &Value) -> Res {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| err("io", format!("can't make {}: {e}", dir.display())))?;
    confine(vec![(dir.as_path(), sandbox::READ_WRITE)])?;
    let v = vault::Vault { dir, iterations: vault::ITERATIONS };
    let primary = req.get("primary").and_then(Value::as_str).filter(|s| !s.is_empty());
    match op {
        "vault_status" => Ok(v.status()),
        "vault_list" => {
            let d = v.open(primary)?;
            let entries: Vec<Value> = d["entries"].as_array().into_iter().flatten()
                .map(|e| json!({"id": e["id"], "origin": e["origin"], "username": e["username"], "created": e["created"], "updated": e["updated"]}))
                .collect();
            Ok(json!({"entries": entries, "never": d["never"]}))
        }
        "vault_find" => {
            let o = origin(req)?;
            let d = v.open(primary)?;
            let never = d["never"].as_array().map_or(false, |n| n.iter().any(|x| x == &json!(o)));
            let found: Vec<Value> = d["entries"].as_array().into_iter().flatten().filter(|e| e["origin"] == o.as_str())
                .map(|e| json!({"id": e["id"], "username": e["username"], "password": e["password"]})).collect();
            Ok(json!({"entries": found, "never": never}))
        }
        "vault_get" => {
            let id = text(req, "id")?;
            let d = v.open(primary)?;
            d["entries"].as_array().into_iter().flatten().find(|e| e["id"] == id)
                .map(|e| json!({"password": e["password"]})).ok_or(err("missing", "That saved password isn't there any more."))
        }
        "vault_save" => {
            let (o, user, pass) = (origin(req)?, text(req, "username")?, text(req, "password")?);
            if pass.is_empty() || pass.len() > 4096 || user.len() > 1024 {
                return Err(err("request", "nothing to save"));
            }
            v.save(primary, &o, user, pass, now())
        }
        "vault_delete" => v.delete(primary, text(req, "id")?),
        "vault_never" => v.never(primary, &origin(req)?, req.get("add").and_then(Value::as_bool).unwrap_or(true)),
        "vault_primary" => {
            let new = req.get("new").and_then(Value::as_str).filter(|s| !s.is_empty());
            if new.map_or(false, |n| n.chars().count() < 8) {
                return Err(err("weak", "Use at least 8 characters for the primary password."));
            }
            v.set_primary(primary, new)
        }
        _ => unreachable!(),
    }
}

fn handle(input: &[u8]) -> Res {
    let req: Value = serde_json::from_slice(input).map_err(|e| err("request", format!("bad request: {e}")))?;
    let op = text(&req, "op")?.to_string();
    match op.as_str() {
        "reader" => {
            confine(vec![])?; // reads nothing, writes nothing
            let page = text(&req, "html")?;
            let url = text(&req, "url")?;
            let a = reader::extract(page, url).ok_or(err("no_article", "This page doesn't look like an article."))?;
            Ok(json!({"title": a.title, "byline": a.byline, "html": a.html, "words": a.words, "minutes": (a.words + 199) / 200}))
        }
        "import_bookmarks" => {
            let p = abs(&req, "path")?;
            confine(vec![(p.as_path(), sandbox::READ)])?;
            let mut raw = Vec::new();
            std::fs::File::open(&p).and_then(|f| f.take(64 << 20).read_to_end(&mut raw))
                .map_err(|e| err("io", format!("That file can't be read: {e}")))?;
            let (items, skipped) = bookmarks::import(&String::from_utf8_lossy(&raw));
            if items.is_empty() {
                return Err(err("empty", "No bookmarks found in that file. Export them from the other browser as an HTML file."));
            }
            Ok(json!({"items": items, "skipped": skipped}))
        }
        "export_bookmarks" => {
            let p = abs(&req, "path")?;
            let dir = p.parent().ok_or(err("request", "no folder"))?.to_path_buf();
            confine(vec![(dir.as_path(), sandbox::READ_WRITE)])?;
            let items = req.get("items").and_then(Value::as_array).ok_or(err("request", "missing 'items'"))?;
            std::fs::write(&p, bookmarks::export(items)).map_err(|e| err("io", format!("The bookmarks weren't saved: {e}")))?;
            Ok(json!({"path": p, "count": items.len()}))
        }
        op if op.starts_with("vault_") && ["vault_status", "vault_list", "vault_find", "vault_get", "vault_save", "vault_delete", "vault_never", "vault_primary"].contains(&op) => vault_op(op, &req),
        other => Err(err("request", format!("unknown op {other}"))),
    }
}

fn main() {
    let mut input = Vec::new();
    let _ = std::io::stdin().take(80 << 20).read_to_end(&mut input);
    let reply = match handle(&input) {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(f) => json!({"ok": false, "error": f.error, "code": f.code}),
    };
    let _ = writeln!(std::io::stdout().lock(), "{reply}");
}
