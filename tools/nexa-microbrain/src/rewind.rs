//! Rewind (Magic Layer): a timeline of settings changes, and "go back to
//! just before this". MicroBrain watches the desktop's settings files
//! (inotify: no polling), keeps what each looked like before a change, and
//! groups changes made within a few seconds into one moment (a theme
//! change touches several things at once).
//!
//!   {"cmd": "rewind"}                 the moments, newest first
//!   {"cmd": "rewind-to", "id": N}     every file changed at or after moment
//!                                     N goes back to how it was just before
//!
//! Going back is itself a moment, so it can be undone the same way. Only
//! settings files are tracked, never documents; security choices (app
//! permissions, login and lock rules) are left out on purpose: rewinding
//! must never quietly undo one. History: the last 100 moments, at most
//! 1 MB, in MicroBrain's own folder.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// (file under home, what people call it, how the desktop re-applies it).
pub const TRACKED: &[(&str, &str, &str)] = &[
    (".config/nexa/theme.json", "Theme", "theme"),
    (".config/nexa/magic.json", "Magic Layer", "magic"),
    (".config/nexa/topbar", "Top bar", "topbar"),
    (".config/nexa/idle.conf", "Screen lock and screen off timing", "idle"),
    (".config/nexa/field.json", "Settings layout", "none"),
    (".config/nexa/interface/global.json", "Interface DNA", "none"),
    (".config/sway/config.d/10-nexa-keyboard.conf", "Keyboard layout", "keyboard"),
];

const MAX_FILE: u64 = 64 * 1024;
const KEEP: usize = 100;
const MAX_BYTES: usize = 1 << 20;
const GROUP_SECS: u64 = 3;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Change {
    pub file: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Moment {
    pub id: u64,
    pub at: u64,
    pub changes: Vec<Change>,
    /// This moment was going back to before that one.
    #[serde(default)]
    pub rewind_of: Option<u64>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    next_id: u64,
    moments: VecDeque<Moment>,
}

pub struct Rewind {
    home: PathBuf,
    file: PathBuf,
    current: HashMap<&'static str, Option<String>>,
    saved: Saved,
    pending_rewind: Option<u64>,
}

fn read(path: &Path) -> Option<String> {
    let md = std::fs::metadata(path).ok()?;
    if !md.is_file() || md.len() > MAX_FILE {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

pub fn label(file: &str) -> &'static str {
    TRACKED.iter().find(|t| t.0 == file).map(|t| t.1).unwrap_or("Settings")
}

pub fn apply_of(file: &str) -> &'static str {
    TRACKED.iter().find(|t| t.0 == file).map(|t| t.2).unwrap_or("none")
}

/// key -> value, flattened ("a.b"), for JSON files and key=value files.
fn pairs(text: &str) -> BTreeMap<String, String> {
    fn walk(prefix: &str, v: &Value, out: &mut BTreeMap<String, String>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    walk(&if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, x, out);
                }
            }
            Value::String(s) => {
                out.insert(prefix.to_string(), s.clone());
            }
            other => {
                out.insert(prefix.to_string(), other.to_string());
            }
        }
    }
    let mut out = BTreeMap::new();
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        walk("", &v, &mut out);
        return out;
    }
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        match l.split_once('=') {
            Some((k, v)) => out.insert(k.trim().to_string(), v.trim().to_string()),
            None => out.insert(String::new(), l.to_string()),
        };
    }
    out
}

/// What changed, in a few words: ["theme: aurora → horizon", ...].
pub fn summary(before: Option<&str>, after: Option<&str>) -> Vec<String> {
    match (before, after) {
        (None, None) => vec![],
        (None, Some(_)) => vec!["set for the first time".into()],
        (Some(_), None) => vec!["back to the defaults (the file was removed)".into()],
        (Some(b), Some(a)) => {
            let (pb, pa) = (pairs(b), pairs(a));
            let mut out = Vec::new();
            for k in pb.keys().chain(pa.keys().filter(|k| !pb.contains_key(*k))) {
                let (x, y) = (pb.get(k), pa.get(k));
                if x != y {
                    let name = if k.is_empty() { String::new() } else { format!("{k}: ") };
                    out.push(format!("{name}{} \u{2192} {}", x.map(String::as_str).unwrap_or("(none)"), y.map(String::as_str).unwrap_or("(none)")));
                }
            }
            let more = out.len().saturating_sub(4);
            out.truncate(4);
            if more > 0 {
                out.push(format!("and {more} more"));
            }
            if out.is_empty() {
                out.push("reformatted, same values".into());
            }
            out
        }
    }
}

