//! The installed apps, from their .desktop files: the only apps a
//! workspace, a skill or a command can name. A name that isn't an
//! installed app is refused, so nothing stored can smuggle in a command.

use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct App {
    /// "nexa-files" for nexa-files.desktop.
    pub id: String,
    pub name: String,
    pub icon: String,
    /// The app opens files or folders it's given (%f %F %u %U in Exec).
    pub takes_files: bool,
    pub hidden: bool,
    pub keywords: Vec<String>,
    pub wm_class: Option<String>,
}

pub fn dirs(home: &Path) -> Vec<PathBuf> {
    vec![home.join(".local/share/applications"), "/usr/local/share/applications".into(), "/usr/share/applications".into()]
}

pub fn parse(id: &str, text: &str) -> Option<App> {
    let mut in_entry = false;
    let mut kv: BTreeMap<&str, &str> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            kv.entry(k.trim()).or_insert(v.trim());
        }
    }
    if kv.get("Type").copied().unwrap_or("Application") != "Application" || !kv.contains_key("Exec") {
        return None;
    }
    let exec = kv.get("Exec").copied().unwrap_or("");
    Some(App {
        id: id.to_string(),
        name: kv.get("Name").copied().unwrap_or(id).to_string(),
        icon: kv.get("Icon").copied().unwrap_or("").to_string(),
        takes_files: ["%f", "%F", "%u", "%U"].iter().any(|c| exec.contains(c)),
        hidden: kv.get("NoDisplay").is_some_and(|v| *v == "true") || kv.get("Hidden").is_some_and(|v| *v == "true"),
        keywords: kv.get("Keywords").map(|k| k.split(';').filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).collect()).unwrap_or_default(),
        wm_class: kv.get("StartupWMClass").map(|s| s.to_string()),
    })
}

/// Every installed app; the user's own .desktop files win over the system's.
pub fn index(home: &Path) -> BTreeMap<String, App> {
    let mut out = BTreeMap::new();
    for dir in dirs(home).iter().rev() {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".desktop") else { continue };
            if let Ok(text) = fs::read_to_string(e.path()) {
                if let Some(app) = parse(id, &text) {
                    out.insert(id.to_string(), app);
                }
            }
        }
    }
    out
}

/// The app behind a window: its app_id is usually the .desktop id, or its
/// StartupWMClass ("foot" for foot.desktop, "org.gnome.Foo"...).
pub fn for_window<'a>(apps: &'a BTreeMap<String, App>, app_id: &str) -> Option<&'a App> {
    let lower = app_id.to_lowercase();
    apps.get(app_id)
        .or_else(|| apps.values().find(|a| a.id.to_lowercase() == lower))
        .or_else(|| apps.values().find(|a| a.wm_class.as_deref().is_some_and(|c| c.to_lowercase() == lower)))
        .or_else(|| apps.values().find(|a| a.id.rsplit('.').next().is_some_and(|tail| tail.to_lowercase() == lower)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_desktop_files() {
        let a = parse("nexa-edit", "[Desktop Entry]\nType=Application\nName=Nexa Editor\nExec=/usr/local/bin/nexa-edit %F\nIcon=nexa-edit\nKeywords=text;code;\n[Desktop Action new]\nName=Ignored\n").unwrap();
        assert_eq!((a.name.as_str(), a.takes_files, a.hidden), ("Nexa Editor", true, false));
        assert_eq!(a.keywords, vec!["text", "code"]);
        assert!(parse("x", "[Desktop Entry]\nType=Link\nURL=http://x\n").is_none(), "links aren't apps");
        assert!(parse("y", "[Desktop Entry]\nName=No exec\n").is_none());
        assert!(parse("z", "[Desktop Entry]\nExec=z\nNoDisplay=true\n").unwrap().hidden);
    }

    #[test]
    fn finds_the_app_behind_a_window() {
        let mut apps = BTreeMap::new();
        for (id, text) in [("nexa-files", "[Desktop Entry]\nExec=nexa-files %U\nName=Files\n"),
                           ("org.codeberg.dnkl.foot", "[Desktop Entry]\nExec=foot\nName=Foot\nStartupWMClass=foot\n")] {
            apps.insert(id.to_string(), parse(id, text).unwrap());
        }
        assert_eq!(for_window(&apps, "nexa-files").unwrap().id, "nexa-files");
        assert_eq!(for_window(&apps, "foot").unwrap().id, "org.codeberg.dnkl.foot");
        assert!(for_window(&apps, "unknown").is_none());
    }
}
