//! nexa-textio: opens and saves text files for the Editor. One request per
//! run, as the user:
//!
//!   echo '{"op": "open", "path": "/home/u/notes.txt"}' | nexa-textio
//!
//! Ops:
//! - open (path): {text, encoding, line_ending, mixed, replaced, mtime_ns,
//!   size, writable}. Any encoding (text.rs); "\n" line endings in "text".
//!   Refuses binary files and files over 64 MB.
//! - save (path, text, encoding, line_ending, expect: {mtime_ns, size} or
//!   null, force): {mtime_ns, size}. Written to a new file next to it,
//!   synced, then swapped in, keeping the permissions (and the owner when
//!   allowed); a file with several hard links, or in a folder you can't
//!   write to, is written in place. With "expect" and no "force", refuses
//!   with code "changed" (or "gone") when another program changed (or
//!   removed) the file since it was opened.
//! - draft_put (id, path, title, text, encoding, line_ending), draft_list,
//!   draft_get (id), draft_drop (id): unsaved changes kept in
//!   $XDG_DATA_HOME/nexa-edit/drafts (0700), so a crash or power cut loses
//!   nothing.
//!
//! Reply: one line, {"ok": true, "result": ...} or {"ok": false, "error":
//! "...", "code": "..."}. Before touching anything it confines itself with
//! Landlock to the file's folder (and the drafts folder).

mod text;

use nexa_sys::sandbox;
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

const MAX_TEXT: u64 = 64 << 20;
const MAX_REQUEST: u64 = 512 << 20;

struct Fail {
    error: String,
    code: &'static str,
}

impl From<String> for Fail {
    fn from(error: String) -> Self {
        Fail { error, code: "" }
    }
}
impl From<&str> for Fail {
    fn from(e: &str) -> Self {
        Fail { error: e.into(), code: "" }
    }
}

/// A plain sentence for an I/O error on `what`.
fn plain(e: &std::io::Error, what: &Path) -> String {
    let name = what.file_name().map_or_else(|| what.display().to_string(), |n| n.to_string_lossy().into_owned());
    match e.raw_os_error() {
        Some(libc::EACCES) | Some(libc::EPERM) => format!("You don't have permission for {name}. Use Save As to keep a copy somewhere else."),
        Some(libc::ENOSPC) | Some(libc::EDQUOT) => format!("The drive is full, so {name} wasn't saved. Free some space and save again."),
        Some(libc::EROFS) => format!("{name} is on a drive that can't be written to. Use Save As to keep a copy somewhere else."),
        Some(libc::ENOENT) => format!("{name} isn't there any more."),
        Some(libc::EISDIR) => format!("{name} is a folder."),
        _ => format!("{name}: {e}"),
    }
}

fn abs(req: &Value, key: &str) -> Result<PathBuf, Fail> {
    let s = req.get(key).and_then(Value::as_str).ok_or(format!("missing '{key}'"))?;
    let p = Path::new(s);
    if !p.is_absolute() || s.contains('\0') || p.components().any(|c| c == Component::ParentDir) {
        return Err(format!("not a full path: {s}").into());
    }
    Ok(p.components().collect())
}

fn stamp(m: &fs::Metadata) -> (i128, u64) {
    (m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128, m.len())
}

fn confine(read: &[&Path], write: &[&Path]) -> Result<(), Fail> {
    let mut rules: Vec<(String, u64)> = read.iter().map(|p| (p.to_string_lossy().into_owned(), sandbox::READ)).collect();
    rules.extend(write.iter().map(|p| (p.to_string_lossy().into_owned(), sandbox::READ_WRITE)));
    sandbox::restrict(&sandbox::Rules { fs: rules, connect_ports: vec![], any_network: false })
        .map(|_| ())
        .map_err(|e| format!("couldn't confine the text engine: {e}").into())
}

