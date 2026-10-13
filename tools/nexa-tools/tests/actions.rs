//! End-to-end: run the real nexa-tools binary (each run sandboxes itself)
//! against a temporary home folder.

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn home(name: &str) -> PathBuf {
    let h = std::env::temp_dir().join(format!("nexa-tools-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&h);
    fs::create_dir_all(h.join("Documents")).unwrap();
    fs::create_dir_all(h.join(".ssh")).unwrap();
    fs::write(h.join(".ssh/id_ed25519"), "SECRET").unwrap();
    h.canonicalize().unwrap()
}

fn run(home: &PathBuf, tool: &str, args: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nexa-tools"))
        .env("HOME", home)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(json!({"tool": tool, "args": args}).to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| json!({"ok": false, "error": String::from_utf8_lossy(&out.stdout)}))
}

fn ok(v: &Value) -> &Value {
    assert_eq!(v["ok"], true, "{v}");
    &v["result"]
}

#[test]
fn write_then_undo() {
    let h = home("write");
    let r = run(&h, "write_text", json!({"path": "~/Notes/todo.txt", "content": "buy milk\n"}));
    let id = ok(&r)["undo_id"].as_str().unwrap().to_string();
    assert_eq!(fs::read_to_string(h.join("Notes/todo.txt")).unwrap(), "buy milk\n");
    let again = run(&h, "write_text", json!({"path": "~/Notes/todo.txt", "content": "x"}));
    assert!(again["error"].as_str().unwrap().contains("already exists"), "create never overwrites");
    ok(&run(&h, "undo", json!({"id": id})));
    assert!(!h.join("Notes/todo.txt").exists() && !h.join("Notes").exists(), "file and the folder made for it are gone");
    assert!(fs::read_dir(h.join(".local/share/Trash/files")).unwrap().count() == 1, "an undone new file goes to the Trash, not oblivion");
}

#[test]
fn overwrite_keeps_the_old_version() {
    let h = home("overwrite");
    fs::write(h.join("Documents/essay.txt"), "first draft").unwrap();
    let r = run(&h, "write_text", json!({"path": "Documents/essay.txt", "content": "second draft", "mode": "overwrite"}));
    let id = ok(&r)["undo_id"].as_str().unwrap().to_string();
    assert_eq!(fs::read_to_string(h.join("Documents/essay.txt")).unwrap(), "second draft");
    ok(&run(&h, "undo", json!({"id": id.clone()})));
    assert_eq!(fs::read_to_string(h.join("Documents/essay.txt")).unwrap(), "first draft");
    assert!(run(&h, "undo", json!({"id": id}))["error"].as_str().unwrap().contains("already undone"));
}

#[test]
fn trash_is_recoverable() {
    let h = home("trash");
    fs::write(h.join("Documents/old.pdf"), "pdf").unwrap();
    let id = ok(&run(&h, "trash", json!({"path": "Documents/old.pdf"})))["undo_id"].as_str().unwrap().to_string();
    assert!(!h.join("Documents/old.pdf").exists());
    let info = fs::read_to_string(h.join(".local/share/Trash/info/old.pdf.trashinfo")).unwrap();
    assert!(info.contains("[Trash Info]") && info.contains("Documents/old.pdf") && info.contains("DeletionDate="), "{info}");
    ok(&run(&h, "undo", json!({"id": id})));
    assert_eq!(fs::read_to_string(h.join("Documents/old.pdf")).unwrap(), "pdf");
}

#[test]
fn organise_plan_then_apply_then_undo() {
    let h = home("organise");
    let dl = h.join("Downloads");
    fs::create_dir_all(&dl).unwrap();
    for f in ["a.jpg", "b.pdf", "c.mp3", "d.zip", "e.weird"] {
        fs::write(dl.join(f), f).unwrap();
    }
    let plan = ok(&run(&h, "organize_plan", json!({"path": "~/Downloads"}))).clone();
    assert_eq!(plan["moves"].as_array().unwrap().len(), 5);
    assert!(dl.join("a.jpg").exists(), "planning moves nothing");
    let r = run(&h, "apply_moves", json!({"moves": plan["moves"]}));
    let id = ok(&r)["undo_id"].as_str().unwrap().to_string();
    assert!(dl.join("Pictures/a.jpg").exists() && dl.join("Documents/b.pdf").exists() && dl.join("Other/e.weird").exists());
    ok(&run(&h, "undo", json!({"id": id})));
    assert!(dl.join("a.jpg").exists() && !dl.join("Pictures").exists(), "undo puts every file back and removes the new folders");
}

#[test]
fn duplicates_found() {
    let h = home("dups");
    fs::write(h.join("Documents/one.txt"), "same content").unwrap();
    fs::write(h.join("Documents/two.txt"), "same content").unwrap();
    fs::write(h.join("Documents/three.txt"), "different!!!").unwrap();
    let r = run(&h, "find_duplicates", json!({"path": "~"}));
    let groups = ok(&r)["groups"].as_array().unwrap().clone();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["files"].as_array().unwrap().len(), 2);
}

#[test]
fn secrets_and_the_rest_of_the_system_are_off_limits() {
    let h = home("limits");
    for (tool, args) in [
        ("read_text", json!({"path": "~/.ssh/id_ed25519"})),
        ("read_text", json!({"path": "/etc/shadow"})),
        ("read_text", json!({"path": "Documents/../../../etc/passwd"})),
        ("trash", json!({"path": "~/.ssh"})),
        ("write_text", json!({"path": "~/.local/share/nexa-mind/activity.jsonl", "content": "forged", "mode": "append"})),
        ("move", json!({"from": "~/Documents", "to": "/tmp/stolen"})),
        ("trash", json!({"path": "~"})),
    ] {
        let r = run(&h, tool, args.clone());
        assert_eq!(r["ok"], false, "{tool} {args} should be refused: {r}");
    }
    // a symlink inside home pointing at the keys doesn't help
    std::os::unix::fs::symlink(h.join(".ssh"), h.join("Documents/keys")).unwrap();
    assert_eq!(run(&h, "read_text", json!({"path": "Documents/keys/id_ed25519"}))["ok"], false);
    // search never walks into secret folders
    let r = run(&h, "search", json!({"pattern": "id_ed25519"}));
    assert!(ok(&r)["matches"].as_array().unwrap().is_empty());
    assert_eq!(fs::read_to_string(h.join(".ssh/id_ed25519")).unwrap(), "SECRET");
}

#[test]
fn history_lists_actions() {
    let h = home("history");
    ok(&run(&h, "make_dir", json!({"path": "Projects/new"})));
    ok(&run(&h, "write_text", json!({"path": "Projects/new/readme.md", "content": "# hi"})));
    let acts = ok(&run(&h, "history", json!({}))).clone();
    let summaries: Vec<&str> = acts["actions"].as_array().unwrap().iter().map(|a| a["summary"].as_str().unwrap()).collect();
    assert_eq!(summaries.len(), 2, "{summaries:?}");
    assert!(summaries[0].starts_with("Created ~/Projects/new/readme.md"), "newest first: {summaries:?}");
    let log = fs::read_to_string(h.join(".local/share/nexa-mind/activity.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 2);
}
