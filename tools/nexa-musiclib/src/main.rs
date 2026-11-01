//! nexa-musiclib: the Music app's library. One request per run, as the user:
//!
//!   echo '{"op": "scan"}' | nexa-musiclib
//!
//! Ops:
//! - scan (dirs: optional, default ~/Music): every song under them with its
//!   tags, length and cover. Files unchanged since the last scan come from
//!   the index ($XDG_CACHE_HOME/nexa-music/library.json); covers are saved
//!   once per album in .../covers, or a cover.jpg/folder.jpg next to the
//!   songs is used. Prints {"progress": {"done", "total"}} while it reads.
//! - tags (paths): the same for some files (opened from Files).
//! - playlists; playlist_get (name); playlist_put (name, paths);
//!   playlist_delete (name); playlist_rename (name, to): .m3u files in
//!   $XDG_DATA_HOME/nexa-music/playlists. read_m3u (path): any .m3u.
//!
//! Reply: {"ok": true, "result": ...} or {"ok": false, "error": "..."}.
//! It confines itself with Landlock to the music folders (read) and its own
//! cache or playlists folder (write).

mod tags;

use nexa_sys::sandbox;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

const SONGS: [&str; 10] = ["mp3", "flac", "ogg", "oga", "opus", "m4a", "mp4", "aac", "wav", "wave"];
const MAX_FILES: usize = 200_000;
const INDEX_VERSION: u64 = 1;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into())
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
}

fn cache_dir() -> PathBuf { xdg("XDG_CACHE_HOME", ".cache").join("nexa-music") }
fn playlists_dir() -> PathBuf { xdg("XDG_DATA_HOME", ".local/share").join("nexa-music/playlists") }

fn confine(read: &[&Path], write: &[&Path]) -> Result<(), String> {
    let mut rules: Vec<(String, u64)> = read.iter().map(|p| (p.to_string_lossy().into_owned(), sandbox::READ)).collect();
    rules.extend(write.iter().map(|p| (p.to_string_lossy().into_owned(), sandbox::READ_WRITE)));
    sandbox::restrict(&sandbox::Rules { fs: rules, connect_ports: vec![], any_network: false })
        .map(|_| ())
        .map_err(|e| format!("couldn't confine the music engine: {e}"))
}

fn abs(v: &Value) -> Result<PathBuf, String> {
    let s = v.as_str().ok_or("paths must be text")?;
    let p = Path::new(s);
    if !p.is_absolute() || s.contains('\0') || p.components().any(|c| c == Component::ParentDir) {
        return Err(format!("not a full path: {s}"));
    }
    Ok(p.components().collect())
}

fn write_atomic(target: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = target.with_extension(format!("tmp{}", std::process::id()));
    let mut f = fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(&tmp, target)
}

/// FNV-1a: a short stable name for an album's cover file.
fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn is_song(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map_or(false, |e| SONGS.contains(&e.to_ascii_lowercase().as_str()))
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, seen: &mut HashSet<(u64, u64)>) {
    if depth > 16 || out.len() >= MAX_FILES {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<fs::DirEntry> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(t) = e.file_type() else { continue };
        let p = e.path();
        if t.is_dir() {
            // A folder reached twice (bind mounts): once is enough.
            if let Ok(m) = e.metadata() {
                if !seen.insert((m.dev(), m.ino())) {
                    continue;
                }
            }
            walk(&p, depth + 1, out, seen);
        } else if t.is_file() && is_song(&p) {
            out.push(p);
        }
    }
}

/// A cover image sitting next to the songs (cover.jpg, folder.png, ...).
fn folder_cover(dir: &Path, cache: &mut HashMap<PathBuf, Option<PathBuf>>) -> Option<PathBuf> {
    cache.entry(dir.to_path_buf()).or_insert_with(|| {
        let names = ["cover", "folder", "front", "album", "albumart"];
        let mut found: Vec<PathBuf> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            names.contains(&stem.as_str()) && matches!(ext.as_str(), "jpg" | "jpeg" | "png")
        }).collect();
        found.sort_by_key(|p| names.iter().position(|n| p.file_stem().and_then(|s| s.to_str()).map_or(false, |s| s.eq_ignore_ascii_case(n))).unwrap_or(9));
        found.into_iter().next()
    }).clone()
}

