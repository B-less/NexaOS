//! Constellation's graph: which project each open window belongs to.
//!
//! A window's clues, each with a weight (TODOnexa_identity.md, section
//! "Constellation"):
//!
//! | clue | weight |
//! |---|---|
//! | a path in the title (`~/…`, `/home/…`) | 4 |
//! | a file name in the title naming exactly one recent or FileTrail file | 3 |
//! | a file its processes hold open | 2 each, 3 files at most |
//! | the working folder of a child process (a terminal's shell) | 2 |
//! | its own working folder | 1 |
//!
//! Each clue's folder becomes a project root ([`root_of`]); the root with
//! the highest total is the window's project. Only the user's own processes
//! are looked at; nothing is written.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub const MAX_WINDOWS: usize = 128;
const MAX_TITLE: usize = 512;
const MAX_FDS: usize = 256;
const MAX_TREE: usize = 64;
const MAX_KNOWN: usize = 2000;
const FILES_COUNTED: usize = 3;

/// Folders whose presence makes a folder a project.
const MARKERS: &[&str] = &[".git", "Cargo.toml", "package.json", "pyproject.toml", "go.mod", ".nexa-project"];
/// Home's standard folders: a project only one level inside them.
const STANDARD: &[&str] = &["Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos", "Films", "Public", "Templates"];

#[derive(Debug, Clone, PartialEq)]
pub struct Win {
    pub id: String,
    pub pid: i32,
    pub app: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub id: String,
    pub project: Option<PathBuf>,
    pub why: &'static str,
}

/// Where to look: `/proc`, the home folder, whose processes count, and
/// files known by name (recent files, FileTrail).
pub struct Env {
    pub proc_root: PathBuf,
    pub home: PathBuf,
    pub uid: u32,
    pub known: Vec<PathBuf>,
}

/// "id\tpid\tapp\ttitle" (the C side replaces tabs and line breaks in a
/// title with spaces). None for a line that isn't one.
pub fn parse_line(line: &str) -> Option<Win> {
    let mut f = line.trim_end_matches(['\n', '\r']).splitn(4, '\t');
    let id = f.next()?.trim();
    let pid: i32 = f.next()?.trim().parse().ok()?;
    let app = f.next()?.trim();
    let title = f.next().unwrap_or("");
    if id.is_empty() || id.len() > 32 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) || pid <= 0 {
        return None;
    }
    let mut t: String = title.chars().filter(|c| !c.is_control()).collect();
    if t.len() > MAX_TITLE {
        let mut cut = MAX_TITLE;
        while !t.is_char_boundary(cut) {
            cut -= 1;
        }
        t.truncate(cut);
    }
    Some(Win { id: id.to_string(), pid, app: app.chars().take(128).collect(), title: t })
}

/// A folder's project root, or None. The folder must be inside home (not
/// home itself) and not hidden. The nearest folder at or above it with a
/// marker wins; without one: two levels under home, or one level for a
/// folder that isn't a standard one.
pub fn root_of(folder: &Path, home: &Path) -> Option<PathBuf> {
    let rel = folder.strip_prefix(home).ok()?;
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if parts.is_empty() || parts.iter().any(|p| p.starts_with('.') || p == "..") {
        return None;
    }
    let mut at = folder.to_path_buf();
    while at != home && at.starts_with(home) {
        if MARKERS.iter().any(|m| at.join(m).exists()) {
            return Some(at);
        }
        if !at.pop() {
            break;
        }
    }
    if parts.len() >= 2 {
        Some(home.join(&parts[0]).join(&parts[1]))
    } else if STANDARD.contains(&parts[0].as_str()) {
        None
    } else {
        Some(home.join(&parts[0]))
    }
}