impl Rewind {
    pub fn new(home: PathBuf, store_dir: &Path) -> Rewind {
        let file = store_dir.join("rewind.json");
        let saved: Saved = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let current = TRACKED.iter().map(|t| (t.0, read(&home.join(t.0)))).collect();
        Rewind { home, file, current, saved, pending_rewind: None }
    }

    /// The folders to watch (those that exist).
    pub fn dirs(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = TRACKED.iter().filter_map(|t| self.home.join(t.0).parent().map(Path::to_path_buf)).collect();
        v.sort();
        v.dedup();
        v.retain(|d| d.is_dir());
        v
    }

    /// Look at every tracked file; record what changed. True if anything did.
    pub fn scan(&mut self, now: u64) -> bool {
        let mut changed = Vec::new();
        for t in TRACKED {
            let cur = read(&self.home.join(t.0));
            let old = self.current.get(t.0).cloned().flatten();
            if cur != old {
                changed.push(Change { file: t.0.to_string(), before: old, after: cur.clone() });
                self.current.insert(t.0, cur);
            }
        }
        if changed.is_empty() {
            return false;
        }
        let rewind_of = self.pending_rewind.take();
        let join = rewind_of.is_none()
            && self.saved.moments.back().is_some_and(|m| m.rewind_of.is_none() && now.saturating_sub(m.at) <= GROUP_SECS);
        if join {
            let m = self.saved.moments.back_mut().expect("checked");
            for c in changed {
                match m.changes.iter_mut().find(|x| x.file == c.file) {
                    Some(x) => x.after = c.after, // same file again: keep the first "before"
                    None => m.changes.push(c),
                }
            }
            m.at = now;
        } else {
            self.saved.next_id += 1;
            self.saved.moments.push_back(Moment { id: self.saved.next_id, at: now, changes: changed, rewind_of });
        }
        self.trim();
        self.save();
        true
    }

    fn trim(&mut self) {
        while self.saved.moments.len() > KEEP {
            self.saved.moments.pop_front();
        }
        while self.saved.moments.len() > 1 && serde_json::to_vec(&self.saved).map(|v| v.len()).unwrap_or(0) > MAX_BYTES {
            self.saved.moments.pop_front();
        }
    }

    fn save(&self) {
        if let Ok(text) = serde_json::to_vec(&self.saved) {
            let tmp = self.file.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &self.file);
            }
        }
    }

    pub fn list(&self, limit: usize) -> Vec<Value> {
        self.saved.moments.iter().rev().take(limit).map(|m| {
            json!({
                "id": m.id, "at": m.at, "rewind_of": m.rewind_of,
                "changes": m.changes.iter().map(|c| json!({
                    "file": c.file, "label": label(&c.file), "apply": apply_of(&c.file),
                    "summary": summary(c.before.as_deref(), c.after.as_deref()),
                })).collect::<Vec<_>>(),
            })
        }).collect()
    }

    /// Every file changed at or after moment `id` back to how it was just
    /// before it. Returns what to re-apply: [(label, apply)].
    pub fn rewind_to(&mut self, id: u64, now: u64) -> Result<Vec<(&'static str, &'static str)>, String> {
        if !self.saved.moments.iter().any(|m| m.id == id) {
            return Err("That moment is no longer in the history.".into());
        }
        let mut target: BTreeMap<String, Option<String>> = BTreeMap::new();
        for m in self.saved.moments.iter().filter(|m| m.id >= id) {
            for c in &m.changes {
                target.entry(c.file.clone()).or_insert_with(|| c.before.clone());
            }
        }
        let mut done = Vec::new();
        for (file, content) in &target {
            let path = self.home.join(file);
            let r = match content {
                Some(text) => {
                    let tmp = path.with_extension("rewind-tmp");
                    std::fs::create_dir_all(path.parent().unwrap_or(&self.home))
                        .and_then(|_| std::fs::write(&tmp, text))
                        .and_then(|_| std::fs::rename(&tmp, &path))
                }
                None => match std::fs::remove_file(&path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                    _ => Ok(()),
                },
            };
            r.map_err(|e| format!("{}: {e}", label(file)))?;
            let t = TRACKED.iter().find(|t| t.0 == file.as_str()).expect("only tracked files are recorded");
            done.push((t.1, t.2));
        }
        // Record the going-back as its own moment (so it can be undone).
        self.pending_rewind = Some(id);
        self.scan(now);
        self.pending_rewind = None;
        Ok(done)
    }
}

