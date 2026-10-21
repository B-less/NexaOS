//! The real binary end to end, Landlock included: what the Files app sends.

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Home {
        let h = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("fileops-{name}"));
        let _ = fs::remove_dir_all(&h);
        fs::create_dir_all(&h).unwrap();
        Home(h.canonicalize().unwrap())
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    /// Run one request; returns (all progress lines, final reply).
    fn run(&self, req: Value) -> (Vec<Value>, Value) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nexa-fileops"))
            .env_clear().env("HOME", &self.0)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(req.to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        let mut lines: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let last = lines.pop().unwrap();
        assert_eq!(out.status.success(), last["ok"] == true, "exit code matches the reply");
        (lines, last)
    }

    fn ok(&self, req: Value) -> Value {
        let (_, r) = self.run(req.clone());
        assert_eq!(r["ok"], true, "{req} -> {r}");
        r["result"].clone()
    }

    fn err(&self, req: Value) -> String {
        let (_, r) = self.run(req.clone());
        assert_eq!(r["ok"], false, "{req} should fail: {r}");
        r["error"].as_str().unwrap().to_string()
    }
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn a_files_session() {
    let h = Home::new("session");
    let r = h.ok(json!({"op": "new_folder", "dir": s(&h.0), "name": "Documents"}));
    assert_eq!(r["path"], s(&h.p("Documents")));
    let r = h.ok(json!({"op": "new_folder", "dir": s(&h.0), "name": "Documents"}));
    assert_eq!(r["path"], s(&h.p("Documents (2)")), "a taken name gets a number, never an error");
    h.ok(json!({"op": "new_file", "dir": s(&h.p("Documents")), "name": "notes.txt"}));
    fs::write(h.p("Documents/notes.txt"), "remember the milk").unwrap();
    fs::write(h.p("video.bin"), vec![1u8; 5 << 20]).unwrap();

    // Copy two things into a folder: progress lines, then the result.
    let (progress, r) = h.run(json!({"op": "copy", "sources": [s(&h.p("video.bin")), s(&h.p("Documents"))], "dest_dir": s(&h.p("Documents (2)"))}));
    assert_eq!(r["result"]["done"], "Copied 2 items");
    assert!(progress.iter().all(|p| p["progress"]["total"] == (5 << 20) + 17), "{progress:?}");
    assert_eq!(progress.last().unwrap()["progress"]["finishing"], true, "\"Copied\" only after it is written to the drive");
    assert_eq!(fs::read_to_string(h.p("Documents (2)/Documents/notes.txt")).unwrap(), "remember the milk");
    let undo_copy = r["result"]["undo"].clone();

    // Duplicate, rename, and a rename that would overwrite.
    let r = h.ok(json!({"op": "duplicate", "paths": [s(&h.p("Documents/notes.txt"))]}));
    assert_eq!(r["created"][0], s(&h.p("Documents/notes (copy).txt")));
    let e = h.err(json!({"op": "rename", "path": s(&h.p("Documents/notes (copy).txt")), "name": "notes.txt"}));
    assert!(e.contains("already something called \"notes.txt\""), "{e}");
    let r = h.ok(json!({"op": "rename", "path": s(&h.p("Documents/notes (copy).txt")), "name": "shopping.txt"}));
    let undo_rename = r["undo"].clone();
    assert!(h.p("Documents/shopping.txt").exists());

    // Move, then undo the move and the rename.
    let r = h.ok(json!({"op": "move", "sources": [s(&h.p("Documents/shopping.txt"))], "dest_dir": s(&h.0)}));
    assert_eq!(r["moved"][0], s(&h.p("shopping.txt")));
    h.ok(json!({"op": "undo", "steps": r["undo"]}));
    h.ok(json!({"op": "undo", "steps": undo_rename}));
    assert!(h.p("Documents/notes (copy).txt").exists() && !h.p("shopping.txt").exists());

    // Trash: listed, restorable, undoable.
    let r = h.ok(json!({"op": "trash", "paths": [s(&h.p("Documents/notes (copy).txt")), s(&h.p("video.bin"))]}));
    assert_eq!(r["done"], "Moved 2 items to the Trash");
    assert!(!h.p("video.bin").exists());
    let info = fs::read_to_string(h.p(".local/share/Trash/info/video.bin.trashinfo")).unwrap();
    assert!(info.contains(&format!("Path={}/video.bin\n", s(&h.0))), "{info}");
    let items = h.ok(json!({"op": "trash_list"}))["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 2);
    let video = items.as_array().unwrap().iter().find(|i| i["name"] == "video.bin").unwrap();
    assert_eq!(video["size"], 5 << 20);
    h.ok(json!({"op": "restore", "items": [video["trashed"]]}));
    assert!(h.p("video.bin").exists());
    h.ok(json!({"op": "undo", "steps": r["undo"]}));
    assert!(h.p("Documents/notes (copy).txt").exists(), "undo restores what's still in the Trash");

    // Undoing the copy sends the copies to the Trash (not oblivion).
    h.ok(json!({"op": "undo", "steps": undo_copy}));
    assert!(!h.p("Documents (2)/video.bin").exists());
    assert_eq!(h.ok(json!({"op": "trash_list"}))["items"].as_array().unwrap().len(), 2);
    let r = h.ok(json!({"op": "size", "paths": [s(&h.p("Documents"))]}));
    assert_eq!((r["files"].as_u64(), r["folders"].as_u64()), (Some(2), Some(1)));
    assert_eq!(h.ok(json!({"op": "empty_trash"}))["done"], "Emptied the Trash (2 items)");
    h.ok(json!({"op": "delete", "paths": [s(&h.p("Documents (2)"))]}));
    assert!(!h.p("Documents (2)").exists());
}

#[test]
fn refusals() {
    let h = Home::new("refuse");
    fs::create_dir_all(h.p("a/b")).unwrap();
    assert!(h.err(json!({"op": "copy", "sources": [s(&h.p("a"))], "dest_dir": s(&h.p("a/b"))})).contains("into itself"));
    assert!(h.err(json!({"op": "move", "sources": [s(&h.p("a"))], "dest_dir": s(&h.p("a/b"))})).contains("inside itself"));
    assert!(h.err(json!({"op": "trash", "paths": [s(&h.0)]})).contains("home folder"));
    assert!(h.err(json!({"op": "delete", "paths": ["relative/path"]})).contains("full path"));
    assert!(h.err(json!({"op": "delete", "paths": [format!("{}/a/../..", s(&h.0))]})).contains(".."));
    assert!(h.err(json!({"op": "rename", "path": s(&h.p("a")), "name": "x/y"})).contains("\"/\""));
    assert!(h.err(json!({"op": "nonsense"})).contains("unknown op"));
}

#[test]
fn confined_to_what_the_request_names() {
    // A copy reads only its sources: a symlink inside one pointing
    // elsewhere is copied as a link, and the process can't read past it.
    let h = Home::new("confine");
    fs::create_dir_all(h.p("secret")).unwrap();
    fs::write(h.p("secret/key"), "x").unwrap();
    fs::create_dir_all(h.p("pub")).unwrap();
    std::os::unix::fs::symlink(h.p("secret/key"), h.p("pub/link")).unwrap();
    fs::create_dir_all(h.p("out")).unwrap();
    h.ok(json!({"op": "copy", "sources": [s(&h.p("pub"))], "dest_dir": s(&h.p("out"))}));
    assert_eq!(fs::read_link(h.p("out/pub/link")).unwrap(), h.p("secret/key"));
    // Sizing "pub" can't see into "secret" through the link either.
    assert_eq!(h.ok(json!({"op": "size", "paths": [s(&h.p("pub"))]}))["files"], 1);
}