fn writable(p: &Path) -> bool {
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap_or_default();
    // SAFETY: access(2) with a NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

fn open(req: &Value) -> Result<Value, Fail> {
    let path = abs(req, "path")?;
    confine(&[&path], &[])?;
    let meta = fs::metadata(&path).map_err(|e| plain(&e, &path))?;
    if meta.is_dir() {
        return Err(plain(&std::io::Error::from_raw_os_error(libc::EISDIR), &path).into());
    }
    if !meta.is_file() {
        return Err("That isn't a file that can be opened as text.".into());
    }
    if meta.len() > MAX_TEXT {
        return Err(format!("This file is {} MB, too big to open here (the Editor opens files up to {} MB).",
            meta.len() >> 20, MAX_TEXT >> 20).into());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    fs::File::open(&path).and_then(|f| f.take(MAX_TEXT + 1).read_to_end(&mut bytes)).map_err(|e| plain(&e, &path))?;
    let d = text::decode(&bytes).map_err(Fail::from)?;
    let (mtime_ns, size) = stamp(&meta);
    Ok(json!({"text": d.text, "encoding": d.encoding, "line_ending": d.line_ending, "mixed": d.mixed, "replaced": d.replaced,
              "mtime_ns": mtime_ns.to_string(), "size": size, "writable": writable(&path)}))
}

fn umask() -> u32 {
    // SAFETY: umask(2) can't fail; set it straight back.
    unsafe {
        let m = libc::umask(0o022);
        libc::umask(m);
        m as u32
    }
}

fn fsync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

/// Write `bytes` to `target` so that a crash leaves either the old file or
/// the new one, never half of each.
fn write_safely(target: &Path, bytes: &[u8], existing: Option<&fs::Metadata>) -> Result<(), String> {
    let dir = target.parent().ok_or("no folder")?;
    let name = target.file_name().ok_or("no name")?.to_string_lossy().into_owned();
    // Several hard links share one file: a swap would split them. Write in place.
    let mut in_place = existing.map_or(false, |m| m.nlink() > 1);
    if !in_place {
        let mode = existing.map_or(0o666 & !umask(), |m| m.permissions().mode() & 0o7777);
        let mut tmp = None;
        for n in 0..100 {
            let t = dir.join(format!(".{name}.nexa-edit-{}-{n}", std::process::id()));
            match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&t) {
                Ok(f) => {
                    tmp = Some((t, f));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                // A folder you can't make files in, but the file itself is writable.
                Err(e) if matches!(e.raw_os_error(), Some(libc::EACCES) | Some(libc::EPERM)) && existing.is_some() && writable(target) => {
                    in_place = true;
                    break;
                }
                Err(e) => return Err(plain(&e, target)),
            }
        }
        if let Some((t, mut f)) = tmp {
            let done = (|| -> std::io::Result<()> {
                f.write_all(bytes)?;
                f.set_permissions(fs::Permissions::from_mode(mode))?;
                if let Some(m) = existing {
                    // Keeps the owner and group when allowed (it's yours, or you're root).
                    let _ = std::os::unix::fs::fchown(&f, Some(m.uid()), Some(m.gid()));
                }
                f.sync_all()?;
                fs::rename(&t, target)
            })();
            if let Err(e) = done {
                let _ = fs::remove_file(&t);
                return Err(plain(&e, target));
            }
            fsync_dir(dir);
            return Ok(());
        }
        if !in_place {
            return Err(format!("{name} couldn't be saved (no free temporary name in its folder)."));
        }
    }
    let mut f = fs::OpenOptions::new().write(true).open(target).map_err(|e| plain(&e, target))?;
    f.write_all(bytes).and_then(|_| f.set_len(bytes.len() as u64)).and_then(|_| f.sync_all()).map_err(|e| plain(&e, target))
}

fn save(req: &Value) -> Result<Value, Fail> {
    let path = abs(req, "path")?;
    let text = req.get("text").and_then(Value::as_str).ok_or("missing 'text'")?;
    let encoding = req.get("encoding").and_then(Value::as_str).unwrap_or("utf-8");
    let line_ending = req.get("line_ending").and_then(Value::as_str).unwrap_or("lf");
    let force = req.get("force").and_then(Value::as_bool).unwrap_or(false);
    let expect = req.get("expect").filter(|v| !v.is_null());
    if text.len() as u64 > MAX_TEXT * 2 {
        return Err("This text is too big to save.".into());
    }
    let bytes = text::encode(text, encoding, line_ending).map_err(Fail::from)?;
    // A link is followed: the file it points to is the one saved.
    let target = match fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(&path).map_err(|e| plain(&e, &path))?,
        _ => path.clone(),
    };
    let dir = target.parent().ok_or("no folder")?.to_path_buf();
    confine(&[], &[&dir])?;
    let existing = match fs::metadata(&target) {
        Ok(m) if !m.is_file() => return Err(format!("{} isn't a regular file.", target.display()).into()),
        Ok(m) => Some(m),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(plain(&e, &target).into()),
    };
    if let (Some(want), false) = (expect, force) {
        let want = (want.get("mtime_ns").and_then(Value::as_str).and_then(|s| s.parse::<i128>().ok()),
                    want.get("size").and_then(Value::as_u64));
        match &existing {
            None => return Err(Fail { error: "The file was deleted or moved by another program since you opened it.".into(), code: "gone" }),
            Some(m) if (Some(stamp(m).0), Some(stamp(m).1)) != want => {
                return Err(Fail { error: "Another program changed this file since you opened it.".into(), code: "changed" });
            }
            _ => {}
        }
    }
    write_safely(&target, &bytes, existing.as_ref()).map_err(Fail::from)?;
    let m = fs::metadata(&target).map_err(|e| plain(&e, &target))?;
    let (mtime_ns, size) = stamp(&m);
    Ok(json!({"mtime_ns": mtime_ns.to_string(), "size": size, "path": target.to_string_lossy()}))
}

// ---------- drafts ----------

fn drafts_dir() -> Result<PathBuf, Fail> {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("no home folder")?;
    Ok(base.join("nexa-edit/drafts"))
}

fn draft_id(req: &Value) -> Result<String, Fail> {
    let id = req.get("id").and_then(Value::as_str).ok_or("missing 'id'")?;
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("bad draft id".into());
    }
    Ok(id.to_string())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn drafts(op: &str, req: &Value) -> Result<Value, Fail> {
    let dir = drafts_dir()?;
    fs::create_dir_all(&dir).map_err(|e| plain(&e, &dir))?;
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    confine(&[], &[&dir])?;
    match op {
        "draft_put" => {
            let id = draft_id(req)?;
            let text = req.get("text").and_then(Value::as_str).ok_or("missing 'text'")?;
            if text.len() as u64 > MAX_TEXT * 2 {
                return Err("too big to keep a draft of".into());
            }
            let d = json!({"id": id, "path": req.get("path").cloned().unwrap_or(Value::Null),
                           "title": req.get("title").and_then(Value::as_str).unwrap_or(""), "text": text,
                           "encoding": req.get("encoding").and_then(Value::as_str).unwrap_or("utf-8"),
                           "line_ending": req.get("line_ending").and_then(Value::as_str).unwrap_or("lf"), "saved": now()});
            let target = dir.join(format!("{id}.json"));
            write_safely(&target, d.to_string().as_bytes(), None).map_err(Fail::from)?;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o600));
            Ok(json!({"id": id}))
        }
        "draft_list" => {
            let mut out: Vec<Value> = Vec::new();
            for e in fs::read_dir(&dir).map_err(|e| plain(&e, &dir))?.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("json") {
                    continue;
                }
                let Ok(raw) = fs::read(&p) else { continue };
                let Ok(v) = serde_json::from_slice::<Value>(&raw) else { continue };
                out.push(json!({"id": v["id"], "path": v["path"], "title": v["title"], "saved": v["saved"],
                                "bytes": v["text"].as_str().map_or(0, str::len)}));
            }
            out.sort_by_key(|v| v["saved"].as_u64().unwrap_or(0));
            Ok(Value::Array(out))
        }
        "draft_get" => {
            let p = dir.join(format!("{}.json", draft_id(req)?));
            let raw = fs::read(&p).map_err(|e| plain(&e, &p))?;
            serde_json::from_slice(&raw).map_err(|_| "That draft is damaged.".into())
        }
        "draft_drop" => {
            let p = dir.join(format!("{}.json", draft_id(req)?));
            match fs::remove_file(&p) {
                Ok(()) => Ok(json!({"dropped": true})),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({"dropped": false})),
                Err(e) => Err(plain(&e, &p).into()),
            }
        }
        _ => unreachable!(),
    }
}

