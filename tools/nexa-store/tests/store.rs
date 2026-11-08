//! The real nexa-store against catalogues built with nexa-pack: signed and
//! unsigned, a tampered index, a tampered package, installed and update
//! states, search and categories, verified downloads.

use nexa_pack::manifest::{Manifest, ABI, ARCH, FORMAT};
use nexa_pack::{package, repo, sign};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct World {
    root: PathBuf,
}

impl World {
    fn new(name: &str) -> World {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("store-{name}"));
        let _ = fs::remove_dir_all(&root);
        for d in ["home", "keys", "registry", "repo/packages", "unsigned/packages"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        World { root }
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn package(&self, repo_dir: &str, id: &str, name: &str, version: &str, categories: &str, perms: &[&str], key: Option<&ed25519_dalek_key::Key>) {
        let mut files = BTreeMap::new();
        files.insert(format!("bin/{id}"), format!("#!/bin/sh\necho {id} {version}\n").into_bytes());
        files.insert("icon.svg".to_string(), b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec());
        let shots = if id == "notes" {
            files.insert("screenshots/1.png".to_string(), b"\x89PNG\r\n\x1a\n a picture".to_vec());
            vec!["screenshots/1.png".to_string()]
        } else {
            vec![]
        };
        let m = Manifest {
            format: FORMAT, id: id.into(), name: name.into(), version: version.into(),
            description: format!("{name} does useful things"), categories: categories.into(),
            icon: Some("icon.svg".into()), arch: ARCH.into(), abi: ABI.into(), depends: vec![],
            permissions: perms.iter().map(|s| s.to_string()).collect(), terminal: false, screenshots: shots, opens: vec![],
            files: files.iter().map(|(p, d)| (p.clone(), sign::sha256_hex(d))).collect(),
        };
        let bytes = package::write(&m, &files, key.map(|k| &k.0)).unwrap();
        fs::write(self.p(repo_dir).join("packages").join(format!("{id}-{version}.nexa")), bytes).unwrap();
    }

    fn conf(&self, cats: &[(&str, &str)]) {
        let list: Vec<Value> = cats.iter().map(|(n, d)| serde_json::json!({"name": n, "url": format!("file://{}", self.p(d).display())})).collect();
        fs::write(self.p("store.conf"), serde_json::json!({"catalogues": list}).to_string()).unwrap();
    }

    fn run(&self, args: &[&str]) -> (Vec<Value>, Value) {
        let out = Command::new(env!("CARGO_BIN_EXE_nexa-store")).args(args)
            .env_clear().env("HOME", self.p("home")).env("NEXA_STORE_CONF", self.p("store.conf"))
            .env("NEXA_STORE_KEYS", self.p("keys")).env("NEXA_STORE_REGISTRY", self.p("registry"))
            .output().unwrap();
        let mut lines: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let last = lines.pop().unwrap();
        (lines, last)
    }

    fn ok(&self, args: &[&str]) -> Value {
        let (_, r) = self.run(args);
        assert_eq!(r["ok"], true, "{args:?} -> {r}");
        r["result"].clone()
    }
}

mod ed25519_dalek_key {
    pub struct Key(pub ed25519_dalek::SigningKey);
}

fn trusted_key(w: &World) -> ed25519_dalek_key::Key {
    let k = sign::generate().unwrap();
    fs::write(w.p("keys").join(format!("{}.pub", sign::key_id(&k.verifying_key()))), sign::public_text(&k.verifying_key())).unwrap();
    ed25519_dalek_key::Key(k)
}

fn by_id(list: &Value, id: &str) -> Value {
    list["apps"].as_array().unwrap().iter().find(|a| a["id"] == id).cloned().unwrap_or(Value::Null)
}

#[test]
fn browse_and_download() {
    let w = World::new("browse");
    let key = trusted_key(&w);
    w.package("repo", "notes", "Notes", "1.2.0", "Office;", &["files.documents"], Some(&key));
    w.package("repo", "clock", "Clock", "0.3", "Utility;", &[], Some(&key));
    w.package("repo", "chat", "Chat", "2.0", "Network;", &["network", "microphone"], Some(&key));
    repo::build(&w.p("repo"), Some(&key.0), Some(&w.p("keys"))).unwrap();
    w.conf(&[("NexaOS", "repo")]);
    // Notes 1.0 is installed (an update waits); Clock 0.3 is up to date.
    fs::write(w.p("registry/notes.json"), r#"{"id":"notes","version":"1.0.0"}"#).unwrap();
    fs::write(w.p("registry/clock.json"), r#"{"id":"clock","version":"0.3"}"#).unwrap();

    assert!(w.ok(&["list"])["apps"].as_array().unwrap().is_empty(), "nothing before the first refresh");
    let r = w.ok(&["refresh"]);
    assert_eq!((r["catalogues"][0]["ok"].as_bool(), r["catalogues"][0]["apps"].as_u64(), r["catalogues"][0]["trust"].as_str()),
               (Some(true), Some(3), Some("trusted")), "{r}");

    let list = w.ok(&["list"]);
    let notes = by_id(&list, "notes");
    assert_eq!((notes["state"].as_str(), notes["installed"].as_str(), notes["verified"].as_bool()), (Some("update"), Some("1.0.0"), Some(true)));
    assert_eq!(by_id(&list, "clock")["state"], "installed");
    assert_eq!(by_id(&list, "chat")["state"], "available");
    assert_eq!(by_id(&list, "chat")["permissions"], serde_json::json!(["network", "microphone"]));
    assert!(Path::new(notes["icon"].as_str().unwrap()).is_file(), "icons are cached");
    assert_eq!(w.ok(&["list", "--search", "CHAT"])["apps"].as_array().unwrap().len(), 1);
    assert_eq!(w.ok(&["list", "--category", "office"])["apps"].as_array().unwrap().len(), 1);
    assert_eq!(w.ok(&["updates"])["apps"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["notes"]);
    assert_eq!(w.ok(&["info", "chat"])["name"], "Chat");

    // Installed: Notes (an update waits), Clock, and an app no catalogue
    // has, named by its own menu entry.
    fs::write(w.p("registry/nexa-sketch.desktop"), "[Desktop Entry]\nName=Sketch\nComment=Draw things\n[Desktop Action x]\nName=Wrong\n").unwrap();
    fs::write(w.p("registry/sketch.json"), serde_json::json!({"id": "sketch", "version": "0.9", "trust": "unknown-publisher",
        "paths": [w.p("registry/nexa-sketch.desktop")], "permissions": ["files.pictures"]}).to_string()).unwrap();
    fs::write(w.p("registry/bad name.json"), "{}").unwrap();
    let inst = w.ok(&["installed"]);
    let names: Vec<&str> = inst["apps"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Clock", "Notes", "Sketch"], "{inst}");
    let sketch = by_id(&inst, "sketch");
    assert_eq!((sketch["description"].as_str(), sketch["verified"].as_bool(), sketch["catalogue"].is_null()), (Some("Draw things"), Some(false), true));
    assert_eq!((by_id(&inst, "notes")["state"].as_str(), by_id(&inst, "notes")["version"].as_str(), by_id(&inst, "notes")["installed"].as_str()),
               (Some("update"), Some("1.2.0"), Some("1.0.0")));

    let (progress, r) = w.run(&["download", "chat"]);
    assert_eq!(r["ok"], true, "{r}");
    let path = PathBuf::from(r["result"]["path"].as_str().unwrap());
    assert!(path.is_file() && r["result"]["verified"] == true);
    assert!(progress.last().map_or(false, |p| p["progress"]["done"] == p["progress"]["total"]), "{progress:?}");
    assert!(package::verify(&fs::read(&path).unwrap(), Some(&w.p("keys"))).is_ok(), "a real, signed package");

    // A package swapped after the catalogue was signed: refused, nothing kept.
    w.package("repo", "chat", "Chat", "2.0", "Network;", &["network", "camera"], Some(&key));
    fs::remove_file(&path).unwrap();
    let (_, r) = w.run(&["download", "chat"]);
    assert!(r["error"].as_str().unwrap().contains("doesn't match the catalogue"), "{r}");
    assert!(!path.exists());
}

#[test]
fn signatures_and_trust() {
    let w = World::new("trust");
    let key = trusted_key(&w);
    w.package("repo", "notes", "Notes", "1.0", "Office;", &[], Some(&key));
    repo::build(&w.p("repo"), Some(&key.0), Some(&w.p("keys"))).unwrap();
    w.package("unsigned", "notes", "Notes (other)", "9.0", "Office;", &[], None);
    w.package("unsigned", "game", "Game", "1.0", "Game;", &[], None);
    repo::build(&w.p("unsigned"), None, None).unwrap();
    w.conf(&[("NexaOS", "repo"), ("Someone", "unsigned")]);
    let r = w.ok(&["refresh"]);
    assert_eq!(r["catalogues"][1]["trust"], "unsigned");
    let list = w.ok(&["list"]);
    assert_eq!(by_id(&list, "notes")["catalogue"], "NexaOS", "a verified catalogue wins over a newer unverified version");
    assert_eq!((by_id(&list, "game")["verified"].as_bool(), by_id(&list, "game")["catalogue"].as_str()), (Some(false), Some("Someone")));

    // The signed index changed after signing: refresh refuses it.
    let idx = fs::read_to_string(w.p("repo/index.json")).unwrap().replace("\"1.0\"", "\"1.1\"");
    fs::write(w.p("repo/index.json"), idx).unwrap();
    let r = w.ok(&["refresh"]);
    assert_eq!(r["catalogues"][0]["ok"], false);
    assert!(r["catalogues"][0]["error"].as_str().unwrap().contains("changed after signing"), "{r}");
    // ...and the copy already in the cache, tampered: dropped when read.
    let shots = by_id(&list, "notes")["screenshots"].as_array().unwrap().clone();
    assert_eq!(shots.len(), 1, "the screenshot came with the catalogue");
    assert!(fs::read(shots[0].as_str().unwrap()).unwrap().starts_with(b"\x89PNG"));
    let cached = PathBuf::from(by_id(&list, "notes")["icon"].as_str().unwrap()).parent().unwrap().parent().unwrap().join("index.json");
    let t = fs::read_to_string(&cached).unwrap().replace("Notes", "Notez");
    fs::write(&cached, t).unwrap();
    assert!(by_id(&w.ok(&["list"]), "notes")["catalogue"] != "NexaOS", "a changed cache isn't trusted");

    // Plain http is refused outright.
    fs::write(w.p("store.conf"), r#"{"catalogues":[{"name":"x","url":"http://example.com/repo"}]}"#).unwrap();
    assert!(w.ok(&["refresh"])["catalogues"][0]["error"].as_str().unwrap().contains("HTTPS"));
}
