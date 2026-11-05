//! nexa-run: starts an installed .nexa app confined to what it declared.
//!
//!   video-player [FILE...]          (a link to nexa-run: the usual way)
//!   nexa-run ID [ARGS...]
//!   nexa-run --explain ID           what the app may touch, and why
//!
//! The installer puts the app's program in /usr/local/libexec/nexa-apps/<id>
//! and makes /usr/local/bin/<id> a link to this launcher. The launcher reads
//! the app's permissions from the root-owned install record
//! (/var/lib/nexa-packages/<id>.json), takes away any the person switched
//! off (~/.config/nexa/app-permissions.json, which can only remove), locks
//! itself down with Landlock to exactly that, then becomes the app.
//!
//! Enforced: files (only the app's own folders and the ones its files.*
//! permissions name; a file given on the command line is readable), TCP
//! network (none without "network"), the camera (/dev/video*). Not
//! enforceable with Landlock (yet): the microphone and notifications (they
//! go through PipeWire and D-Bus sockets), location, background, and UDP.

use nexa_sys::sandbox::{self, READ, READ_EXEC, READ_WRITE};
use serde_json::Value;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

const REGISTRY: &str = "/var/lib/nexa-packages";
const LIBEXEC: &str = "/usr/local/libexec/nexa-apps";
const LIB: &str = "/usr/local/lib";

/// Permissions this launcher knows how to enforce.
pub const ENFORCED: &[&str] = &["files.home", "files.documents", "files.pictures", "files.music", "files.videos", "files.drives", "network", "camera"];

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().next().map_or(false, |c| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_".contains(c))
}

/// Everything the plan needs to know about this person and machine.
pub struct Env {
    pub home: PathBuf,
    pub uid: u32,
    /// XDG user folders (Documents, Pictures, ...) from user-dirs.dirs.
    pub xdg: Vec<(String, PathBuf)>,
    pub cameras: Vec<PathBuf>,
    /// Where the system broker mounts removable drives (/media).
    pub media: PathBuf,
}

#[derive(Debug, PartialEq)]
pub struct Plan {
    pub rules: Vec<(PathBuf, u64)>,
    pub any_network: bool,
    pub granted: Vec<String>,
    pub not_enforced: Vec<String>,
}

fn xdg_dir(env: &Env, key: &str, default: &str) -> PathBuf {
    env.xdg.iter().find(|(k, _)| k == key).map(|(_, p)| p.clone()).unwrap_or_else(|| env.home.join(default))
}

/// The rules for one app: pure, so it can be tested.
pub fn plan(id: &str, lib: &Path, program: &Path, permissions: &[String], env: &Env, files: &[PathBuf]) -> Plan {
    let h = |p: &str| env.home.join(p);
    let mut rules: Vec<(PathBuf, u64)> = Vec::new();
    // The system, read-only; the app's own program and data.
    for p in ["/usr", "/lib", "/bin", "/sbin"] {
        rules.push((p.into(), READ_EXEC));
    }
    for p in ["/etc", "/proc", "/sys", "/run", "/var/cache/fontconfig"] {
        rules.push((p.into(), READ));
    }
    rules.push((program.to_path_buf(), READ_EXEC));
    rules.push((lib.join(format!("nexa-{id}")), READ));
    // Screen, sound, GPU and the session: every desktop app needs them.
    for p in ["/dev/dri", "/dev/snd", "/dev/shm", "/dev/null", "/dev/zero", "/dev/urandom", "/dev/random", "/tmp"] {
        rules.push((p.into(), READ_WRITE));
    }
    rules.push((PathBuf::from(format!("/run/user/{}", env.uid)), READ_WRITE));
    // What GTK reads from the home folder: theme, fonts, icons, folder names.
    for p in [".config/gtk-3.0", ".config/fontconfig", ".local/share/fonts", ".fonts", ".icons", ".local/share/icons",
              ".local/share/mime", ".config/user-dirs.dirs", ".config/dconf"] {
        rules.push((h(p), READ));
    }
    for p in [".cache/fontconfig", ".cache/mesa_shader_cache", ".local/share/recently-used.xbel"] {
        rules.push((h(p), READ_WRITE));
    }
    // Its own folders.
    for p in [format!(".local/share/{id}"), format!(".config/{id}"), format!(".cache/{id}")] {
        rules.push((h(&p), READ_WRITE));
    }
    // What it was given.
    let mut granted = Vec::new();
    let mut not_enforced = Vec::new();
    let mut any_network = false;
    for perm in permissions {
        let folder = match perm.as_str() {
            "files.home" => Some(env.home.clone()),
            "files.documents" => Some(xdg_dir(env, "XDG_DOCUMENTS_DIR", "Documents")),
            "files.pictures" => Some(xdg_dir(env, "XDG_PICTURES_DIR", "Pictures")),
            "files.music" => Some(xdg_dir(env, "XDG_MUSIC_DIR", "Music")),
            "files.videos" => Some(xdg_dir(env, "XDG_VIDEOS_DIR", "Videos")),
            // USB sticks and other removable drives, as the broker mounts them.
            "files.drives" => Some(env.media.clone()),
            _ => None,
        };
        if let Some(f) = folder {
            rules.push((f, READ_WRITE));
            granted.push(perm.clone());
        } else if perm == "network" {
            any_network = true;
            granted.push(perm.clone());
        } else if perm == "camera" {
            for c in &env.cameras {
                rules.push((c.clone(), READ_WRITE));
            }
            granted.push(perm.clone());
        } else {
            not_enforced.push(perm.clone());
        }
    }
    // A file the person opened with the app (on the command line) is readable.
    for f in files {
        rules.push((f.clone(), READ));
    }
    Plan { rules, any_network, granted, not_enforced }
}