fn album_key(album_artist: &str, album: &str) -> String {
    format!("{}\0{}", album_artist.to_lowercase(), album.to_lowercase())
}

/// One song as the app sees it, the cover saved if it has one.
fn track(path: &Path, covers: &Path) -> Value {
    let t = tags::read(path).unwrap_or_default();
    let stem = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let folder = path.parent().and_then(|p| p.file_name()).map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let artist = t.artist.clone().unwrap_or_default();
    let album = t.album.clone().unwrap_or_default();
    let album_artist = t.album_artist.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| artist.clone());
    let mut cover = Value::Null;
    if let Some((mime, data)) = &t.picture {
        let ext = if mime == "image/png" { "png" } else { "jpg" };
        let name = if album.is_empty() { fnv(&path.to_string_lossy()) } else { fnv(&album_key(&album_artist, &album)) };
        let file = covers.join(format!("{name}.{ext}"));
        if file.exists() || write_atomic(&file, data).is_ok() {
            cover = json!(file.to_string_lossy());
        }
    }
    json!({
        "path": path.to_string_lossy(),
        "title": t.title.unwrap_or_else(|| stem.clone()),
        "artist": artist, "album": album, "album_artist": album_artist,
        "genre": t.genre, "year": t.year, "track": t.track, "disc": t.disc,
        "duration": t.duration.map(|d| (d * 10.0).round() / 10.0),
        "cover": cover, "folder": folder,
    })
}

fn scan(req: &Value) -> Result<Value, String> {
    let dirs: Vec<PathBuf> = match req.get("dirs").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a.iter().map(abs).collect::<Result<_, _>>()?,
        _ => vec![home().join("Music")],
    };
    let dirs: Vec<PathBuf> = dirs.into_iter().filter(|d| d.is_dir()).collect();
    let cache = cache_dir();
    let covers = cache.join("covers");
    fs::create_dir_all(&covers).map_err(|e| format!("can't make the music cache: {e}"))?;
    let read: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
    confine(&read, &[&cache])?;
    let index_path = cache.join("library.json");
    let old: Map<String, Value> = fs::read(&index_path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(|v| v["version"].as_u64() == Some(INDEX_VERSION))
        .and_then(|v| v.get("tracks").and_then(Value::as_object).cloned()).unwrap_or_default();
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for d in &dirs {
        walk(d, 0, &mut files, &mut seen);
    }
    let total = files.len();
    let mut out = Map::new();
    let mut folder_covers = HashMap::new();
    let mut fresh = 0;
    let mut stdout = std::io::stdout().lock();
    for (i, p) in files.iter().enumerate() {
        if i % 50 == 0 {
            let _ = writeln!(stdout, "{}", json!({"progress": {"done": i, "total": total}}));
            let _ = stdout.flush();
        }
        let Ok(m) = fs::metadata(p) else { continue };
        let stamp = json!([m.mtime(), m.mtime_nsec(), m.len()]);
        let key = p.to_string_lossy().into_owned();
        let cached = old.get(&key).filter(|e| e["stamp"] == stamp && e["track"]["cover"].as_str().map_or(true, |c| Path::new(c).exists()));
        let mut t = match cached {
            Some(e) => e["track"].clone(),
            None => {
                fresh += 1;
                track(p, &covers)
            }
        };
        if t["cover"].is_null() {
            if let Some(c) = p.parent().and_then(|d| folder_cover(d, &mut folder_covers)) {
                t["cover"] = json!(c.to_string_lossy());
            }
        }
        out.insert(key, json!({"stamp": stamp, "track": t}));
    }
    // Albums share a cover: a track without one gets its album's.
    let mut album_cover: HashMap<String, Value> = HashMap::new();
    for e in out.values() {
        let t = &e["track"];
        if !t["cover"].is_null() && !t["album"].as_str().unwrap_or("").is_empty() {
            album_cover.entry(album_key(t["album_artist"].as_str().unwrap_or(""), t["album"].as_str().unwrap_or(""))).or_insert(t["cover"].clone());
        }
    }
    let mut tracks: Vec<Value> = out.values().map(|e| {
        let mut t = e["track"].clone();
        if t["cover"].is_null() {
            if let Some(c) = album_cover.get(&album_key(t["album_artist"].as_str().unwrap_or(""), t["album"].as_str().unwrap_or(""))) {
                t["cover"] = c.clone();
            }
        }
        t
    }).collect();
    tracks.sort_by(|a, b| {
        let k = |t: &Value| (t["album_artist"].as_str().unwrap_or("").to_lowercase(), t["album"].as_str().unwrap_or("").to_lowercase(),
                             t["disc"].as_u64().unwrap_or(1), t["track"].as_u64().unwrap_or(0), t["path"].as_str().unwrap_or("").to_string());
        k(a).cmp(&k(b))
    });
    let _ = write_atomic(&index_path, json!({"version": INDEX_VERSION, "tracks": out}).to_string().as_bytes());
    Ok(json!({"tracks": tracks, "read": fresh, "dirs": dirs}))
}

