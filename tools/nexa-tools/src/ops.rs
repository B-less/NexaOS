//! The assistant's file actions. `prepare` checks a request and works out
//! exactly which folders it needs; main.rs then confines the process to
//! those with Landlock; `run` performs it. Changes are journaled (undo).

use crate::journal::{Journal, Step};
use crate::paths::{allowed, is_sensitive, not_found};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAX_WRITE: usize = 5 * 1024 * 1024;
pub const MAX_READ: u64 = 256 * 1024;
const MAX_WALK: usize = 20_000;

/// What a request may touch once the sandbox is on.
#[derive(Debug, Default)]
pub struct Access {
    pub read: Vec<PathBuf>,
    pub write: Vec<PathBuf>,
}

pub struct Request {
    pub tool: String,
    pub args: Value,
    pub paths: BTreeMap<&'static str, PathBuf>,
    pub moves: Vec<(PathBuf, PathBuf)>,
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| format!("missing '{key}'"))
}

/// Nearest existing folder at or above `p` (Landlock rules need one).
pub fn existing_dir(p: &Path) -> PathBuf {
    let mut d = p.to_path_buf();
    while !d.is_dir() {
        if !d.pop() {
            return PathBuf::from("/");
        }
    }
    d
}

fn parent(p: &Path) -> PathBuf {
    existing_dir(p.parent().unwrap_or(p))
}

/// Check the request and decide what it may touch.
pub fn prepare(home: &Path, j: &Journal, tool: &str, args: Value) -> Result<(Request, Access), String> {
    let path = |key| -> Result<PathBuf, String> { allowed(home, arg(&args, key)?).map_err(|e| e.to_string()) };
    let mut req = Request { tool: tool.into(), args: args.clone(), paths: BTreeMap::new(), moves: Vec::new() };
    let mut acc = Access::default();
    let records = j.mind_dir();
    match tool {
        "list_dir" | "read_text" | "search" | "find_duplicates" | "organize_plan" => {
            let p = if tool == "search" && args.get("path").is_none() { home.to_path_buf() } else { path("path")? };
            if !p.exists() {
                return Err(not_found(home, &p));
            }
            if tool != "read_text" && !p.is_dir() {
                return Err(format!("{} is a file, not a folder", p.strip_prefix(home).map(|r| format!("~/{}", r.display())).unwrap_or_default()));
            }
            if tool == "read_text" && p.is_dir() {
                return Err("that's a folder; use list_dir to see what's in it".into());
            }
            acc.read.push(if p.is_dir() { p.clone() } else { parent(&p) });
            req.paths.insert("path", p);
        }
        "write_text" | "make_dir" => {
            let p = path("path")?;
            acc.write.push(parent(&p));
            acc.write.push(records.clone());
            req.paths.insert("path", p);
        }
        "trash" => {
            let p = path("path")?;
            if p == home {
                return Err("the home folder itself can't be deleted".into());
            }
            if !p.exists() {
                return Err(not_found(home, &p));
            }
            acc.write.push(parent(&p));
            acc.write.push(j.trash_dir().parent().unwrap().to_path_buf());
            acc.write.push(records.clone());
            req.paths.insert("path", p);
        }
        "move" | "copy" => {
            let (from, to) = (path("from")?, path("to")?);
            if !from.exists() {
                return Err(not_found(home, &from));
            }
            acc.write.push(parent(&from));
            acc.write.push(parent(&to));
            if from.is_dir() {
                acc.write.push(from.clone());
            }
            acc.write.push(records.clone());
            req.paths.insert("from", from);
            req.paths.insert("to", to);
        }
        "apply_moves" => {
            let list = args.get("moves").and_then(Value::as_array).ok_or("missing 'moves'")?;
            if list.is_empty() || list.len() > 5000 {
                return Err("between 1 and 5000 moves, please".into());
            }
            for m in list {
                let from = allowed(home, arg(m, "from")?).map_err(|e| e.to_string())?;
                let to = allowed(home, arg(m, "to")?).map_err(|e| e.to_string())?;
                acc.write.push(parent(&from));
                acc.write.push(parent(&to));
                req.moves.push((from, to));
            }
            acc.write.push(records.clone());
        }
        "undo" => {
            let entry = j.load(arg(&args, "id")?)?;
            for step in &entry.steps {
                let ps: Vec<&PathBuf> = match step {
                    Step::Created { path } => vec![path],
                    Step::Replaced { path, backup } => vec![path, backup],
                    Step::Moved { from, to } => vec![from, to],
                    Step::Trashed { path, trashed, info } => vec![path, trashed, info],
                };
                for p in ps {
                    acc.write.push(parent(p));
                }
            }
            acc.write.push(j.trash_dir().parent().unwrap().to_path_buf());
            acc.write.push(records.clone());
        }
        "history" => acc.read.push(records.clone()),
        "net_info" => acc.read.extend(crate::net::READ_PATHS.iter().map(PathBuf::from)),
        other => return Err(format!("unknown tool '{other}'")),
    }
    Ok((req, acc))
}

