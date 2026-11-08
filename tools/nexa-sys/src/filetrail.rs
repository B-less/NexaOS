//! FileTrail (Magic Layer): "where did I move it?". nexa-fileops writes one
//! line per change it made, and anyone can then follow a file from where it
//! was to where it is now, even through later moves of it or its folder.
//!
//!   ~/.local/state/nexa/filetrail.jsonl
//!   {"at": 1729870000, "op": "move", "sources": [...],
//!    "steps": [{"op": "moved", "from": "/home/u/a.pdf", "to": "/home/u/Docs/a.pdf"}]}
//!
//! Steps: moved {from, to}, created {path}, trashed {path, trashed, info},
//! restored {to}, deleted {path} (for good), emptied (the Trash). Only names and places, never contents; the last 2000
//! changes, 1 MB at most; private (0700 folder, 0600 file). Off with the
//! FileTrail switch in Settings > Magic Layer; Files can forget it all.

use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub const KEEP: usize = 2000;
pub const MAX_BYTES: u64 = 1 << 20;

pub fn dir(home: &Path) -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/state")).join("nexa")
}

pub fn path(home: &Path) -> PathBuf {
    dir(home).join("filetrail.jsonl")
}

/// The journal, ready to append to; None when FileTrail is switched off.
/// Opened before nexa-fileops confines itself, so the sandbox doesn't need
/// to include it. Too big: the oldest half goes first.
pub fn open(home: &Path) -> Option<File> {
    if !crate::magic::enabled(home, "filetrail") {
        return None;
    }
    let d = dir(home);
    let _ = fs::DirBuilder::new().recursive(true).mode(0o700).create(&d);
    let mut f = OpenOptions::new().read(true).write(true).create(true).mode(0o600).open(path(home)).ok()?;
    if f.metadata().map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let lines: Vec<String> = BufReader::new(&f).lines().map_while(Result::ok).collect();
        let keep = &lines[lines.len().saturating_sub(KEEP / 2)..];
        f.set_len(0).ok()?;
        f.seek(SeekFrom::Start(0)).ok()?;
        for l in keep {
            writeln!(f, "{l}").ok()?;
        }
    }
    f.seek(SeekFrom::End(0)).ok()?;
    Some(f)
}

/// One change: what the request was, what it did.
pub fn append(f: &mut File, op: &str, steps: &[Value], sources: Option<&Value>, now: u64) {
    if steps.is_empty() {
        return;
    }
    let mut line = json!({"at": now, "op": op, "steps": steps});
    if let Some(s) = sources {
        line["sources"] = s.clone();
    }
    let _ = writeln!(f, "{line}");
}