// ---- watching ------------------------------------------------------------------

/// Watch the settings folders and scan when something in them is written.
/// Runs on its own thread; sleeps in read() between changes.
pub fn watch(r: std::sync::Arc<std::sync::Mutex<Rewind>>) {
    // SAFETY: plain syscalls on a descriptor we own.
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        return;
    }
    let mut watched: Vec<PathBuf> = Vec::new();
    let add = |watched: &mut Vec<PathBuf>, dirs: Vec<PathBuf>| {
        for d in dirs {
            if watched.contains(&d) {
                continue;
            }
            if let Ok(c) = std::ffi::CString::new(d.as_os_str().as_encoded_bytes()) {
                let mask = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_DELETE | libc::IN_CREATE;
                // SAFETY: valid fd and C string.
                if unsafe { libc::inotify_add_watch(fd, c.as_ptr(), mask) } >= 0 {
                    watched.push(d);
                }
            }
        }
    };
    // Folders that don't exist yet (a first change): watch the nearest existing parent too.
    let parents = |r: &Rewind| -> Vec<PathBuf> {
        let mut v = r.dirs();
        for t in TRACKED {
            let mut p = r.home.join(t.0);
            while let Some(parent) = p.parent().map(Path::to_path_buf) {
                if parent.is_dir() {
                    v.push(parent);
                    break;
                }
                p = parent;
            }
        }
        v.sort();
        v.dedup();
        v
    };
    let first = parents(&r.lock().expect("rewind lock"));
    add(&mut watched, first);
    let mut buf = [0u8; 4096];
    loop {
        // SAFETY: reading into our own buffer.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        // Let a burst of writes finish (a theme change writes several files).
        std::thread::sleep(std::time::Duration::from_millis(400));
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: drain what else arrived meanwhile.
        while unsafe { libc::poll(&mut pfd, 1, 0) } > 0 && unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
        let mut g = r.lock().expect("rewind lock");
        g.scan(crate::now());
        let more = parents(&g);
        drop(g);
        add(&mut watched, more);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (PathBuf, PathBuf) {
        let d = std::env::temp_dir().join(format!("rewind-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("home/.config/nexa")).unwrap();
        std::fs::create_dir_all(d.join("store")).unwrap();
        (d.join("home"), d.join("store"))
    }

    fn put(home: &Path, file: &str, text: &str) {
        std::fs::create_dir_all(home.join(file).parent().unwrap()).unwrap();
        std::fs::write(home.join(file), text).unwrap();
    }

    #[test]
    fn changes_are_grouped_summarised_and_rewound() {
        let (home, store) = setup();
        put(&home, ".config/nexa/theme.json", r#"{"theme": "horizon", "opacity": 0.9}"#);
        put(&home, ".config/nexa/idle.conf", "lock_minutes=5\nblank_minutes=10\n");
        let mut r = Rewind::new(home.clone(), &store);
        assert!(!r.scan(100), "nothing changed yet");
        // A theme change, and a second file a second later: one moment.
        put(&home, ".config/nexa/theme.json", r#"{"theme": "aurora", "opacity": 0.9}"#);
        assert!(r.scan(100));
        put(&home, ".config/nexa/topbar", "autohide\n");
        r.scan(101);
        // Much later: the idle timing.
        put(&home, ".config/nexa/idle.conf", "lock_minutes=15\nblank_minutes=10\n");
        r.scan(500);
        let l = r.list(10);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0]["changes"][0]["summary"][0], "lock_minutes: 5 \u{2192} 15");
        assert_eq!(l[1]["changes"].as_array().unwrap().len(), 2, "theme + top bar in one moment");
        assert_eq!(l[1]["changes"][0]["summary"][0], "theme: horizon \u{2192} aurora");
        assert_eq!(l[1]["changes"][1]["summary"][0], "set for the first time");

        // Back to before the theme moment: theme, top bar AND the later idle change go back.
        let first = l[1]["id"].as_u64().unwrap();
        let done = r.rewind_to(first, 600).unwrap();
        assert!(done.contains(&("Theme", "theme")) && done.contains(&("Top bar", "topbar")));
        assert!(std::fs::read_to_string(home.join(".config/nexa/theme.json")).unwrap().contains("horizon"));
        assert!(!home.join(".config/nexa/topbar").exists(), "it didn't exist before: removed");
        assert!(std::fs::read_to_string(home.join(".config/nexa/idle.conf")).unwrap().contains("lock_minutes=5"));
        // The going back is a moment too, and can itself be undone.
        let l = r.list(10);
        assert_eq!(l[0]["rewind_of"], first);
        let undo = l[0]["id"].as_u64().unwrap();
        r.rewind_to(undo, 700).unwrap();
        assert!(std::fs::read_to_string(home.join(".config/nexa/theme.json")).unwrap().contains("aurora"));
        assert!(home.join(".config/nexa/topbar").exists());
    }

    #[test]
    fn history_survives_a_restart_and_stays_bounded() {
        let (home, store) = setup();
        let mut r = Rewind::new(home.clone(), &store);
        for i in 0..130u64 {
            put(&home, ".config/nexa/idle.conf", &format!("lock_minutes={i}\n"));
            r.scan(i * 100);
        }
        assert_eq!(r.list(1000).len(), KEEP);
        let mut again = Rewind::new(home.clone(), &store);
        assert_eq!(again.list(1)[0]["changes"][0]["summary"][0], "lock_minutes: 128 \u{2192} 129");
        assert!(again.rewind_to(1, 99999).is_err(), "too old: said so");
    }

    #[test]
    fn only_settings_files_and_not_security_ones() {
        assert!(TRACKED.iter().all(|t| !t.0.contains("permission") && !t.0.contains("login") && !t.0.contains("security")));
        let (home, store) = setup();
        let mut r = Rewind::new(home.clone(), &store);
        put(&home, ".config/nexa/app-permissions.json", r#"{"x": {"camera": false}}"#);
        put(&home, "Documents/letter.txt", "hello");
        assert!(!r.scan(1), "neither is tracked");
        // A huge file isn't kept.
        put(&home, ".config/nexa/field.json", &"x".repeat(100_000));
        assert!(!r.scan(2));
    }

    #[test]
    fn summaries_read_like_people_talk() {
        assert_eq!(summary(Some("autohide\n"), Some("always\n")), vec!["autohide \u{2192} always"]);
        assert_eq!(summary(Some(r#"{"a": {"b": 1}}"#), Some(r#"{"a": {"b": 2}}"#)), vec!["a.b: 1 \u{2192} 2"]);
        assert_eq!(summary(Some("{}"), Some("{ }")), vec!["reformatted, same values"]);
        let many = summary(Some("a=1\nb=1\nc=1\nd=1\ne=1\nf=1"), Some("a=2\nb=2\nc=2\nd=2\ne=2\nf=2"));
        assert_eq!(many.len(), 5);
        assert_eq!(many[4], "and 2 more");
    }
}
