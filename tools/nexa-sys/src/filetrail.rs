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

/// Every change, newest first, each followed to where the thing is now.
/// `query`: part of a name (any case); empty for everything.
pub fn trail(home: &Path, query: &str, limit: usize) -> Vec<Value> {
    let all = entries(home);
    let q = query.trim().to_lowercase();
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
            let (now, status) = follow(&all, i + 1, &to, kind == "trashed");
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