fn tags_of(req: &Value) -> Result<Value, String> {
    let paths: Vec<PathBuf> = req.get("paths").and_then(Value::as_array).ok_or("missing 'paths'")?.iter().map(abs).collect::<Result<_, _>>()?;
    let covers = cache_dir().join("covers");
    fs::create_dir_all(&covers).map_err(|e| format!("can't make the music cache: {e}"))?;
    let folders: Vec<PathBuf> = paths.iter().filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
    let read: Vec<&Path> = folders.iter().map(PathBuf::as_path).collect();
    let cache = cache_dir();
    confine(&read, &[&cache])?;
    let mut fc = HashMap::new();
    Ok(Value::Array(paths.iter().filter(|p| p.is_file()).map(|p| {
        let mut t = track(p, &covers);
        if t["cover"].is_null() {
            if let Some(c) = p.parent().and_then(|d| folder_cover(d, &mut fc)) {
                t["cover"] = json!(c.to_string_lossy());
            }
        }
        t
    }).collect()))
}

// ---------- playlists ----------

fn playlist_name(v: Option<&Value>) -> Result<String, String> {
    let n = v.and_then(Value::as_str).ok_or("missing playlist name")?.trim().to_string();
    if n.is_empty() || n.len() > 100 || n.starts_with('.') || n.contains(['/', '\0', '\n', '\r']) {
        return Err("A playlist name can't be empty, start with a dot, or have a / in it.".into());
    }
    Ok(n)
}

/// The songs an .m3u lists (relative names are from the playlist's folder).
fn parse_m3u(text: &str, dir: &Path) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| {
        let l = l.strip_prefix("file://").unwrap_or(l);
        let p = Path::new(l);
        let full = if p.is_absolute() { p.to_path_buf() } else { dir.join(p) };
        (!full.components().any(|c| c == Component::ParentDir) || p.is_absolute()).then(|| full.to_string_lossy().into_owned())
    }).collect()
}

fn playlists(op: &str, req: &Value) -> Result<Value, String> {
    let dir = playlists_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("can't make the playlists folder: {e}"))?;
    confine(&[], &[&dir])?;
    let file = |name: &str| dir.join(format!("{name}.m3u"));
    match op {
        "playlists" => {
            let mut out: Vec<Value> = fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten().filter_map(|e| {
                let p = e.path();
                (p.extension().and_then(|x| x.to_str()) == Some("m3u")).then(|| {
                    let n = parse_m3u(&fs::read_to_string(&p).unwrap_or_default(), &dir).len();
                    json!({"name": p.file_stem().unwrap_or_default().to_string_lossy(), "count": n})
                })
            }).collect();
            out.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_lowercase());
            Ok(Value::Array(out))
        }
        "playlist_get" => {
            let name = playlist_name(req.get("name"))?;
            let text = fs::read_to_string(file(&name)).map_err(|_| format!("There's no playlist called {name}."))?;
            Ok(json!(parse_m3u(&text, &dir)))
        }
        "playlist_put" => {
            let name = playlist_name(req.get("name"))?;
            let paths: Vec<PathBuf> = req.get("paths").and_then(Value::as_array).ok_or("missing 'paths'")?.iter().map(abs).collect::<Result<_, _>>()?;
            let mut text = String::from("#EXTM3U\n");
            for p in paths.iter().take(100_000) {
                text.push_str(&p.to_string_lossy());
                text.push('\n');
            }
            write_atomic(&file(&name), text.as_bytes()).map_err(|e| format!("The playlist wasn't saved: {e}"))?;
            Ok(json!({"name": name, "count": paths.len()}))
        }
        "playlist_delete" => {
            let name = playlist_name(req.get("name"))?;
            fs::remove_file(file(&name)).map_err(|_| format!("There's no playlist called {name}."))?;
            Ok(json!({"deleted": name}))
        }
        "playlist_rename" => {
            let (name, to) = (playlist_name(req.get("name"))?, playlist_name(req.get("to"))?);
            if file(&to).exists() {
                return Err(format!("There's already a playlist called {to}."));
            }
            fs::rename(file(&name), file(&to)).map_err(|_| format!("There's no playlist called {name}."))?;
            Ok(json!({"name": to}))
        }
        _ => unreachable!(),
    }
}

