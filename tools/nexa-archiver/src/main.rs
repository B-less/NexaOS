//! nexa-archiver: compresses and extracts archives for Files and Archive.
//! One request per run, as the user:
//!
//!   echo '{"op": "extract", "archive": "/home/u/a.zip", "dest_dir": "/home/u"}' | nexa-archiver
//!
//! Ops: list (archive); extract (archive, dest_dir); compress (paths,
//! dest_dir: optional, the first item's folder; format: "zip" or "tar.gz";
//! name: optional). Reads zip, tar, and tar or single files compressed with
//! gzip, xz, bzip2 or zstd, recognised by their first bytes.
//!
//! Like nexa-fileops: long jobs print {"progress": {"done", "total",
//! "current"}} lines; the last line is {"ok": true, "result": {...}} or
//! {"ok": false, "error": "..."}. What it makes comes back with "undo"
//! steps in nexa-fileops' form, so Files' Undo can take it away.
//! SIGTERM/SIGINT stop it; a stopped job leaves nothing behind.
//!
//! Before reading anything the process confines itself with Landlock: the
//! archive or the items to pack read-only, the destination folder writable,
//! nothing else. How extraction stays inside its folder: see archive.rs.

mod archive;

use archive::Progress;
use nexa_sys::sandbox;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;

const MAX_REQUEST: u64 = 4 << 20;
const MAX_PATHS: usize = 20_000;

extern "C" fn on_signal(_: libc::c_int) {
    archive::STOP.store(true, Ordering::Relaxed);
}

/// A full path from the request, tidied; ".." refused.
fn abs(v: &Value) -> Result<PathBuf, String> {
    let s = v.as_str().ok_or("paths must be strings")?;
    let p = Path::new(s);
    if !p.is_absolute() || s.contains('\0') {
        return Err(format!("not a full path: {s}"));
    }
    if p.components().any(|c| c == Component::ParentDir) {
        return Err(format!("\"..\" isn't allowed in {s}"));
    }
    Ok(p.components().collect())
}

fn path(req: &Value, key: &str) -> Result<PathBuf, String> {
    abs(req.get(key).ok_or(format!("missing '{key}'"))?)
}

fn folder(p: PathBuf) -> Result<PathBuf, String> {
    if p.is_dir() {
        Ok(p)
    } else {
        Err(format!("{} isn't a folder", p.display()))
    }
}

fn confine(read: &[PathBuf], write: &[PathBuf]) -> Result<(), String> {
    let mut fs: Vec<(String, u64)> = read.iter().map(|p| (p.to_string_lossy().into_owned(), sandbox::READ)).collect();
    fs.extend(write.iter().map(|p| (p.to_string_lossy().into_owned(), sandbox::READ_WRITE | sandbox::MAKE_SYM)));
    sandbox::restrict(&sandbox::Rules { fs, connect_ports: vec![], any_network: false })
        .map(|_| ())
        .map_err(|e| format!("couldn't confine the archiver: {e}"))
}

fn handle(input: &[u8]) -> Result<Value, String> {
    let req: Value = serde_json::from_slice(input).map_err(|e| format!("bad request: {e}"))?;
    let op = req.get("op").and_then(Value::as_str).ok_or("missing 'op'")?;
    match op {
        "list" => {
            let a = path(&req, "archive")?;
            confine(&[a.clone()], &[])?;
            archive::list(&a).map_err(|e| e.to_string())
        }
        "extract" => {
            let a = path(&req, "archive")?;
            let dest = match req.get("dest_dir") {
                Some(v) => folder(abs(v)?)?,
                None => folder(a.parent().unwrap_or(Path::new("/")).to_path_buf())?,
            };
            let size = std::fs::metadata(&a).map_err(|e| format!("{}: {e}", a.display()))?.len();
            confine(&[a.clone()], &[dest.clone()])?;
            archive::extract(&a, &dest, &mut Progress::new(size.max(1))).map_err(|e| e.to_string())
        }
        "compress" => {
            let arr = req.get("paths").and_then(Value::as_array).ok_or("missing 'paths'")?;
            if arr.is_empty() || arr.len() > MAX_PATHS {
                return Err(format!("between 1 and {MAX_PATHS} items, please"));
            }
            let paths = arr.iter().map(abs).collect::<Result<Vec<_>, _>>()?;
            let dest = match req.get("dest_dir") {
                Some(v) => folder(abs(v)?)?,
                None => folder(paths[0].parent().unwrap_or(Path::new("/")).to_path_buf())?,
            };
            let format = req.get("format").and_then(Value::as_str).unwrap_or("zip").to_string();
            let name = req.get("name").and_then(Value::as_str).map(str::to_string);
            confine(&paths, &[dest.clone()])?;
            archive::compress(&paths, &dest, &format, name.as_deref(), &mut Progress::new(1)).map_err(|e| e.to_string())
        }
        other => Err(format!("unknown op '{other}'")),
    }
}

fn main() {
    // SAFETY: on_signal only stores to an atomic, which is signal-safe.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
    let mut input = Vec::new();
    let read = std::io::stdin().take(MAX_REQUEST).read_to_end(&mut input);
    let reply = match read.map_err(|e| e.to_string()).and_then(|_| handle(&input)) {
        Ok(v) => json!({"ok": true, "result": v}),
        Err(e) => json!({"ok": false, "error": e}),
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{reply}");
    let _ = out.flush();
    std::process::exit(if reply["ok"] == true { 0 } else { 1 });
}
