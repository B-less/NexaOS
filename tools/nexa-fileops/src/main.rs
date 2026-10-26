//! nexa-fileops: the Files app's engine. One request per run, as the user:
//!
//!   echo '{"op": "copy", "sources": ["/home/u/a.txt"], "dest_dir": "/media/STICK"}' | nexa-fileops
//!
//! Long copies and moves print progress lines first
//! ({"progress": {"done": bytes, "total": bytes, "current": name}}); the
//! last line is always {"ok": true, "result": {...}} or {"ok": false,
//! "error": "..."}. Changes come back with "undo": steps to send as
//! {"op": "undo", "steps": [...]} to reverse them.
//!
//! Ops: copy, move (sources, dest_dir); duplicate, trash, delete, size
//! (paths); rename (path, name); new_folder, new_file (dir, name); restore
//! (items: paths inside a Trash); trash_list; empty_trash; undo (steps);
//! trail (query, limit): FileTrail, where things went; trail_forget.
//!
//! Every change is written to FileTrail's journal (nexa_sys::filetrail)
//! unless it is switched off in Settings > Magic Layer.
//!
//! Nothing is ever overwritten (see ops.rs), and before doing anything the
//! process confines itself with Landlock to the folders the request names,
//! read-only where it only reads.

mod ops;
mod trash;

use nexa_sys::sandbox;
use ops::{place, Failures, Progress, Step};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

const MAX_REQUEST: u64 = 8 << 20;
const MAX_PATHS: usize = 20_000;

enum Job {
    Copy { sources: Vec<PathBuf>, dest: PathBuf },
    Move { sources: Vec<PathBuf>, dest: PathBuf },
    Duplicate(Vec<PathBuf>),
    Trash(Vec<(PathBuf, trash::TrashDir)>),
    Delete(Vec<PathBuf>),
    Size(Vec<PathBuf>),
    Rename { path: PathBuf, name: String },
    Create { dir: PathBuf, name: String, folder: bool },
    Restore(Vec<PathBuf>),
    TrashList(Vec<PathBuf>),
    EmptyTrash(Vec<PathBuf>),
    Undo(Vec<Step>),
    Trail { query: String, limit: usize },
    TrailForget,
}