fn kind(md: &fs::Metadata) -> &'static str {
    if md.is_dir() { "folder" } else if md.file_type().is_symlink() { "link" } else { "file" }
}

/// Every regular file under `root`, skipping hidden and sensitive folders.
fn walk(home: &Path, root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let hidden = e.file_name().to_string_lossy().starts_with('.');
            let Ok(md) = fs::symlink_metadata(&p) else { continue };
            if md.is_dir() {
                if !hidden && depth < max_depth && !is_sensitive(home, &p) {
                    stack.push((p, depth + 1));
                }
            } else if md.is_file() && !hidden {
                out.push(p);
                if out.len() >= MAX_WALK {
                    return out;
                }
            }
        }
    }
    out
}

fn category(p: &Path) -> &'static str {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "svg" | "bmp" | "tiff" => "Pictures",
        "mp4" | "mkv" | "webm" | "mov" | "avi" => "Videos",
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "opus" => "Music",
        "pdf" | "doc" | "docx" | "odt" | "txt" | "md" | "rtf" | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "odp" | "csv" => "Documents",
        "zip" | "tar" | "gz" | "xz" | "7z" | "rar" | "zst" => "Archives",
        "iso" | "img" | "nexa" | "apk" | "deb" | "appimage" => "Installers",
        _ => "Other",
    }
}

