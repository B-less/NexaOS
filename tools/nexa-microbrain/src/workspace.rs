//! PocketWorkspace: a named set of apps, the folder or files each opens,
//! and where their windows were. Saved from what's on screen, restored on
//! request. A workspace holds no commands: apps are .desktop ids that must
//! be installed, paths must exist inside your home folder or on a drive
//! you plugged in (symlinks resolved first). Restoring returns a plan the
//! desktop carries out, and a list of anything that couldn't come back.

use crate::apps::{self, App};
use crate::store::Doc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MAX_WORKSPACES: usize = 30;
pub const MAX_APPS: usize = 16;
pub const MAX_PATHS: usize = 8;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WsApp {
    pub app: String,
    #[serde(default)]
    pub paths: Vec<String>,
    /// x, y, width, height where the window was (best effort to restore).
    #[serde(default)]
    pub rect: Option<[i32; 4]>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Workspace {
    pub name: String,
    pub apps: Vec<WsApp>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub used: u64,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Workspaces(pub Vec<Workspace>);

impl Doc for Workspaces {
    const NAME: &'static str = "workspaces";
    const VERSION: u32 = 1;
}

/// A window on screen, as the desktop reports it.
#[derive(Deserialize, Clone, Debug)]
pub struct Window {
    pub app_id: String,
    #[serde(default)]
    pub rect: Option<[i32; 4]>,
}

pub fn clean_name(s: &str) -> Result<String, String> {
    let n: String = s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(60).collect();
    if n.is_empty() {
        return Err("Give the workspace a name.".into());
    }
    Ok(n)
}

/// A path a workspace may open: it exists, and (symlinks resolved) is
/// inside the home folder or under /media or /mnt.
pub fn safe_path(home: &Path, p: &str) -> Result<PathBuf, String> {
    if p.chars().any(char::is_control) || p.len() > 4096 {
        return Err(format!("{p:?} isn't a usable path"));
    }
    let expanded = match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    };
    if !expanded.is_absolute() {
        return Err(format!("{p} isn't a full path"));
    }
    let real = expanded.canonicalize().map_err(|_| format!("{p} isn't there any more"))?;
    let home_real = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    if real.starts_with(&home_real) || real.starts_with("/media") || real.starts_with("/mnt") {
        Ok(real)
    } else {
        Err(format!("{p} is outside your home folder"))
    }
}

/// Keeps what's valid; says what was dropped and why.
pub fn validate(ws: &Workspace, apps: &BTreeMap<String, App>, home: &Path) -> (Workspace, Vec<String>) {
    let mut problems = Vec::new();
    let mut out = Workspace { name: ws.name.clone(), apps: Vec::new(), folder: None, created: ws.created, used: ws.used };
    if let Some(f) = &ws.folder {
        match safe_path(home, f) {
            Ok(p) if p.is_dir() => out.folder = Some(p.to_string_lossy().into_owned()),
            Ok(_) => problems.push(format!("{f} isn't a folder")),
            Err(e) => problems.push(e),
        }
    }
    for a in ws.apps.iter().take(MAX_APPS) {
        let Some(app) = apps.get(&a.app) else {
            problems.push(format!("{} isn't installed", a.app));
            continue;
        };
        let mut paths = Vec::new();
        for p in a.paths.iter().take(MAX_PATHS) {
            match safe_path(home, p) {
                Ok(real) => paths.push(real.to_string_lossy().into_owned()),
                Err(e) => problems.push(e),
            }
        }
        if !paths.is_empty() && !app.takes_files {
            problems.push(format!("{} can't open files, so its paths were left out", app.name));
            paths.clear();
        }
        out.apps.push(WsApp { app: a.app.clone(), paths, rect: a.rect });
    }
    if ws.apps.len() > MAX_APPS {
        problems.push(format!("only the first {MAX_APPS} apps are kept"));
    }
    (out, problems)
}

/// A workspace from the windows on screen. The project folder (if any)
/// goes to the apps that open folders (Files, a terminal).
pub fn from_windows(name: &str, windows: &[Window], folder: Option<&str>, apps: &BTreeMap<String, App>, home: &Path, now: u64)
                    -> Result<(Workspace, Vec<String>), String> {
    let name = clean_name(name)?;
    let mut list = Vec::new();
    let mut skipped = Vec::new();
    for w in windows.iter().take(MAX_APPS) {
        match apps::for_window(apps, &w.app_id) {
            Some(app) if !app.hidden || app.id.starts_with("nexa-") => {
                list.push(WsApp { app: app.id.clone(), paths: Vec::new(), rect: w.rect });
            }
            _ => skipped.push(format!("a window of {} (not an installed app)", w.app_id)),
        }
    }
    if list.is_empty() {
        return Err("No apps to save: open the apps you want in this workspace first.".into());
    }
    let ws = Workspace { name, apps: list, folder: folder.map(str::to_string), created: now, used: 0 };
    let (clean, mut problems) = validate(&ws, apps, home);
    problems.extend(skipped);
    Ok((clean, problems))
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Launch {
    pub app: String,
    pub name: String,
    pub paths: Vec<String>,
    pub rect: Option<[i32; 4]>,
}

/// What restoring a workspace does, checked against what's there now.
pub fn plan(ws: &Workspace, apps: &BTreeMap<String, App>, home: &Path) -> (Vec<Launch>, Vec<String>) {
    let (clean, problems) = validate(ws, apps, home);
    let launches = clean.apps.iter().map(|a| {
        let app = &apps[&a.app];
        // The project folder opens in apps that take files and were given none.
        let paths = if a.paths.is_empty() && app.takes_files { clean.folder.iter().cloned().collect() } else { a.paths.clone() };
        Launch { app: a.app.clone(), name: app.name.clone(), paths, rect: a.rect }
    }).collect();
    (launches, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(tag: &str) -> (PathBuf, BTreeMap<String, App>) {
        let home = std::env::temp_dir().join(format!("mb-ws-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(home.join("Projects/MyApp")).unwrap();
        std::fs::write(home.join("Projects/MyApp/main.rs"), "fn main() {}").unwrap();
        let mut apps = BTreeMap::new();
        for (id, text) in [("nexa-edit", "[Desktop Entry]\nName=Editor\nExec=nexa-edit %F\n"),
                           ("nexa-files", "[Desktop Entry]\nName=Files\nExec=nexa-files %U\n"),
                           ("foot", "[Desktop Entry]\nName=Terminal\nExec=foot\n"),
                           ("nexa-calc", "[Desktop Entry]\nName=Calculator\nExec=nexa-calc\n")] {
            apps.insert(id.into(), apps::parse(id, text).unwrap());
        }
        (home, apps)
    }

    #[test]
    fn saves_what_is_on_screen_and_plans_it_back() {
        let (home, apps) = setup("a");
        let windows = vec![Window { app_id: "nexa-edit".into(), rect: Some([10, 50, 800, 600]) },
                           Window { app_id: "foot".into(), rect: None },
                           Window { app_id: "nexa-files".into(), rect: None },
                           Window { app_id: "steam".into(), rect: None }];
        let (ws, problems) = from_windows("  Android Development ", &windows, Some("~/Projects/MyApp"), &apps, &home, 5).unwrap();
        assert_eq!(ws.name, "Android Development");
        assert_eq!(ws.apps.len(), 3);
        assert!(problems.iter().any(|p| p.contains("steam")), "{problems:?}");
        let (launches, _) = plan(&ws, &apps, &home);
        let folder = home.join("Projects/MyApp").canonicalize().unwrap().to_string_lossy().into_owned();
        assert_eq!(launches[0].paths, vec![folder.clone()], "Editor gets the project folder");
        assert_eq!(launches[1].paths, Vec::<String>::new(), "the terminal can't open files");
        assert_eq!(launches[2].paths, vec![folder]);
        assert_eq!(launches[0].rect, Some([10, 50, 800, 600]));
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn nothing_outside_home_and_no_unknown_apps() {
        let (home, apps) = setup("b");
        std::os::unix::fs::symlink("/etc", home.join("sneaky")).unwrap();
        let ws = Workspace { name: "x".into(), folder: Some("~/sneaky".into()), created: 0, used: 0, apps: vec![
            WsApp { app: "nexa-edit".into(), paths: vec!["/etc/shadow".into(), "~/../../etc/passwd".into(), "~/Projects/MyApp/main.rs".into()], rect: None },
            WsApp { app: "rm -rf ~".into(), paths: vec![], rect: None },
            WsApp { app: "nexa-calc".into(), paths: vec!["~/Projects".into()], rect: None }] };
        let (clean, problems) = validate(&ws, &apps, &home);
        assert_eq!(clean.folder, None, "a symlink out of home is refused");
        assert_eq!(clean.apps.len(), 2, "the made-up app is dropped");
        assert_eq!(clean.apps[0].paths.len(), 1, "only the file inside home stays");
        assert!(clean.apps[1].paths.is_empty(), "the calculator can't open files");
        assert!(problems.iter().any(|p| p.contains("isn't installed")));
        assert!(problems.iter().filter(|p| p.contains("outside your home")).count() >= 2, "{problems:?}");
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn a_vanished_folder_is_reported_not_fatal() {
        let (home, apps) = setup("c");
        let ws = Workspace { name: "x".into(), folder: Some("~/Gone".into()), created: 0, used: 0,
                             apps: vec![WsApp { app: "nexa-files".into(), paths: vec![], rect: None }] };
        let (launches, problems) = plan(&ws, &apps, &home);
        assert_eq!(launches.len(), 1);
        assert!(problems[0].contains("isn't there any more"));
        assert!(clean_name("   ").is_err());
        std::fs::remove_dir_all(home).ok();
    }
}