#[derive(Default)]
struct Access {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

/// A full path from the request, tidied ("a/./b/" -> "a/b"); ".." refused.
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

fn paths(req: &Value, key: &str) -> Result<Vec<PathBuf>, String> {
    let arr = req.get(key).and_then(Value::as_array).ok_or(format!("missing '{key}'"))?;
    if arr.is_empty() || arr.len() > MAX_PATHS {
        return Err(format!("between 1 and {MAX_PATHS} items, please"));
    }
    arr.iter().map(abs).collect()
}

fn path(req: &Value, key: &str) -> Result<PathBuf, String> {
    abs(req.get(key).ok_or(format!("missing '{key}'"))?)
}

fn text(req: &Value, key: &str) -> Result<String, String> {
    req.get(key).and_then(Value::as_str).map(str::to_string).ok_or(format!("missing '{key}'"))
}

fn parent(p: &Path) -> PathBuf {
    p.parent().unwrap_or(Path::new("/")).to_path_buf()
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn folder(p: &Path) -> Result<PathBuf, String> {
    if !p.is_dir() {
        return Err(format!("{} isn't a folder", p.display()));
    }
    Ok(p.to_path_buf())
}

/// Work out what the request will touch (for Landlock) and get ready.
fn plan(req: &Value, home: &Path) -> Result<(Job, Access), String> {
    let op = req.get("op").and_then(Value::as_str).ok_or("missing 'op'")?;
    let mut a = Access::default();
    let job = match op {
        "copy" | "move" => {
            let (sources, dest) = (paths(req, "sources")?, folder(&path(req, "dest_dir")?)?);
            for s in &sources {
                if op == "move" { a.write.push(parent(s)) } else { a.read.push(s.clone()) }
            }
            a.write.push(dest.clone());
            if op == "copy" { Job::Copy { sources, dest } } else { Job::Move { sources, dest } }
        }
        "duplicate" => {
            let ps = paths(req, "paths")?;
            a.write.extend(ps.iter().map(|p| parent(p)));
            Job::Duplicate(ps)
        }
        "trash" => {
            let mut items = Vec::new();
            for p in paths(req, "paths")? {
                if p == home || p == Path::new("/") {
                    return Err("your home folder itself can't go to the Trash".into());
                }
                let t = trash::trash_for(&p, home);
                // A drive we can't make a Trash on: that item fails, the rest go on.
                if trash::prepare(&t).is_ok() {
                    a.write.push(t.dir.clone());
                }
                a.write.push(parent(&p));
                items.push((p, t));
            }
            Job::Trash(items)
        }
        "delete" => {
            let ps = paths(req, "paths")?;
            for p in &ps {
                if p == home || p == Path::new("/") {
                    return Err("your home folder itself can't be deleted".into());
                }
                a.write.push(parent(p));
                if let Some(info) = trash::info_of(p) {
                    a.write.push(parent(&info));
                }
            }
            Job::Delete(ps)
        }
        "size" => {
            let ps = paths(req, "paths")?;
            a.read.extend(ps.iter().cloned());
            Job::Size(ps)
        }
        "rename" => {
            let (p, name) = (path(req, "path")?, text(req, "name")?);
            ops::check_name(&name)?;
            a.write.push(parent(&p));
            Job::Rename { path: p, name }
        }
        "new_folder" | "new_file" => {
            let (dir, name) = (folder(&path(req, "dir")?)?, text(req, "name")?);
            ops::check_name(&name)?;
            a.write.push(dir.clone());
            Job::Create { dir, name, folder: op == "new_folder" }
        }
        "restore" => {
            let items = paths(req, "items")?;
            for t in &items {
                let (original, _) = trash::original_of(t).ok_or(format!("{} isn't in the Trash", name_of(t)))?;
                a.write.push(parent(t));
                a.write.push(parent(&trash::info_of(t).unwrap()));
                a.write.push(ops::existing_ancestor(&parent(&original)));
            }
            Job::Restore(items)
        }
        "trash_list" | "empty_trash" => {
            let dirs = trash::all(home);
            if op == "trash_list" {
                a.read.extend(dirs.iter().cloned());
                Job::TrashList(dirs)
            } else {
                a.write.extend(dirs.iter().cloned());
                Job::EmptyTrash(dirs)
            }
        }
        "undo" => {
            let steps: Vec<Step> = serde_json::from_value(req.get("steps").cloned().ok_or("missing 'steps'")?)
                .map_err(|e| format!("bad undo steps: {e}"))?;
            for s in &steps {
                match s {
                    Step::Created { path } => {
                        let t = trash::trash_for(path, home);
                        if trash::prepare(&t).is_ok() {
                            a.write.push(t.dir);
                        }
                        a.write.push(parent(path));
                    }
                    Step::Moved { from, to } => {
                        a.write.push(ops::existing_ancestor(&parent(from)));
                        a.write.push(parent(to));
                    }
                    Step::Trashed { path, trashed, info } => {
                        a.write.push(ops::existing_ancestor(&parent(path)));
                        a.write.push(parent(trashed));
                        a.write.push(parent(info));
                    }
                }
            }
            Job::Undo(steps)
        }
        "trail" => {
            let d = nexa_sys::filetrail::dir(home);
            if d.is_dir() {
                a.read.push(d);
            }
            let query: String = req.get("query").and_then(Value::as_str).unwrap_or("").chars().take(200).collect();
            Job::Trail { query, limit: req.get("limit").and_then(Value::as_u64).unwrap_or(200).min(2000) as usize }
        }
        "trail_forget" => {
            let d = nexa_sys::filetrail::dir(home);
            if d.is_dir() {
                a.write.push(d);
            }
            Job::TrailForget
        }
        other => return Err(format!("unknown op '{other}'")),
    };
    Ok((job, a))
}

fn confine(a: &Access) -> Result<(), String> {
    let lossy = |p: &PathBuf| p.to_string_lossy().into_owned();
    // The local time zone, for Trash dates.
    let mut rules: Vec<(String, u64)> = vec![("/etc/localtime".into(), sandbox::READ), ("/usr/share/zoneinfo".into(), sandbox::READ)];
    rules.extend(a.read.iter().map(|p| (lossy(p), sandbox::READ)));
    rules.extend(a.write.iter().map(|p| (lossy(p), sandbox::READ_WRITE | sandbox::MAKE_SYM)));
    sandbox::restrict(&sandbox::Rules { fs: rules, connect_ports: vec![], any_network: false })
        .map(|_| ()).map_err(|e| format!("couldn't confine myself: {e}"))
}

fn failures(f: &Failures) -> Value {
    Value::Array(f.iter().map(|(p, e)| json!({"path": p, "error": e})).collect())
}

fn items(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{n} items") }
}

/// Batch results: an error if nothing at all worked, else what did and didn't.
fn outcome(done: String, n_ok: usize, failed: &Failures, mut extra: Value) -> Result<Value, String> {
    if n_ok == 0 {
        if let Some((p, why)) = failed.first() {
            return Err(format!("{}: {why}", name_of(p)));
        }
    }
    extra["done"] = json!(done);
    extra["failed"] = failures(failed);
    Ok(extra)
}

/// Copy folders refuse to go into themselves (the copy would never end).
fn into_itself(src: &Path, dest: &Path) -> bool {
    match (fs::symlink_metadata(src), fs::canonicalize(src), fs::canonicalize(dest)) {
        (Ok(md), Ok(s), Ok(d)) => md.is_dir() && ops::is_within(&d, &s),
        _ => false,
    }
}

fn run(job: Job, home: &Path) -> Result<Value, String> {
    match job {
        Job::Copy { sources, dest } => {
            let pairs = sources.into_iter().map(|s| (s, dest.clone())).collect();
            copy_into(pairs)
        }
        Job::Duplicate(ps) => copy_into(ps.into_iter().map(|p| { let d = parent(&p); (p, d) }).collect()),
        Job::Move { sources, dest } => {
            let dest_dev = ops::device(&dest);
            let across: Vec<PathBuf> = sources.iter().filter(|s| ops::device(s) != dest_dev).cloned().collect();
            let mut prog = Progress::new(ops::usage(&across).bytes);
            let (mut moved, mut undo, mut failed) = (Vec::new(), Vec::new(), Failures::new());
            for s in sources {
                if parent(&s) == dest {
                    continue; // already there
                }
                if into_itself(&s, &dest) {
                    failed.push((s, "a folder can't go inside itself".into()));
                    continue;
                }
                let Ok(md) = fs::symlink_metadata(&s) else {
                    failed.push((s, "it isn't there any more".into()));
                    continue;
                };
                match place(&dest, s.file_name().unwrap_or_default(), false, md.is_dir(), |t| ops::move_item(&s, t, &mut prog)) {
                    Ok(t) => {
                        undo.push(Step::Moved { from: s, to: t.clone() });
                        moved.push(t);
                    }
                    Err(e) => failed.push((s, ops::say(&e))),
                }
            }
            outcome(format!("Moved {}", items(moved.len())), moved.len(), &failed, json!({"moved": moved, "undo": undo}))
        }
        Job::Trash(list) => {
            let (mut undo, mut failed) = (Vec::new(), Failures::new());
            for (p, t) in list {
                match trash::trash(&p, &t) {
                    Ok(step) => undo.push(step),
                    Err(e) if t.topdir.is_some() && matches!(e.raw_os_error(), Some(libc::EACCES | libc::EPERM | libc::EROFS)) => {
                        failed.push((p, "this drive has no Trash you can use - it can only be deleted for good".into()))
                    }
                    Err(e) => failed.push((p, ops::say(&e))),
                }
            }
            let n = undo.len();
            outcome(format!("Moved {} to the Trash", items(n)), n, &failed, json!({"undo": undo}))
        }
        Job::Delete(ps) => {
            let (mut deleted, mut failed) = (Vec::new(), Failures::new());
            for p in ps {
                match ops::remove(&p) {
                    Ok(()) => {
                        trash::forget(&p);
                        deleted.push(p);
                    }
                    Err(e) => failed.push((p, ops::say(&e))),
                }
            }
            let n = deleted.len();
            outcome(format!("Deleted {} for good", items(n)), n, &failed, json!({"deleted": deleted}))
        }
        Job::Size(ps) => {
            let u = ops::usage(&ps);
            Ok(json!({"bytes": u.bytes, "files": u.files, "folders": u.folders}))
        }
        Job::Rename { path, name } => {
            let target = parent(&path).join(&name);
            if target == path {
                return Ok(json!({"done": "Nothing to change", "path": path, "undo": []}));
            }
            let src = fs::symlink_metadata(&path).map_err(|e| ops::say(&e))?;
            let same_file = fs::symlink_metadata(&target).map(|t| (t.dev(), t.ino()) == (src.dev(), src.ino())).unwrap_or(false);
            if same_file {
                // Only the letter case changes on a drive that ignores case
                // (FAT, exFAT): go through a temporary name.
                let tmp = place(&parent(&path), std::ffi::OsStr::new(".nexa-rename"), false, false, |t| ops::rename_noreplace(&path, t))
                    .map_err(|e| ops::say(&e))?;
                ops::rename_noreplace(&tmp, &target).map_err(|e| {
                    let _ = ops::rename_noreplace(&tmp, &path);
                    ops::say(&e)
                })?;
            } else {
                ops::rename_noreplace(&path, &target).map_err(|e| match e.kind() {
                    std::io::ErrorKind::AlreadyExists => format!("There's already something called \"{name}\" here."),
                    _ => ops::say(&e),
                })?;
            }
            Ok(json!({"done": format!("Renamed to {name}"), "path": target, "undo": [Step::Moved { from: path, to: target.clone() }]}))
        }
        Job::Create { dir, name, folder } => {
            let made = place(&dir, std::ffi::OsStr::new(&name), false, folder, |t| {
                if folder { fs::create_dir(t) } else { fs::OpenOptions::new().write(true).create_new(true).open(t).map(|_| ()) }
            }).map_err(|e| ops::say(&e))?;
            Ok(json!({"done": format!("Created {}", name_of(&made)), "path": made, "undo": [Step::Created { path: made.clone() }]}))
        }
        Job::Restore(list) => {
            let (mut back, mut failed) = (Vec::new(), Failures::new());
            for t in list {
                match trash::restore(&t) {
                    Ok(p) => back.push(p),
                    Err(e) => failed.push((t, ops::say(&e))),
                }
            }
            let n = back.len();
            outcome(format!("Restored {}", items(n)), n, &failed, json!({"restored": back}))
        }
        Job::TrashList(dirs) => Ok(json!({"items": trash::list(&dirs)})),
        Job::EmptyTrash(dirs) => {
            let (n, failed) = trash::empty(&dirs);
            Ok(json!({"done": format!("Emptied the Trash ({})", items(n as usize)), "failed": failures(&failed)}))
        }
        Job::Undo(steps) => {
            // What was really done (for FileTrail): where things ended up.
            let (mut n, mut failed, mut did) = (0, Failures::new(), Vec::new());
            for s in steps.into_iter().rev() {
                let r = match &s {
                    Step::Created { path } => trash::trash(path, &trash::trash_for(path, home))
                        .map(|st| did.push(serde_json::to_value(st).unwrap_or_default())),
                    Step::Moved { from, to } => fs::create_dir_all(parent(from)).and_then(|_| {
                        let is_dir = fs::symlink_metadata(to)?.is_dir();
                        place(&parent(from), from.file_name().unwrap_or_default(), false, is_dir,
                              |t| ops::move_item(to, t, &mut Progress::new(0)))
                            .map(|t| did.push(json!({"op": "moved", "from": to, "to": t})))
                    }),
                    Step::Trashed { trashed, .. } => trash::restore(trashed).map(|p| did.push(json!({"op": "restored", "to": p}))),
                };
                match r {
                    Ok(()) => n += 1,
                    Err(e) => failed.push((match s { Step::Created { path } => path, Step::Moved { to, .. } => to, Step::Trashed { path, .. } => path }, ops::say(&e))),
                }
            }
            outcome(format!("Undid {}", if n == 1 { "1 change".into() } else { format!("{n} changes") }), n, &failed, json!({"did": did}))
        }
        Job::Trail { query, limit } => Ok(json!({"items": nexa_sys::filetrail::trail(home, &query, limit)})),
        Job::TrailForget => Ok(json!({"done": "FileTrail forgot every change", "forgot": nexa_sys::filetrail::forget(home)})),
    }
}

/// Copy each source into the folder paired with it.
fn copy_into(pairs: Vec<(PathBuf, PathBuf)>) -> Result<Value, String> {
    let sources: Vec<PathBuf> = pairs.iter().map(|(s, _)| s.clone()).collect();
    let mut prog = Progress::new(ops::usage(&sources).bytes);
    let (mut created, mut undo, mut failed) = (Vec::new(), Vec::new(), Failures::new());
    for (s, dest) in pairs {
        if into_itself(&s, &dest) {
            failed.push((s, "a folder can't be copied into itself".into()));
            continue;
        }
        let Ok(md) = fs::symlink_metadata(&s) else {
            failed.push((s, "it isn't there any more".into()));
            continue;
        };
        let mut inside = Failures::new();
        // A name that's taken (always, when duplicating) becomes "name (copy)".
        match place(&dest, s.file_name().unwrap_or_default(), true, md.is_dir(), |t| ops::copy_item(&s, t, &mut prog, &mut inside)) {
            Ok(t) => {
                undo.push(Step::Created { path: t.clone() });
                created.push(t);
            }
            Err(e) => failed.push((s, ops::say(&e))),
        }
        failed.extend(inside);
    }
    // "Copied" only once it is on the drive, not just in memory.
    if !created.is_empty() {
        prog.finishing();
    }
    let mut flushed: Vec<PathBuf> = Vec::new();
    for t in &created {
        let dir = parent(t);
        if flushed.contains(&dir) {
            continue;
        }
        if let Err(e) = ops::flush_fs(&dir) {
            failed.push((t.clone(), format!("the drive couldn't store it ({})", ops::say(&e))));
        }
        flushed.push(dir);
    }
    let n = created.len();
    outcome(format!("Copied {}", items(n)), n, &failed, json!({"created": created, "undo": undo}))
}

fn handle(input: &[u8]) -> Result<Value, String> {
    let req: Value = serde_json::from_slice(input).map_err(|e| format!("bad request: {e}"))?;
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("HOME is not set")?;
    let home = home.canonicalize().unwrap_or(home);
    let (job, access) = plan(&req, &home)?;
    let op = req.get("op").and_then(Value::as_str).unwrap_or("").to_string();
    let changes = !matches!(op.as_str(), "size" | "trash_list" | "trail" | "trail_forget");
    // FileTrail's journal: opened before confining, so the sandbox stays as tight.
    let mut journal = if changes { nexa_sys::filetrail::open(&home) } else { None };
    confine(&access)?;
    let result = run(job, &home)?;
    if let Some(f) = journal.as_mut() {
        let steps: Vec<Value> = match op.as_str() {
            "undo" => result["did"].as_array().cloned().unwrap_or_default(),
            "restore" => result["restored"].as_array().into_iter().flatten().map(|p| json!({"op": "restored", "to": p})).collect(),
            "delete" => result["deleted"].as_array().into_iter().flatten().map(|p| json!({"op": "deleted", "path": p})).collect(),
            // Emptying the Trash: everything in it is gone for good.
            "empty_trash" => vec![json!({"op": "emptied"})],
            _ => result["undo"].as_array().cloned().unwrap_or_default(),
        };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        nexa_sys::filetrail::append(f, &op, &steps, req.get("sources").or_else(|| req.get("paths")), now);
    }
    Ok(result)
}

fn main() {
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