/// The folder a path stands for: itself if a folder, else its parent.
fn folder_of(p: &Path) -> Option<PathBuf> {
    let m = fs::metadata(p).ok()?;
    if m.is_dir() {
        Some(p.to_path_buf())
    } else if m.is_file() {
        p.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// Paths written in a title: "~/x/y" or "/home/u/x", up to a space before
/// a separator (" — ", " - ", " | ", " · ") or a quote/bracket. A path that
/// doesn't exist is shortened until it does.
pub fn title_paths(title: &str, home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let h = home.to_string_lossy();
    let b = title.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let rest = &title[i..];
        let start_ok = i == 0 || matches!(b[i - 1], b' ' | b'(' | b'[' | b'"' | b'\'' | b':');
        let (is_path, skip) = if rest.starts_with("~/") {
            (true, 1)
        } else if rest.starts_with(&*h) && rest[h.len()..].starts_with('/') {
            (true, h.len())
        } else {
            (false, 0)
        };
        if !start_ok || !is_path {
            i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            continue;
        }
        let tail = &rest[skip..];
        let mut end = tail.len();
        for sep in [" — ", " – ", " - ", " | ", " · ", "\"", "'", ")", "]"] {
            if let Some(k) = tail.find(sep) {
                end = end.min(k);
            }
        }
        let text = tail[..end].trim_end().trim_start_matches('/');
        // Paths may hold spaces ("My Thesis"): the longest piece, cut at a
        // space, that exists; else the longest folder that does.
        let mut cuts: Vec<usize> = text.match_indices(' ').map(|(k, _)| k).collect();
        cuts.push(text.len());
        let whole = cuts.iter().rev().map(|&k| home.join(text[..k].trim_end())).find(|p| p.exists() && p != home);
        if let Some(p) = whole {
            out.push(p);
            i += skip + end.max(1);
            continue;
        }
        let mut p = home.join(text);
        loop {
            if p.exists() {
                if p != home {
                    out.push(p.clone());
                }
                break;
            }
            if !p.pop() || !p.starts_with(home) {
                break;
            }
        }
        i += skip + end.max(1);
    }
    out
}

/// Whether `name` appears in `title` as a whole word (not inside a longer
/// name: "a.txt" is not in "data.txt").
pub fn names_in(title: &str, name: &str) -> bool {
    if name.chars().count() < 3 || !(name.contains('.') || name.chars().count() >= 5) {
        return false;
    }
    let part = |c: char| c.is_alphanumeric() || matches!(c, '.' | '_' | '-');
    let mut from = 0;
    while let Some(k) = title[from..].find(name) {
        let s = from + k;
        let e = s + name.len();
        let before = title[..s].chars().next_back();
        let after = title[e..].chars().next();
        // "notes.txt." at the end of a sentence still counts
        let after_ok = match after {
            None => true,
            Some('.') => title[e + 1..].chars().next().map_or(true, |c| !part(c)),
            Some(c) => !part(c),
        };
        if before.map_or(true, |c| !part(c)) && after_ok {
            return true;
        }
        from = s + name.len();
    }
    false
}

/// GTK's recent files (~/.local/share/recently-used.xbel): local paths.
pub fn recent_files(home: &Path) -> Vec<PathBuf> {
    let p = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("recently-used.xbel");
    let Ok(text) = fs::read_to_string(&p) else { return Vec::new() };
    let mut out = Vec::new();
    for chunk in text.split("href=\"file://").skip(1) {
        let Some(end) = chunk.find('"') else { continue };
        if let Some(path) = percent_decode(&chunk[..end]) {
            out.push(PathBuf::from(path));
        }
        if out.len() >= MAX_KNOWN {
            break;
        }
    }
    out
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else if b[i] == b'&' {
            // XML entities in an attribute
            let rest = &s[i..];
            let (ch, n) = if rest.starts_with("&amp;") {
                (b'&', 5)
            } else if rest.starts_with("&apos;") {
                (b'\'', 6)
            } else if rest.starts_with("&quot;") {
                (b'"', 6)
            } else if rest.starts_with("&lt;") {
                (b'<', 4)
            } else if rest.starts_with("&gt;") {
                (b'>', 4)
            } else {
                (b'&', 1)
            };
            out.push(ch);
            i += n;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    let s = String::from_utf8(out).ok()?;
    s.starts_with('/').then_some(s)
}

/// Files FileTrail followed to where they are now.
pub fn trail_files(home: &Path) -> Vec<PathBuf> {
    nexa_sys::filetrail::trail(home, "", MAX_KNOWN)
        .into_iter()
        .filter(|v| v["status"].as_str() == Some("here"))
        .filter_map(|v| v["now"].as_str().map(PathBuf::from))
        .collect()
}

/// Each process's parent, from <proc>/<pid>/stat.
fn parents(proc_root: &Path) -> HashMap<i32, i32> {
    let mut out = HashMap::new();
    let Ok(rd) = fs::read_dir(proc_root) else { return out };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
        let Ok(stat) = fs::read_to_string(e.path().join("stat")) else { continue };
        // "pid (comm) S ppid ..." — comm may hold spaces and brackets
        let Some(close) = stat.rfind(')') else { continue };
        let mut f = stat[close + 1..].split_whitespace();
        f.next();
        if let Some(ppid) = f.next().and_then(|s| s.parse().ok()) {
            out.insert(pid, ppid);
        }
    }
    out
}

/// A window's process and its descendants (the window's own first), the
/// user's own only.
fn tree(env: &Env, pid: i32, kids: &HashMap<i32, Vec<i32>>) -> Vec<i32> {
    let mut out = Vec::new();
    let mut todo = vec![pid];
    let mut seen = HashSet::new();
    while let Some(p) = todo.pop() {
        if out.len() >= MAX_TREE || !seen.insert(p) {
            continue;
        }
        match fs::metadata(env.proc_root.join(p.to_string())) {
            Ok(m) if m.uid() == env.uid => out.push(p),
            _ => continue,
        }
        if let Some(k) = kids.get(&p) {
            todo.extend(k.iter().rev());
        }
    }
    out
}

fn open_files(env: &Env, pid: i32) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(env.proc_root.join(pid.to_string()).join("fd")) else { return out };
    for e in rd.flatten().take(MAX_FDS) {
        if let Ok(t) = fs::read_link(e.path()) {
            if t.is_absolute() && t.starts_with(&env.home) && fs::metadata(&t).map(|m| m.is_file()).unwrap_or(false) {
                out.push(t);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn cwd(env: &Env, pid: i32) -> Option<PathBuf> {
    fs::read_link(env.proc_root.join(pid.to_string()).join("cwd")).ok()
}

/// What the windows' processes show in /proc: per window pid, its tree
/// (the window's process first), and each process's working folder and
/// open files. Read before confining: Landlock keeps a confined process
/// from reading other processes' cwd and fd links (as it would debugging
/// them).
pub struct Procs {
    trees: HashMap<i32, Vec<i32>>,
    cwd: HashMap<i32, PathBuf>,
    files: HashMap<i32, Vec<PathBuf>>,
}

pub fn snapshot(env: &Env, wins: &[Win]) -> Procs {
    let par = parents(&env.proc_root);
    let mut kids: HashMap<i32, Vec<i32>> = HashMap::new();
    for (&p, &pp) in &par {
        kids.entry(pp).or_default().push(p);
    }
    for v in kids.values_mut() {
        v.sort();
    }
    let mut out = Procs { trees: HashMap::new(), cwd: HashMap::new(), files: HashMap::new() };
    for w in wins.iter().take(MAX_WINDOWS) {
        if out.trees.contains_key(&w.pid) {
            continue;
        }
        let t = tree(env, w.pid, &kids);
        for &p in &t {
            if !out.cwd.contains_key(&p) {
                if let Some(c) = cwd(env, p) {
                    out.cwd.insert(p, c);
                }
                out.files.insert(p, open_files(env, p));
            }
        }
        out.trees.insert(w.pid, t);
    }
    out
}

/// Every window's project (reads /proc itself: for tests and callers that
/// aren't confined).
pub fn find(env: &Env, wins: &[Win]) -> Vec<Found> {
    find_in(env, &snapshot(env, wins), wins)
}

/// Every window's project, from a /proc snapshot.
pub fn find_in(env: &Env, procs: &Procs, wins: &[Win]) -> Vec<Found> {
    // file name -> the project roots of the known files with that name
    let mut by_name: HashMap<String, HashSet<PathBuf>> = HashMap::new();
    for k in env.known.iter().take(MAX_KNOWN) {
        let (Some(name), Some(parent)) = (k.file_name(), k.parent()) else { continue };
        if let Some(r) = root_of(parent, &env.home) {
            by_name.entry(name.to_string_lossy().into_owned()).or_default().insert(r);
        }
    }
    wins.iter()
        .take(MAX_WINDOWS)
        .map(|w| {
            // root -> (score, the strongest clue's name, its weight)
            let mut score: BTreeMap<PathBuf, (u32, &'static str, u32)> = BTreeMap::new();
            let mut add = |folder: Option<PathBuf>, weight: u32, why: &'static str| {
                if let Some(r) = folder.and_then(|f| root_of(&f, &env.home)) {
                    let e = score.entry(r).or_insert((0, why, 0));
                    e.0 += weight;
                    if weight > e.2 {
                        e.1 = why;
                        e.2 = weight;
                    }
                }
            };
            for p in title_paths(&w.title, &env.home) {
                add(folder_of(&p), 4, "title");
            }
            for (name, roots) in &by_name {
                if roots.len() == 1 && names_in(&w.title, name) {
                    add(roots.iter().next().cloned(), 3, "recent");
                }
            }
            let none = Vec::new();
            let tree = procs.trees.get(&w.pid).unwrap_or(&none);
            let mut files: Vec<PathBuf> = tree.iter().flat_map(|p| procs.files.get(p).cloned().unwrap_or_default()).collect();
            files.sort();
            files.dedup();
            for f in files.iter().take(FILES_COUNTED) {
                add(f.parent().map(Path::to_path_buf), 2, "files");
            }
            let mut child_dirs: Vec<PathBuf> = tree.iter().skip(1).filter_map(|p| procs.cwd.get(p).cloned()).collect();
            child_dirs.sort();
            child_dirs.dedup();
            for d in child_dirs {
                add(Some(d), 2, "terminal");
            }
            if let Some(own) = tree.first() {
                add(procs.cwd.get(own).cloned(), 1, "folder");
            }
            // highest score; a tie goes to the deeper (more specific), then the first by name
            let best = score.into_iter().max_by(|a, b| {
                (a.1 .0, a.0.components().count()).cmp(&(b.1 .0, b.0.components().count())).then_with(|| b.0.cmp(&a.0))
            });
            match best {
                Some((root, (_, why, _))) => Found { id: w.id.clone(), project: Some(root), why },
                None => Found { id: w.id.clone(), project: None, why: "" },
            }
        })
        .collect()
}

/// The answer: one JSON line.
pub fn to_json(found: &[Found], home: &Path) -> String {
    let wins: Vec<serde_json::Value> = found
        .iter()
        .map(|f| match &f.project {
            Some(p) => serde_json::json!({
                "id": f.id,
                "project": p.to_string_lossy(),
                "name": p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                "place": p.strip_prefix(home).map(|r| format!("~/{}", r.to_string_lossy())).unwrap_or_default(),
                "why": f.why,
            }),
            None => serde_json::json!({"id": f.id, "project": null}),
        })
        .collect();
    serde_json::json!({ "windows": wins }).to_string()
}