fn sha256(p: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn new_entry(j: &Journal) -> Result<(String, PathBuf), String> {
    j.new_id().map_err(|e| format!("can't record undo information: {e}"))
}

/// Perform a prepared request (inside the sandbox).
pub fn run(home: &Path, j: &Journal, req: Request) -> Result<Value, String> {
    let p = |k: &str| req.paths.get(k).cloned().unwrap();
    let rel = |p: &Path| p.strip_prefix(home).map(|r| format!("~/{}", r.display())).unwrap_or_else(|_| p.display().to_string());
    match req.tool.as_str() {
        "list_dir" => {
            let dir = p("path");
            let mut items = Vec::new();
            for e in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", rel(&dir)))?.flatten() {
                if let Ok(md) = e.metadata() {
                    items.push(json!({"name": e.file_name().to_string_lossy(), "type": kind(&md), "size": md.len()}));
                }
            }
            items.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            Ok(json!({"path": rel(&dir), "entries": items}))
        }
        "read_text" => {
            let f = p("path");
            let max = req.args.get("max_bytes").and_then(Value::as_u64).unwrap_or(MAX_READ).min(MAX_READ);
            let mut buf = Vec::new();
            fs::File::open(&f).and_then(|h| h.take(max + 1).read_to_end(&mut buf)).map_err(|e| format!("{}: {e}", rel(&f)))?;
            let truncated = buf.len() as u64 > max;
            buf.truncate(max as usize);
            if buf.contains(&0) {
                return Err(format!("{} isn't a text file", rel(&f)));
            }
            Ok(json!({"path": rel(&f), "text": String::from_utf8_lossy(&buf), "truncated": truncated}))
        }
        "search" => {
            let root = p("path");
            let needle = arg(&req.args, "pattern")?.to_lowercase();
            let limit = req.args.get("limit").and_then(Value::as_u64).unwrap_or(50).min(500) as usize;
            let hits: Vec<String> = walk(home, &root, 12).into_iter()
                .filter(|f| f.file_name().map_or(false, |n| n.to_string_lossy().to_lowercase().contains(&needle)))
                .take(limit).map(|f| rel(&f)).collect();
            Ok(json!({"matches": hits}))
        }
        "find_duplicates" => {
            let mut by_size: BTreeMap<u64, Vec<PathBuf>> = BTreeMap::new();
            for f in walk(home, &p("path"), 12) {
                if let Ok(md) = fs::metadata(&f) {
                    if md.len() > 0 {
                        by_size.entry(md.len()).or_default().push(f);
                    }
                }
            }
            let mut groups = Vec::new();
            let mut wasted = 0u64;
            for (size, files) in by_size.into_iter().filter(|(_, v)| v.len() > 1) {
                let mut by_hash: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for f in files {
                    if let Ok(h) = sha256(&f) {
                        by_hash.entry(h).or_default().push(rel(&f));
                    }
                }
                for (_, same) in by_hash.into_iter().filter(|(_, v)| v.len() > 1) {
                    wasted += size * (same.len() as u64 - 1);
                    groups.push(json!({"size": size, "files": same}));
                }
            }
            Ok(json!({"groups": groups, "reclaimable_bytes": wasted}))
        }
        "organize_plan" => {
            let root = p("path");
            let by = req.args.get("by").and_then(Value::as_str).unwrap_or("type");
            let mut moves = Vec::new();
            for e in fs::read_dir(&root).map_err(|e| e.to_string())?.flatten() {
                let f = e.path();
                let Ok(md) = fs::symlink_metadata(&f) else { continue };
                if !md.is_file() || e.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let folder = match by {
                    "type" => category(&f).to_string(),
                    "month" => {
                        let secs = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                        crate::journal::trash_date(secs)[..7].to_string()
                    }
                    other => return Err(format!("can't organise by '{other}' (type or month)")),
                };
                let to = root.join(&folder).join(e.file_name());
                if !to.exists() {
                    moves.push(json!({"from": rel(&f), "to": rel(&to)}));
                }
            }
            moves.sort_by(|a, b| a["to"].as_str().cmp(&b["to"].as_str()));
            Ok(json!({"moves": moves, "summary": format!("Move {} files in {} into folders by {by}", moves.len(), rel(&root)),
                      "note": "Nothing has moved yet: run apply_moves with these moves to do it."}))
        }
        "write_text" => {
            let f = p("path");
            let content = arg(&req.args, "content")?;
            if content.len() > MAX_WRITE {
                return Err("that's more than 5 MB of text".into());
            }
            let mode = req.args.get("mode").and_then(Value::as_str).unwrap_or("create");
            let (id, dir) = new_entry(j)?;
            let mut steps = Vec::new();
            let existed = f.exists();
            match (mode, existed) {
                ("create", true) => return Err(format!("{} already exists; say overwrite or append", rel(&f))),
                ("overwrite" | "append", true) => {
                    let backup = dir.join("before");
                    fs::copy(&f, &backup).map_err(|e| e.to_string())?;
                    steps.push(Step::Replaced { path: f.clone(), backup });
                }
                ("create" | "overwrite" | "append", false) => {
                    if let Some(parent) = f.parent() {
                        if !parent.exists() {
                            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            steps.push(Step::Created { path: parent.into() });
                        }
                    }
                    steps.push(Step::Created { path: f.clone() });
                }
                (other, _) => return Err(format!("unknown mode '{other}' (create, overwrite or append)")),
            }
            let mut h = fs::OpenOptions::new().write(true).create(true).append(mode == "append").truncate(mode != "append")
                .open(&f).map_err(|e| format!("{}: {e}", rel(&f)))?;
            h.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
            let verb = if existed { if mode == "append" { "Added to" } else { "Replaced" } } else { "Created" };
            let summary = format!("{verb} {}", rel(&f));
            j.save(&Journal::entry(id.clone(), summary.clone(), steps)).map_err(|e| e.to_string())?;
            Ok(json!({"done": summary, "undo_id": id, "bytes": content.len()}))
        }
        "make_dir" => {
            let d = p("path");
            if d.exists() {
                return Ok(json!({"done": format!("{} already exists", rel(&d))}));
            }
            let mut created = Vec::new();
            let mut probe = d.clone();
            while !probe.exists() {
                created.push(probe.clone());
                if !probe.pop() {
                    break;
                }
            }
            fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            let (id, _) = new_entry(j)?;
            let summary = format!("Created folder {}", rel(&d));
            let steps = created.into_iter().rev().map(|path| Step::Created { path }).collect();
            j.save(&Journal::entry(id.clone(), summary.clone(), steps)).map_err(|e| e.to_string())?;
            Ok(json!({"done": summary, "undo_id": id}))
        }
        "move" | "copy" => {
            let (from, to) = (p("from"), p("to"));
            let to = if to.is_dir() { to.join(from.file_name().ok_or("nothing to move")?) } else { to };
            if to.exists() {
                return Err(format!("{} already exists", rel(&to)));
            }
            if !from.exists() {
                return Err(format!("{} doesn't exist", rel(&from)));
            }
            let (id, _) = new_entry(j)?;
            let step = if req.tool == "move" {
                fs::rename(&from, &to).map_err(|e| format!("{e}"))?;
                Step::Moved { from: from.clone(), to: to.clone() }
            } else {
                if from.is_dir() {
                    return Err("copying folders isn't supported yet; copy the files inside".into());
                }
                fs::copy(&from, &to).map_err(|e| e.to_string())?;
                Step::Created { path: to.clone() }
            };
            let summary = format!("{} {} to {}", if req.tool == "move" { "Moved" } else { "Copied" }, rel(&from), rel(&to));
            j.save(&Journal::entry(id.clone(), summary.clone(), vec![step])).map_err(|e| e.to_string())?;
            Ok(json!({"done": summary, "undo_id": id}))
        }
        "apply_moves" => {
            let (id, _) = new_entry(j)?;
            let mut steps = Vec::new();
            let mut skipped = Vec::new();
            for (from, to) in &req.moves {
                if !from.exists() || to.exists() {
                    skipped.push(rel(from));
                    continue;
                }
                if let Some(parent) = to.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                        steps.push(Step::Created { path: parent.into() });
                    }
                }
                fs::rename(from, to).map_err(|e| e.to_string())?;
                steps.push(Step::Moved { from: from.clone(), to: to.clone() });
            }
            let moved = steps.iter().filter(|s| matches!(s, Step::Moved { .. })).count();
            let summary = format!("Moved {moved} files");
            j.save(&Journal::entry(id.clone(), summary.clone(), steps)).map_err(|e| e.to_string())?;
            Ok(json!({"done": summary, "undo_id": id, "skipped": skipped}))
        }
        "trash" => {
            let f = p("path");
            if !f.exists() {
                return Err(format!("{} doesn't exist", rel(&f)));
            }
            let (id, _) = new_entry(j)?;
            let step = j.trash(&f).map_err(|e| e.to_string())?;
            let summary = format!("Moved {} to the Trash", rel(&f));
            j.save(&Journal::entry(id.clone(), summary.clone(), vec![step])).map_err(|e| e.to_string())?;
            Ok(json!({"done": summary, "undo_id": id}))
        }
        "undo" => {
            let entry = j.undo(arg(&req.args, "id")?)?;
            Ok(json!({"done": format!("Undid: {}", entry.summary)}))
        }
        "history" => {
            let limit = req.args.get("limit").and_then(Value::as_u64).unwrap_or(20).min(200) as usize;
            Ok(json!({"actions": j.recent(limit).iter().map(|e| json!({"id": e.id, "time": e.time, "summary": e.summary, "undone": e.undone})).collect::<Vec<_>>()}))
        }
        "net_info" => Ok(crate::net::info()),
        other => Err(format!("unknown tool '{other}'")),
    }
}