fn read_user_dirs(home: &Path) -> Vec<(String, PathBuf)> {
    std::fs::read_to_string(home.join(".config/user-dirs.dirs")).unwrap_or_default().lines().filter_map(|l| {
        let (k, v) = l.split_once('=')?;
        let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
        (k.starts_with("XDG_") && v.starts_with('/') && !v.contains("..")).then(|| (k.to_string(), PathBuf::from(v)))
    }).collect()
}

/// The permissions recorded at install, minus what the person switched off.
fn permissions(registry: &Path, id: &str, home: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    let rec: Value = std::fs::read(registry.join(format!("{id}.json"))).ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| format!("'{id}' isn't an installed app"))?;
    let declared: Vec<String> = rec["permissions"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let prefs: Value = std::fs::read(home.join(".config/nexa/app-permissions.json")).ok()
        .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let denied: Vec<String> = prefs[id]["denied"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let effective = declared.iter().filter(|p| !denied.contains(p)).cloned().collect();
    Ok((effective, denied.into_iter().filter(|d| declared.contains(d)).collect()))
}

fn env_path(var: &str, default: &str) -> PathBuf {
    // Tests point these at a fake system. A person can set them too, but
    // the sandbox protects people from apps, not from themselves: they can
    // always run the program directly.
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(default))
}

fn die(msg: &str) -> ! {
    eprintln!("nexa-run: {msg}");
    std::process::exit(126);
}

fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    let called = Path::new(&args[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let explain = args.get(1).map(String::as_str) == Some("--explain");
    let (id, app_args) = if called != "nexa-run" {
        (called, args.split_off(1))
    } else {
        let rest = args.split_off(if explain { 2 } else { 1 });
        let mut it = rest.into_iter();
        (it.next().unwrap_or_else(|| die("usage: nexa-run ID [ARGS...] | nexa-run --explain ID")), it.collect())
    };
    if !valid_id(&id) {
        die(&format!("'{id}' isn't a valid app id"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| die("HOME is not set"));
    let (registry, libexec, lib) = (env_path("NEXA_RUN_REGISTRY", REGISTRY), env_path("NEXA_RUN_LIBEXEC", LIBEXEC), env_path("NEXA_RUN_LIB", LIB));
    let program = libexec.join(&id);
    let (perms, switched_off) = permissions(&registry, &id, &home).unwrap_or_else(|e| die(&e));
    let cameras = std::fs::read_dir("/dev").into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("video"))).collect();
    // SAFETY: getuid can't fail.
    let env = Env { home: home.clone(), uid: unsafe { libc::getuid() }, xdg: read_user_dirs(&home), cameras,
                    media: env_path("NEXA_RUN_MEDIA", "/media") };
    let files: Vec<PathBuf> = app_args.iter().filter(|a| !a.starts_with('-'))
        .filter_map(|a| std::fs::canonicalize(a).ok()).filter(|p| p.is_file()).collect();
    let p = plan(&id, &lib, &program, &perms, &env, &files);

    if explain {
        println!("{id} may use:");
        println!("  its own data: ~/.local/share/{id}, ~/.config/{id}, ~/.cache/{id}");
        for g in &p.granted {
            println!("  {g}");
        }
        if !p.any_network {
            println!("  no internet (TCP blocked)");
        }
        for s in &switched_off {
            println!("  {s}: switched off by you");
        }
        for n in &p.not_enforced {
            println!("  {n}: declared, not enforced yet");
        }
        return;
    }
    if !program.is_file() {
        die(&format!("{} is missing; reinstall the app", program.display()));
    }
    // Folders it may write must exist before the doors close (Landlock can
    // only allow what exists): its own, and the font and GPU caches, without
    // which every start would rescan all fonts.
    for d in [format!(".local/share/{id}"), format!(".config/{id}"), format!(".cache/{id}"),
              ".cache/fontconfig".into(), ".cache/mesa_shader_cache".into()] {
        let _ = std::fs::create_dir_all(home.join(d));
    }
    let rules = sandbox::Rules {
        fs: p.rules.iter().map(|(path, r)| (path.to_string_lossy().into_owned(), *r)).collect(),
        connect_ports: vec![],
        any_network: p.any_network,
    };
    match sandbox::restrict(&rules) {
        Ok(abi) if abi > 0 => {}
        Ok(_) => die("this kernel has no Landlock, so apps can't be kept to their permissions; not starting"),
        Err(e) => die(&format!("couldn't apply the app's permissions: {e}")),
    }
    let err = std::process::Command::new(&program).arg0(&id).args(&app_args)
        .env("NEXA_APP_ID", &id).env("NEXA_APP_PERMISSIONS", p.granted.join(",")).exec();
    die(&format!("couldn't start {}: {err}", program.display()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env { home: "/home/ama".into(), uid: 1000, xdg: vec![("XDG_VIDEOS_DIR".into(), "/home/ama/Films".into())],
              cameras: vec!["/dev/video0".into()], media: "/media".into() }
    }

    fn paths(p: &Plan) -> Vec<String> {
        p.rules.iter().map(|(x, _)| x.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn nothing_declared_means_only_its_own_folders() {
        let p = plan("notes", Path::new("/usr/local/lib"), Path::new("/x/notes"), &[], &env(), &[]);
        let ps = paths(&p);
        assert!(ps.contains(&"/home/ama/.local/share/notes".into()) && ps.contains(&"/home/ama/.config/notes".into()));
        assert!(!ps.contains(&"/home/ama".into()) && !ps.iter().any(|x| x.contains("Documents") || x.contains("Films")));
        assert!(!p.any_network && !ps.contains(&"/dev/video0".into()));
        // Home itself is never writable without files.home; secrets are never named.
        assert!(!ps.iter().any(|x| x.contains(".ssh") || x.contains(".gnupg")));
    }

    #[test]
    fn permissions_open_exactly_their_folder() {
        let perms: Vec<String> = ["files.videos", "network", "camera", "microphone"].iter().map(|s| s.to_string()).collect();
        let p = plan("vp", Path::new("/usr/local/lib"), Path::new("/x/vp"), &perms, &env(), &[]);
        let ps = paths(&p);
        assert!(ps.contains(&"/home/ama/Films".into()), "the person's own Videos folder name is used");
        assert!(!ps.contains(&"/home/ama".into()));
        assert!(p.any_network && ps.contains(&"/dev/video0".into()));
        assert_eq!(p.not_enforced, vec!["microphone"]);
        let drives = plan("bk", Path::new("/l"), Path::new("/x/bk"), &["files.drives".into()], &env(), &[]);
        assert!(paths(&drives).contains(&"/media".into()) && !paths(&drives).contains(&"/home/ama".into()), "drives: /media only");
        let all = plan("fm", Path::new("/l"), Path::new("/x/fm"), &["files.home".into()], &env(), &[]);
        assert!(paths(&all).contains(&"/home/ama".into()));
    }

    #[test]
    fn opened_files_are_readable_only() {
        let f = PathBuf::from("/media/STICK/film.mkv");
        let p = plan("vp", Path::new("/l"), Path::new("/x/vp"), &[], &env(), &[f.clone()]);
        assert!(p.rules.contains(&(f, READ)));
    }

    #[test]
    fn ids_and_user_dirs() {
        assert!(valid_id("video-player") && !valid_id("../sh") && !valid_id("Video") && !valid_id(""));
        let d = std::env::temp_dir().join(format!("nexa-run-ud-{}", std::process::id()));
        std::fs::create_dir_all(d.join(".config")).unwrap();
        std::fs::write(d.join(".config/user-dirs.dirs"), "XDG_VIDEOS_DIR=\"$HOME/Films\"\nXDG_MUSIC_DIR=\"/etc/../root\"\n").unwrap();
        let x = read_user_dirs(&d);
        assert_eq!(x, vec![("XDG_VIDEOS_DIR".to_string(), d.join("Films"))], "a path with .. is ignored");
    }

    #[test]
    fn switched_off_permissions_are_removed_and_can_only_remove() {
        let d = std::env::temp_dir().join(format!("nexa-run-perm-{}", std::process::id()));
        std::fs::create_dir_all(d.join("reg")).unwrap();
        std::fs::create_dir_all(d.join("home/.config/nexa")).unwrap();
        std::fs::write(d.join("reg/vp.json"), r#"{"permissions": ["files.videos", "network"]}"#).unwrap();
        std::fs::write(d.join("home/.config/nexa/app-permissions.json"), r#"{"vp": {"denied": ["network", "files.home"]}}"#).unwrap();
        let (eff, off) = permissions(&d.join("reg"), "vp", &d.join("home")).unwrap();
        assert_eq!(eff, vec!["files.videos"]);
        assert_eq!(off, vec!["network"], "denying something never declared means nothing");
        assert!(permissions(&d.join("reg"), "ghost", &d.join("home")).is_err());
    }
}