pub fn forget(home: &Path) -> bool {
    match fs::remove_file(path(home)) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

pub fn entries(home: &Path) -> Vec<Value> {
    let Ok(f) = File::open(path(home)) else { return Vec::new() };
    BufReader::new(f).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str(&l).ok()).collect()
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn name(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

fn parent(p: &str) -> String {
    Path::new(p).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `p` after `from` moved to `to` (`p` itself, or something inside it).
fn moved_along(p: &str, from: &str, to: &str) -> Option<String> {
    if p == from {
        Some(to.to_string())
    } else {
        p.strip_prefix(from).filter(|rest| rest.starts_with('/')).map(|rest| format!("{to}{rest}"))
    }
}

/// Where `start` is now, after every change from entry `after` on:
/// (path, status): "here", "in-trash", "deleted" (for good, by Files),
/// "emptied" (the Trash it was in was emptied) or "gone" (not there, and
/// nothing Files did explains it).
fn follow(all: &[Value], after: usize, start: &str, in_trash: bool) -> (String, &'static str) {
    let mut cur = start.to_string();
    let mut trashed = in_trash;
    let mut ended: Option<&'static str> = None;
    for e in &all[after..] {
        for st in e["steps"].as_array().into_iter().flatten() {
            match st["op"].as_str() {
                Some("moved") => {
                    if let (Some(f), Some(t)) = (s(st, "from"), s(st, "to")) {
                        if let Some(n) = moved_along(&cur, &f, &t) {
                            cur = n;
                            trashed = false;
                        }
                    }
                }
                Some("trashed") => {
                    if let (Some(p), Some(t)) = (s(st, "path"), s(st, "trashed")) {
                        if let Some(n) = moved_along(&cur, &p, &t) {
                            cur = n;
                            trashed = true;
                        }
                    }
                }
                Some("deleted") => {
                    if let Some(p) = s(st, "path") {
                        if moved_along(&cur, &p, &p).is_some() {
                            ended = Some("deleted");
                        }
                    }
                }
                Some("emptied") if trashed => ended = Some("emptied"),
                Some("restored") => {
                    // Restored from the Trash: back where it was (the "to").
                    if trashed {
                        if let Some(t) = s(st, "to") {
                            if name(&t) == name(&cur) || t.ends_with(&name(&cur)) {
                                cur = t;
                                trashed = false;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let exists = fs::symlink_metadata(&cur).is_ok();
    let status = match (exists, ended) {
        (false, Some(why)) => why,
        (false, None) => "gone",
        (true, _) if trashed => "in-trash",
        (true, _) => "here",
    };
    (cur, status)
}

/// Every change, newest first, each followed to where the thing is now:
/// what Files did, and files installed apps saved (see `app_saves`).
/// `query`: part of a name (any case); empty for everything.
pub fn trail(home: &Path, query: &str, limit: usize) -> Vec<Value> {
    trail_with(home, &registry_dir(), query, limit)
}

pub fn trail_with(home: &Path, registry: &Path, query: &str, limit: usize) -> Vec<Value> {
    let all = entries(home);
    let q = query.trim().to_lowercase();
    let mut out = journal_items(&all, &q, limit);
    if crate::magic::enabled(home, "filetrail") {
        for (at, app, path) in app_saves(home, registry) {
            let label = name(&path);
            if !q.is_empty() && !label.to_lowercase().contains(&q) {
                continue;
            }
            // followed through what Files did after it was saved
            let after = all.iter().position(|e| e["at"].as_u64().unwrap_or(0) >= at).unwrap_or(all.len());
            let (now, status) = follow(&all, after, &path, false);
            out.push(json!({"at": at, "kind": "saved", "name": label, "from": null, "to": path, "now": now, "status": status, "app": app}));
        }
        out.sort_by(|a, b| b["at"].as_u64().cmp(&a["at"].as_u64()));
        out.truncate(limit);
    }
    out
}

/// Where installed apps are recorded (the broker writes it; nexa-run reads it).
pub fn registry_dir() -> PathBuf {
    std::env::var_os("NEXA_RUN_REGISTRY").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/var/lib/nexa-packages"))
}

/// An app's own record of files it saved, in its own data folder (a
/// sandboxed app can't write FileTrail's journal, and shouldn't):
/// `~/.local/share/<id>/nexa-saved.jsonl`, one `{"at": unix, "path": "/home/..."}` a line.
pub const APP_LOG: &str = "nexa-saved.jsonl";

/// The folders an app may write, from its files.* permissions, as nexa-run
/// grants them (user-dirs.dirs, or the usual names).
fn app_folders(home: &Path, perms: &[String]) -> Vec<PathBuf> {
    let dirs = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).unwrap_or_default();
    let xdg = |key: &str, fallback: &str| {
        dirs.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| *k == key)
            .map(|(_, v)| PathBuf::from(v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy())))
            .filter(|p| p.is_absolute() && !p.to_string_lossy().contains(".."))
            .unwrap_or_else(|| home.join(fallback))
    };
    perms.iter().filter_map(|p| match p.as_str() {
        "files.home" => Some(home.to_path_buf()),
        "files.documents" => Some(xdg("XDG_DOCUMENTS_DIR", "Documents")),
        "files.pictures" => Some(xdg("XDG_PICTURES_DIR", "Pictures")),
        "files.music" => Some(xdg("XDG_MUSIC_DIR", "Music")),
        "files.videos" => Some(xdg("XDG_VIDEOS_DIR", "Videos")),
        _ => None,
    }).collect()
}

/// Files installed apps say they saved: (at, the app's name, path). Only
/// paths inside a folder that app may write count, so an app can only
/// speak for files it could have made; the newest 200 per app.
pub fn app_saves(home: &Path, registry: &Path) -> Vec<(u64, String, String)> {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".local/share"));
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(registry) else { return out };
    for e in rd.flatten() {
        let Ok(rec) = fs::read(e.path()).map_err(|_| ()).and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|_| ())) else { continue };
        let Some(id) = rec["id"].as_str().filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))) else { continue };
        let perms: Vec<String> = rec["permissions"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        let folders = app_folders(home, &perms);
        if folders.is_empty() {
            continue;
        }
        // the app's name, from the menu entry the broker wrote
        let label = rec["paths"].as_array().into_iter().flatten().filter_map(Value::as_str).find(|p| p.ends_with(".desktop"))
            .and_then(|d| fs::read_to_string(d).ok())
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("Name=").map(|n| n.chars().filter(|c| !c.is_control()).take(80).collect::<String>())))
            .unwrap_or_else(|| id.to_string());
        let Ok(text) = fs::read_to_string(data.join(id).join(APP_LOG)) else { continue };
        let mut mine: Vec<(u64, String, String)> = text.lines().rev().take(200).filter_map(|l| {
            let v: Value = serde_json::from_str(l).ok()?;
            let p = v["path"].as_str()?;
            let path = Path::new(p);
            let clean = path.is_absolute() && !path.components().any(|c| matches!(c, std::path::Component::ParentDir));
            (clean && folders.iter().any(|f| path.starts_with(f)) && p.len() < 4096)
                .then(|| (v["at"].as_u64().unwrap_or(0), label.clone(), p.to_string()))
        }).collect();
        mine.reverse();
        out.extend(mine);
    }
    out
}