fn handle(input: &[u8]) -> Result<Value, Fail> {
    let req: Value = serde_json::from_slice(input).map_err(|e| format!("bad request: {e}"))?;
    match req.get("op").and_then(Value::as_str).ok_or("missing 'op'")? {
        "open" => open(&req),
        "save" => save(&req),
        op @ ("draft_put" | "draft_list" | "draft_get" | "draft_drop") => drafts(op, &req),
        "encodings" => Ok(json!(text::ENCODINGS)),
        other => Err(format!("unknown op {other}").into()),
    }
}

fn main() {
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().take(MAX_REQUEST).read_to_end(&mut input) {
        println!("{}", json!({"ok": false, "error": format!("couldn't read the request: {e}")}));
        std::process::exit(1);
    }
    let reply = match handle(&input) {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(f) => json!({"ok": false, "error": f.error, "code": f.code}),
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{reply}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nexa-textio-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn safe_saves() {
        let d = tmp("save");
        let f = d.join("a.txt");
        fs::write(&f, b"old\r\n").unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o640)).unwrap();
        let m = fs::metadata(&f).unwrap();
        write_safely(&f, b"new\r\n", Some(&m)).unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"new\r\n");
        assert_eq!(fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o640);
        // No temporary file left behind.
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1);
        // Hard links stay one file.
        let g = d.join("b.txt");
        fs::hard_link(&f, &g).unwrap();
        let m = fs::metadata(&f).unwrap();
        write_safely(&f, b"both", Some(&m)).unwrap();
        assert_eq!(fs::read(&g).unwrap(), b"both");
        // A new file.
        write_safely(&d.join("c.txt"), b"c", None).unwrap();
        assert_eq!(fs::read(d.join("c.txt")).unwrap(), b"c");
        // A link: the file it points to is saved, the link stays.
        symlink(&f, d.join("link.txt")).unwrap();
        let t = fs::canonicalize(d.join("link.txt")).unwrap();
        write_safely(&t, b"via link", fs::metadata(&t).ok().as_ref()).unwrap();
        assert!(fs::symlink_metadata(d.join("link.txt")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&f).unwrap(), b"via link");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn plain_words() {
        let p = Path::new("/x/report.txt");
        assert!(plain(&std::io::Error::from_raw_os_error(libc::ENOSPC), p).contains("drive is full"));
        assert!(plain(&std::io::Error::from_raw_os_error(libc::EACCES), p).contains("Save As"));
    }
}