fn handle(input: &[u8]) -> Result<Value, String> {
    let req: Value = serde_json::from_slice(input).map_err(|e| format!("bad request: {e}"))?;
    match req.get("op").and_then(Value::as_str).ok_or("missing 'op'")? {
        "scan" => scan(&req),
        "tags" => tags_of(&req),
        op @ ("playlists" | "playlist_get" | "playlist_put" | "playlist_delete" | "playlist_rename") => playlists(op, &req),
        "read_m3u" => {
            let p = abs(req.get("path").ok_or("missing 'path'")?)?;
            let dir = p.parent().ok_or("no folder")?.to_path_buf();
            confine(&[&dir], &[])?;
            let mut text = String::new();
            fs::File::open(&p).and_then(|f| f.take(16 << 20).read_to_string(&mut text)).map_err(|e| format!("That playlist can't be read: {e}"))?;
            Ok(json!(parse_m3u(&text, &dir)))
        }
        other => Err(format!("unknown op {other}")),
    }
}

fn main() {
    let mut input = Vec::new();
    let _ = std::io::stdin().take(64 << 20).read_to_end(&mut input);
    let reply = match handle(&input) {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(error) => json!({"ok": false, "error": error}),
    };
    let _ = writeln!(std::io::stdout().lock(), "{reply}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn songs_and_covers() {
        let d = std::env::temp_dir().join(format!("nexa-musiclib-main-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let (music, covers) = (d.join("Music/Nexa Band/Hexagons"), d.join("covers"));
        fs::create_dir_all(&music).unwrap();
        fs::create_dir_all(&covers).unwrap();
        fs::write(music.join("01.mp3"), tags::tests::mp3("One", "Nexa Band", "Hexagons", "1")).unwrap();
        let t = track(&music.join("01.mp3"), &covers);
        assert_eq!((t["title"].as_str(), t["album_artist"].as_str(), t["track"].as_u64()), (Some("One"), Some("Nexa Band"), Some(1)));
        let c = t["cover"].as_str().unwrap();
        assert!(c.ends_with(".jpg") && fs::read(c).unwrap().starts_with(&[0xff, 0xd8]));
        // No tags at all: the file's name is the title.
        let mut wav = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x02\0\x44\xac\0\0\x10\xb1\x02\0\x04\0\x10\0data\x10\xb1\x02\0".to_vec();
        wav.resize(wav.len() + 176_400, 0); // one second of silence
        fs::write(music.join("untagged song.wav"), wav).unwrap();
        let t = track(&music.join("untagged song.wav"), &covers);
        assert_eq!((t["title"].as_str(), t["duration"].as_f64()), (Some("untagged song"), Some(1.0)));
        let mut files = vec![];
        walk(&d.join("Music"), 0, &mut files, &mut HashSet::new());
        assert_eq!(files.len(), 2);
        fs::write(music.join("Cover.JPG"), [0xff, 0xd8, 0, 0]).unwrap();
        assert_eq!(folder_cover(&music, &mut HashMap::new()), Some(music.join("Cover.JPG")));
        assert_eq!(parse_m3u("#EXTM3U\n#EXTINF:3,x\na.mp3\n/abs/b.flac\nfile:///c.ogg\n../up.mp3\n", Path::new("/p")),
                   vec!["/p/a.mp3".to_string(), "/abs/b.flac".into(), "/c.ogg".into()]);
        assert!(playlist_name(Some(&json!("a/b"))).is_err());
        assert!(playlist_name(Some(&json!(".hidden"))).is_err());
        assert_eq!(playlist_name(Some(&json!(" Road trip "))).unwrap(), "Road trip");
        fs::remove_dir_all(&d).unwrap();
    }
}
