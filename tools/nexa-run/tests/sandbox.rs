//! The real launcher running a probe "app" (a shell script) that tries to
//! read secrets, use folders, write, and connect, under different
//! permissions. The fake home is under the build folder, not /tmp (every
//! app may use /tmp, which would hide mistakes).

use std::fs;
use std::io::Read;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = r#"#!/bin/sh
OPENED="$1"
try() { if eval "$2" >/dev/null 2>&1; then echo "$1:yes"; else echo "$1:no"; fi; }
try secret 'cat "$HOME/.ssh/id_ed25519"'
try home_read 'cat "$HOME/notes.txt"'
try videos 'cat "$HOME/Videos/clip.txt"'
try own_write 'echo hi > "$HOME/.local/share/probe/note"'
try home_write 'echo hi > "$HOME/new.txt"'
try opened 'cat "$OPENED"'
try net 'nc -w 2 127.0.0.1 $PROBE_PORT </dev/null'
try drive_read 'cat "$NEXA_RUN_MEDIA/STICK/old.txt"'
try drive_write 'echo hi > "$NEXA_RUN_MEDIA/STICK/new.txt"'
"#;

struct World {
    root: PathBuf,
    home: PathBuf,
}

fn world(name: &str, permissions: &str, denied: Option<&str>) -> World {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("nexa-run-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let home = root.join("home");
    for d in [".ssh", "Videos", "Elsewhere", ".config/nexa"] {
        fs::create_dir_all(home.join(d)).unwrap();
    }
    fs::write(home.join(".ssh/id_ed25519"), "SECRET").unwrap();
    fs::write(home.join("notes.txt"), "private notes").unwrap();
    fs::write(home.join("Videos/clip.txt"), "a video").unwrap();
    fs::write(home.join("Elsewhere/opened.txt"), "chosen").unwrap();
    fs::create_dir_all(root.join("media/STICK")).unwrap();
    fs::write(root.join("media/STICK/old.txt"), "on the stick").unwrap();
    if let Some(d) = denied {
        fs::write(home.join(".config/nexa/app-permissions.json"), format!(r#"{{"probe": {{"denied": [{d}]}}}}"#)).unwrap();
    }
    fs::create_dir_all(root.join("registry")).unwrap();
    fs::create_dir_all(root.join("libexec")).unwrap();
    fs::write(root.join("registry/probe.json"), format!(r#"{{"id": "probe", "permissions": [{permissions}]}}"#)).unwrap();
    fs::write(root.join("libexec/probe"), PROBE).unwrap();
    fs::set_permissions(root.join("libexec/probe"), fs::Permissions::from_mode(0o755)).unwrap();
    World { root, home }
}

fn run(w: &World, port: u16) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_nexa-run"))
        .arg("probe").arg(w.home.join("Elsewhere/opened.txt"))
        .env("HOME", &w.home).env("NEXA_RUN_REGISTRY", w.root.join("registry"))
        .env("NEXA_RUN_LIBEXEC", w.root.join("libexec")).env("NEXA_RUN_LIB", w.root.join("lib"))
        .env("PROBE_PORT", port.to_string()).env("NEXA_RUN_MEDIA", w.root.join("media"))
        .output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

fn listener() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let _ = (&s).read(&mut [0u8; 16]);
        }
    });
    port
}

#[test]
fn an_app_with_no_permissions_gets_only_its_own_folder() {
    let w = world("none", "", None);
    let out = run(&w, listener());
    for want in ["secret:no", "home_read:no", "videos:no", "own_write:yes", "home_write:no", "opened:yes", "drive_read:no", "drive_write:no"] {
        assert!(out.contains(want), "{want} in:\n{out}");
    }
    if nexa_sys::sandbox::abi() >= 4 {
        assert!(out.contains("net:no"), "no network without the permission:\n{out}");
    }
    assert!(!w.home.join("new.txt").exists());
    assert_eq!(fs::read_to_string(w.home.join(".local/share/probe/note")).unwrap(), "hi\n");
}

#[test]
fn files_drives_opens_the_drives_and_nothing_else() {
    let w = world("drives", r#""files.drives""#, None);
    let out = run(&w, listener());
    for want in ["drive_read:yes", "drive_write:yes", "secret:no", "home_read:no", "videos:no", "home_write:no"] {
        assert!(out.contains(want), "{want} in:\n{out}");
    }
    assert_eq!(fs::read_to_string(w.root.join("media/STICK/new.txt")).unwrap(), "hi\n");
}

#[test]
fn files_videos_and_network_open_exactly_that() {
    let w = world("videos", r#""files.videos", "network""#, None);
    let out = run(&w, listener());
    for want in ["videos:yes", "net:yes", "secret:no", "home_read:no", "home_write:no"] {
        assert!(out.contains(want), "{want} in:\n{out}");
    }
}

#[test]
fn switching_a_permission_off_takes_it_away() {
    let w = world("off", r#""files.videos", "network""#, Some(r#""network", "files.videos""#));
    let out = run(&w, listener());
    assert!(out.contains("videos:no"), "{out}");
    if nexa_sys::sandbox::abi() >= 4 {
        assert!(out.contains("net:no"), "{out}");
    }
}

#[test]
fn explain_and_refusals() {
    let w = world("explain", r#""files.videos", "microphone""#, None);
    let out = Command::new(env!("CARGO_BIN_EXE_nexa-run")).args(["--explain", "probe"])
        .env("HOME", &w.home).env("NEXA_RUN_REGISTRY", w.root.join("registry")).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("files.videos") && text.contains("no internet") && text.contains("microphone: declared, not enforced yet"), "{text}");
    let bad = Command::new(env!("CARGO_BIN_EXE_nexa-run")).arg("../../bin/sh").output().unwrap();
    assert!(!bad.status.success());
    let ghost = Command::new(env!("CARGO_BIN_EXE_nexa-run")).arg("ghost")
        .env("HOME", &w.home).env("NEXA_RUN_REGISTRY", w.root.join("registry")).output().unwrap();
    assert!(String::from_utf8_lossy(&ghost.stderr).contains("isn't an installed app"));
}