/// What reading the apps' records needs (for a sandbox): the registry,
/// each installed app's own data folder and its menu entry. Read only.
pub fn app_read_paths(home: &Path, registry: &Path) -> Vec<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".local/share"));
    let mut out = vec![];
    let Ok(rd) = fs::read_dir(registry) else { return out };
    out.push(registry.to_path_buf());
    for e in rd.flatten() {
        let Some(rec) = fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { continue };
        if let Some(id) = rec["id"].as_str().filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))) {
            let d = data.join(id);
            if d.is_dir() {
                out.push(d);
            }
        }
        if let Some(desktop) = rec["paths"].as_array().into_iter().flatten().filter_map(Value::as_str).find(|p| p.ends_with(".desktop")) {
            if Path::new(desktop).is_file() {
                out.push(PathBuf::from(desktop));
            }
        }
    }
    out
}

/// What Files did, newest first, each followed to where it is now.
fn journal_items(all: &[Value], q: &str, limit: usize) -> Vec<Value> {
    let mut out = Vec::new();
    for (i, e) in all.iter().enumerate().rev() {
        let at = e["at"].as_u64().unwrap_or(0);
        let op = e["op"].as_str().unwrap_or("");
        let sources: Vec<String> = e["sources"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        for st in e["steps"].as_array().into_iter().flatten() {
            let (kind, from, to) = match st["op"].as_str() {
                Some("moved") => {
                    let (f, t) = (s(st, "from").unwrap_or_default(), s(st, "to").unwrap_or_default());
                    let kind = if op == "undo" { "put back" } else if parent(&f) == parent(&t) { "renamed" } else { "moved" };
                    (kind, Some(f), t)
                }
                Some("created") => {
                    let t = s(st, "path").unwrap_or_default();
                    match op {
                        "copy" | "duplicate" => {
                            // The source with the same name (a duplicate is "name (copy)").
                            let stem = |p: &str| Path::new(p).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            let from = sources.iter().find(|src| name(src) == name(&t))
                                .or_else(|| sources.iter().find(|src| stem(&t).starts_with(&stem(src))))
                                .cloned();
                            ("copied", from, t)
                        }
                        _ => ("created", None, t),
                    }
                }
                Some("trashed") => ("trashed", s(st, "path"), s(st, "trashed").unwrap_or_default()),
                Some("restored") => ("restored", None, s(st, "to").unwrap_or_default()),
                Some("deleted") => ("deleted", None, s(st, "path").unwrap_or_default()),
                _ => continue,
            };
            let shown = if kind == "trashed" { from.clone().unwrap_or_default() } else { to.clone() };
            let label = name(&shown);
            if !q.is_empty() && !label.to_lowercase().contains(&q)
                && !from.as_deref().map(|f| name(f).to_lowercase().contains(&q)).unwrap_or(false) {
                continue;
            }
            let (now, status) = follow(all, i + 1, &to, kind == "trashed");
            let status = if kind == "deleted" && status == "gone" { "deleted" } else { status };
            let mut item = json!({"at": at, "kind": kind, "name": label, "from": from, "to": to, "now": now, "status": status});
            // How to put it back, when that is possible and safe.
            match (kind, status) {
                ("moved" | "renamed", "here") => {
                    if let Some(f) = &from {
                        if fs::symlink_metadata(f).is_err() && Path::new(&parent(f)).is_dir() {
                            item["put_back"] = json!({"op": "moved", "from": f, "to": now});
                        }
                    }
                }
                ("trashed", "in-trash") => {
                    if let (Some(p), Some(t), Some(info)) = (s(st, "path"), s(st, "trashed"), s(st, "info")) {
                        if t == now {
                            item["put_back"] = json!({"op": "trashed", "path": p, "trashed": t, "info": info});
                        }
                    }
                }
                _ => {}
            }
            out.push(item);
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        let h = std::env::temp_dir().join(format!("trail-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = fs::remove_dir_all(&h);
        fs::create_dir_all(&h).unwrap();
        h
    }

    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    #[test]
    fn files_apps_saved() {
        let h = home();
        let reg = h.join("registry");
        fs::create_dir_all(&reg).unwrap();
        let p = |s: &str| h.join(s).to_string_lossy().into_owned();
        fs::create_dir_all(h.join("apps")).unwrap();
        fs::write(h.join("apps/nexa-mail.desktop"), "[Desktop Entry]\nName=Mail\nExec=x\n").unwrap();
        fs::write(reg.join("mail.json"), json!({"id": "mail", "permissions": ["network", "files.documents"],
                                               "paths": [p("apps/nexa-mail.desktop")]}).to_string()).unwrap();
        fs::write(reg.join("other.json"), json!({"id": "other", "permissions": ["network"]}).to_string()).unwrap();
        touch(&h.join("Documents/Mail attachments/invoice.pdf"));
        touch(&h.join(".ssh/id_rsa"));
        let log = h.join(".local/share/mail").join(APP_LOG);
        touch(&log);
        fs::write(&log, [
            json!({"at": 300, "path": p("Documents/Mail attachments/invoice.pdf")}).to_string(),
            json!({"at": 301, "path": p(".ssh/id_rsa")}).to_string(),
            json!({"at": 302, "path": p("Documents/../.ssh/id_rsa")}).to_string(),
            json!({"at": 303, "path": "relative.pdf"}).to_string(),
            "not json".to_string(),
        ].join("\n")).unwrap();
        // an app with no folders can't speak for any file
        touch(&h.join(".local/share/other").join(APP_LOG));
        fs::write(h.join(".local/share/other").join(APP_LOG), json!({"at": 1, "path": p("Documents/x")}).to_string()).unwrap();
        std::env::remove_var("XDG_DATA_HOME");
        let saves = app_saves(&h, &reg);
        assert_eq!(saves, vec![(300, "Mail".to_string(), p("Documents/Mail attachments/invoice.pdf"))], "only inside its own folders");
        // Files then moves it: followed there
        touch(&h.join("Documents/Bills/invoice.pdf"));
        fs::remove_file(h.join("Documents/Mail attachments/invoice.pdf")).unwrap();
        let mut f = open(&h).unwrap();
        append(&mut f, "move", &[json!({"op": "moved", "from": p("Documents/Mail attachments/invoice.pdf"), "to": p("Documents/Bills/invoice.pdf")})], None, 400);
        drop(f);
        let t = trail_with(&h, &reg, "invoice", 10);
        let saved = t.iter().find(|x| x["kind"] == "saved").expect("the save is on the trail");
        assert_eq!((saved["app"].as_str(), saved["now"].as_str(), saved["status"].as_str()),
                   (Some("Mail"), Some(p("Documents/Bills/invoice.pdf").as_str()), Some("here")));
        assert_eq!(t[0]["kind"], "moved", "newest first: the move came after the save");
        assert!(trail_with(&h, &reg, "id_rsa", 10).is_empty());
    }

    #[test]
    fn a_file_is_followed_through_later_moves() {
        let h = home();
        let p = |s: &str| h.join(s).to_string_lossy().into_owned();
        // report.pdf: Downloads -> Docs, then the whole Docs folder -> Work/Docs.
        touch(&h.join("Work/Docs/report.pdf"));
        fs::create_dir_all(h.join("Downloads")).unwrap();
        let mut f = open(&h).unwrap();
        append(&mut f, "move", &[json!({"op": "moved", "from": p("Downloads/report.pdf"), "to": p("Docs/report.pdf")})], None, 100);
        append(&mut f, "move", &[json!({"op": "moved", "from": p("Docs"), "to": p("Work/Docs")})], None, 200);
        drop(f);
        let t = trail(&h, "REPORT", 10);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0]["kind"], "moved");
        assert_eq!(t[0]["from"], p("Downloads/report.pdf"));
        assert_eq!(t[0]["now"], p("Work/Docs/report.pdf"), "followed through the folder move");
        assert_eq!(t[0]["status"], "here");
        assert_eq!(t[0]["put_back"]["to"], p("Work/Docs/report.pdf"), "put back from where it is now");
        // Everything: the folder move is newest.
        let all = trail(&h, "", 10);
        assert_eq!(all[0]["name"], "Docs");
    }

    #[test]
    fn trash_copies_renames_and_gone() {
        let h = home();
        let p = |s: &str| h.join(s).to_string_lossy().into_owned();
        touch(&h.join(".local/share/Trash/files/notes.txt"));
        touch(&h.join("USB/photo.jpg"));
        touch(&h.join("b.txt"));
        let mut f = open(&h).unwrap();
        append(&mut f, "trash", &[json!({"op": "trashed", "path": p("notes.txt"), "trashed": p(".local/share/Trash/files/notes.txt"),
                                        "info": p(".local/share/Trash/info/notes.txt.trashinfo")})], None, 1);
        append(&mut f, "copy", &[json!({"op": "created", "path": p("USB/photo.jpg")})], Some(&json!([p("Pictures/photo.jpg")])), 2);
        append(&mut f, "rename", &[json!({"op": "moved", "from": p("a.txt"), "to": p("b.txt")})], None, 3);
        append(&mut f, "move", &[json!({"op": "moved", "from": p("old.odt"), "to": p("Docs/old.odt")})], None, 4);
        append(&mut f, "move", &[], None, 5); // nothing done: nothing written
        drop(f);
        let t = trail(&h, "", 10);
        assert_eq!(t.len(), 4);
        let by = |n: &str| t.iter().find(|x| x["name"] == n).unwrap_or_else(|| panic!("{n}: {t:?}")).clone();
        assert_eq!(by("notes.txt")["kind"], "trashed");
        assert_eq!(by("notes.txt")["status"], "in-trash");
        assert!(by("notes.txt")["put_back"]["trashed"].is_string());
        assert_eq!(by("photo.jpg")["kind"], "copied");
        assert_eq!(by("photo.jpg")["from"], p("Pictures/photo.jpg"));
        assert!(by("photo.jpg").get("put_back").is_none(), "a copy has nothing to put back");
        assert_eq!(by("b.txt")["kind"], "renamed");
        assert_eq!(by("old.odt")["status"], "gone", "moved later by something else: said so, not guessed");
        // Restored later: back where it was.
        touch(&h.join("notes.txt"));
        let mut f = open(&h).unwrap();
        append(&mut f, "restore", &[json!({"op": "restored", "to": p("notes.txt")})], None, 6);
        drop(f);
        let t = trail(&h, "notes", 10);
        let trashed = t.iter().find(|x| x["kind"] == "trashed").unwrap();
        assert_eq!(trashed["status"], "here");
        assert_eq!(trashed["now"], p("notes.txt"));
        // Deleted for good by Files, and a Trash emptied: said so.
        let mut f = open(&h).unwrap();
        append(&mut f, "delete", &[json!({"op": "deleted", "path": p("b.txt")})], None, 7);
        append(&mut f, "trash", &[json!({"op": "trashed", "path": p("x.txt"), "trashed": p(".local/share/Trash/files/x.txt"), "info": p("i")})], None, 8);
        append(&mut f, "empty_trash", &[json!({"op": "emptied"})], None, 9);
        drop(f);
        fs::remove_file(h.join("b.txt")).unwrap();
        let t = trail(&h, "", 20);
        assert_eq!(t.iter().find(|x| x["kind"] == "renamed").unwrap()["status"], "deleted");
        assert_eq!(t.iter().find(|x| x["name"] == "x.txt").unwrap()["status"], "emptied");
        assert_eq!(t.iter().find(|x| x["kind"] == "deleted").unwrap()["name"], "b.txt");
        assert_eq!(t.iter().find(|x| x["kind"] == "deleted").unwrap()["status"], "deleted");
    }

    #[test]
    fn switched_off_writes_nothing_and_forget_clears() {
        let h = home();
        if std::env::var_os("XDG_CONFIG_HOME").is_some() || std::env::var_os("XDG_STATE_HOME").is_some() {
            return;
        }
        fs::create_dir_all(h.join(".config/nexa")).unwrap();
        fs::write(h.join(".config/nexa/magic.json"), r#"{"features": {"filetrail": {"enabled": false}}}"#).unwrap();
        assert!(open(&h).is_none());
        fs::remove_file(h.join(".config/nexa/magic.json")).unwrap();
        let mut f = open(&h).unwrap();
        append(&mut f, "move", &[json!({"op": "moved", "from": "/a", "to": "/b"})], None, 1);
        drop(f);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(path(&h)).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(dir(&h)).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(forget(&h));
        assert!(trail(&h, "", 10).is_empty());
    }

    #[test]
    fn the_journal_stays_bounded() {
        let h = home();
        let mut f = open(&h).unwrap();
        let long = "x".repeat(600);
        for i in 0..2200u64 {
            append(&mut f, "move", &[json!({"op": "moved", "from": format!("/a/{long}{i}"), "to": format!("/b/{long}{i}")})], None, i);
        }
        drop(f);
        assert!(fs::metadata(path(&h)).unwrap().len() > MAX_BYTES);
        drop(open(&h).unwrap()); // the next open trims
        let n = entries(&h).len();
        assert!(n == KEEP / 2, "{n}");
        assert_eq!(entries(&h).last().unwrap()["at"], 2199, "the newest are kept");
    }
}
