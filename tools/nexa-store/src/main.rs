//! nexa-store: the Nexa Store's engine. Runs as you; the Store app asks it.
//!
//!   nexa-store refresh                 fetch every catalogue (and its icons)
//!   nexa-store list [--search Q] [--category C]
//!   nexa-store info ID
//!   nexa-store updates
//!   nexa-store download ID             fetch the package, check it, print its path
//!   nexa-store installed               installed apps (from any source), with updates
//!
//! Catalogues come from /etc/nexa/store.conf - local folders (file://) or
//! HTTPS only - and are signed (nexa-pack repo build). An index whose
//! signature doesn't match is refused; one without a trusted signature is
//! shown as "not verified". Every read re-checks the cached index, so a
//! changed cache is caught too. A download must match the size and SHA-256
//! the catalogue lists; installing then goes through the system broker,
//! which checks the package's own signature again.
//!
//! The process confines itself with Landlock: it writes only its cache, and
//! reaches the network only on port 443 (and not at all for local
//! catalogues). Output: progress lines {"progress": {"done", "total"}},
//! then one {"ok": true, "result": ...} or {"ok": false, "error": "..."}.

use nexa_pack::package::Trust;
use nexa_pack::repo::{self, Entry, Index};
use nexa_sys::sandbox;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_INDEX: u64 = 8 << 20;
const MAX_ICON: u64 = 1 << 20;
const MAX_SHOT: u64 = 3 << 20;

#[derive(Deserialize, Clone)]
struct Catalogue {
    name: String,
    url: String,
}

#[derive(Deserialize, Default)]
struct Conf {
    #[serde(default)]
    catalogues: Vec<Catalogue>,
}

fn env_path(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| default.into())
}

fn cache_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return x.join("nexa-store");
    }
    env_path("HOME", "/").join(".cache/nexa-store")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn catalogue_dir(url: &str) -> PathBuf {
    cache_dir().join("catalogues").join(&hex(&Sha256::digest(url.as_bytes()))[..16])
}

/// Where a catalogue's files come from.
#[derive(Clone)]
enum Source {
    Local(PathBuf),
    Https(String),
}

fn source(url: &str) -> Result<Source, String> {
    if let Some(p) = url.strip_prefix("file://") {
        let p = PathBuf::from(p);
        if !p.is_absolute() {
            return Err(format!("{url}: not a full path"));
        }
        Ok(Source::Local(p))
    } else if url.starts_with("https://") && !url.contains(char::is_whitespace) {
        Ok(Source::Https(url.trim_end_matches('/').to_string()))
    } else {
        Err(format!("{url}: catalogues must be a local folder (file://) or HTTPS"))
    }
}

/// A file inside a catalogue (the path is already checked to stay inside).
fn join(src: &Source, rel: &str) -> Source {
    match src {
        Source::Local(p) => Source::Local(p.join(rel)),
        Source::Https(u) => Source::Https(format!("{u}/{rel}")),
    }
}

fn progress(done: u64, total: u64) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", json!({"progress": {"done": done, "total": total}}));
    let _ = out.flush();
}

/// Fetch into `dest` (via a .part file), at most `max` bytes.
fn fetch(src: &Source, dest: &Path, max: u64, show: Option<u64>) -> Result<(), String> {
    let part = dest.with_extension("part");
    let _ = fs::remove_file(&part);
    match src {
        Source::Local(p) => {
            let mut f = File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
            if f.metadata().map(|m| m.len()).unwrap_or(0) > max {
                return Err(format!("{} is larger than the catalogue says", p.display()));
            }
            let mut out = File::create(&part).map_err(|e| e.to_string())?;
            let (mut buf, mut done, mut last) = (vec![0u8; 1 << 20], 0u64, Instant::now());
            loop {
                let n = f.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                done += n as u64;
                if done > max {
                    return Err("larger than the catalogue says".into());
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                if let Some(total) = show {
                    if last.elapsed() > Duration::from_millis(150) {
                        progress(done, total);
                        last = Instant::now();
                    }
                }
            }
        }
        Source::Https(url) => {
            let mut child = Command::new("curl")
                .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "2", "--max-time", "900",
                       "--max-filesize", &max.to_string(), "-o"])
                .arg(&part).arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                .spawn().map_err(|e| format!("can't run curl: {e}"))?;
            let status = loop {
                if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
                    break s;
                }
                if let Some(total) = show {
                    progress(fs::metadata(&part).map(|m| m.len()).unwrap_or(0), total);
                }
                std::thread::sleep(Duration::from_millis(200));
            };
            if !status.success() {
                let _ = fs::remove_file(&part);
                return Err(format!("couldn't download {url} (no connection, or the server refused)"));
            }
        }
    }
    if fs::metadata(&part).map(|m| m.len()).unwrap_or(u64::MAX) > max {
        let _ = fs::remove_file(&part);
        return Err("larger than the catalogue says".into());
    }
    fs::rename(&part, dest).map_err(|e| e.to_string())
}

struct Loaded {
    cat: Catalogue,
    src: Source,
    dir: PathBuf,
    index: Index,
    trust: Trust,
}

/// The cached catalogues, each re-verified now.
fn load(conf: &Conf, keys: &Path) -> Vec<Loaded> {
    conf.catalogues.iter().filter_map(|cat| {
        let dir = catalogue_dir(&cat.url);
        let bytes = fs::read(dir.join("index.json")).ok()?;
        let sig = fs::read_to_string(dir.join("index.sig")).ok();
        let (index, trust) = repo::read(&bytes, sig.as_deref(), keys.is_dir().then_some(keys)).ok()?;
        Some(Loaded { cat: cat.clone(), src: source(&cat.url).ok()?, dir, index, trust })
    }).collect()
}

fn installed_version(registry: &Path, id: &str) -> Option<String> {
    let v: Value = serde_json::from_slice(&fs::read(registry.join(format!("{id}.json"))).ok()?).ok()?;
    v["version"].as_str().map(str::to_string)
}

/// Every app once: from a verified catalogue first, then the newest version.
fn merged(loaded: &[Loaded]) -> BTreeMap<String, (&Loaded, &Entry)> {
    let mut apps: BTreeMap<String, (&Loaded, &Entry)> = BTreeMap::new();
    for l in loaded {
        for e in &l.index.apps {
            let better = match apps.get(&e.id) {
                None => true,
                Some((old, oe)) => {
                    let (a, b) = (matches!(l.trust, Trust::Trusted(_)), matches!(old.trust, Trust::Trusted(_)));
                    (a && !b) || (a == b && repo::newer(&e.version, &oe.version))
                }
            };
            if better {
                apps.insert(e.id.clone(), (l, e));
            }
        }
    }
    apps
}

fn describe(l: &Loaded, e: &Entry, registry: &Path) -> Value {
    let installed = installed_version(registry, &e.id);
    let state = match &installed {
        None => "available",
        Some(v) if repo::newer(&e.version, v) => "update",
        Some(_) => "installed",
    };
    let icon = e.icon.as_ref().map(|i| l.dir.join(i)).filter(|p| p.is_file());
    let shots: Vec<PathBuf> = e.screenshots.iter().map(|s| l.dir.join(s)).filter(|p| p.is_file()).collect();
    json!({
        "id": e.id, "name": e.name, "version": e.version, "description": e.description,
        "categories": e.categories, "permissions": e.permissions, "depends": e.depends, "terminal": e.terminal,
        "publisher": e.publisher, "size": e.size, "catalogue": l.cat.name,
        "verified": matches!(l.trust, Trust::Trusted(_)), "trust": l.trust.label(),
        "icon": icon, "screenshots": shots, "installed": installed, "state": state,
    })
}

/// One line of a .desktop file's [Desktop Entry] ("Name", "Comment").
fn desktop_field(text: &str, key: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry {
            if let Some(v) = line.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
                return Some(v.trim().chars().take(200).collect());
            }
        }
    }
    None
}

/// Every app the broker installed (its registry), whether or not a
/// catalogue has it: name, description and icon from its own menu entry,
/// and the catalogue's newer version when there is one.
fn installed(loaded: &[Loaded], registry: &Path) -> Vec<Value> {
    let apps = merged(loaded);
    let mut out: Vec<Value> = fs::read_dir(registry).into_iter().flatten().flatten().filter_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        let id = name.strip_suffix(".json")?.to_string();
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return None;
        }
        let meta = fs::metadata(e.path()).ok()?;
        if !meta.is_file() || meta.len() > 1 << 20 {
            return None;
        }
        let rec: Value = serde_json::from_slice(&fs::read(e.path()).ok()?).ok()?;
        let paths: Vec<PathBuf> = rec["paths"].as_array().into_iter().flatten().filter_map(|p| p.as_str()).map(PathBuf::from).collect();
        let desktop = paths.iter().find(|p| p.extension().is_some_and(|x| x == "desktop"))
            .and_then(|p| fs::read(p).ok().filter(|b| b.len() < 64 << 10)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let icon = paths.iter().find(|p| p.extension().is_some_and(|x| x == "svg" || x == "png")).filter(|p| p.is_file());
        let version = rec["version"].as_str().unwrap_or("").to_string();
        let mut v = json!({
            "id": id, "name": desktop_field(&desktop, "Name").unwrap_or_else(|| id.clone()),
            "description": desktop_field(&desktop, "Comment").unwrap_or_default(),
            "version": version, "installed": version, "state": "installed", "icon": icon,
            "permissions": rec["permissions"], "verified": rec["trust"] == "trusted",
            "publisher": rec["publisher"], "catalogue": Value::Null,
        });
        if let Some((l, entry)) = apps.get(&id) {
            let c = describe(l, entry, registry);
            for k in ["name", "description", "categories", "size", "depends", "catalogue", "state", "screenshots"] {
                v[k] = c[k].clone();
            }
            v["version"] = c["version"].clone();
            if v["icon"].is_null() {
                v["icon"] = c["icon"].clone();
            }
        }
        Some(v)
    }).collect();
    out.sort_by_key(|a| a["name"].as_str().unwrap_or("").to_lowercase());
    out
}

fn refresh(conf: &Conf, keys: &Path) -> Value {
    let mut out = Vec::new();
    for cat in &conf.catalogues {
        let result = (|| -> Result<(usize, Trust), String> {
            let src = source(&cat.url)?;
            let dir = catalogue_dir(&cat.url);
            let tmp = dir.with_extension("new");
            let _ = fs::remove_dir_all(&tmp);
            fs::create_dir_all(tmp.join("icons")).map_err(|e| e.to_string())?;
            fs::create_dir_all(tmp.join("screenshots")).map_err(|e| e.to_string())?;
            fetch(&join(&src, "index.json"), &tmp.join("index.json"), MAX_INDEX, None)?;
            let _ = fetch(&join(&src, "index.sig"), &tmp.join("index.sig"), 4096, None); // unsigned: allowed, flagged
            let bytes = fs::read(tmp.join("index.json")).map_err(|e| e.to_string())?;
            let sig = fs::read_to_string(tmp.join("index.sig")).ok();
            let (index, trust) = repo::read(&bytes, sig.as_deref(), keys.is_dir().then_some(keys))?;
            for e in &index.apps {
                if let Some(icon) = &e.icon {
                    let _ = fetch(&join(&src, icon), &tmp.join(icon), MAX_ICON, None);
                }
                // Pictures of the app: one that doesn't come is left out.
                for s in &e.screenshots {
                    let _ = fetch(&join(&src, s), &tmp.join(s), MAX_SHOT, None);
                }
            }
            let _ = fs::remove_dir_all(&dir);
            fs::rename(&tmp, &dir).map_err(|e| e.to_string())?;
            Ok((index.apps.len(), trust))
        })();
        out.push(match result {
            Ok((n, t)) => json!({"name": cat.name, "url": cat.url, "ok": true, "apps": n, "trust": t.label(), "publisher": t.publisher()}),
            Err(e) => json!({"name": cat.name, "url": cat.url, "ok": false, "error": e}),
        });
    }
    json!({"catalogues": out})
}

fn download(conf: &Conf, keys: &Path, id: &str) -> Result<Value, String> {
    let loaded = load(conf, keys);
    let apps = merged(&loaded);
    let (l, e) = apps.get(id).ok_or_else(|| format!("{id} isn't in any catalogue (try refreshing)"))?;
    let dir = cache_dir().join("packages");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Old downloads of this app go.
    for old in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let n = old.file_name().to_string_lossy().into_owned();
        if n.starts_with(&format!("{id}-")) && n != format!("{id}-{}.nexa", e.version) {
            let _ = fs::remove_file(old.path());
        }
    }
    let dest = dir.join(format!("{id}-{}.nexa", e.version));
    let check = |p: &Path| -> bool {
        let Ok(mut f) = File::open(p) else { return false };
        let (mut h, mut buf, mut n) = (Sha256::new(), vec![0u8; 1 << 20], 0u64);
        while let Ok(k) = f.read(&mut buf) {
            if k == 0 {
                break;
            }
            n += k as u64;
            h.update(&buf[..k]);
        }
        n == e.size && hex(&h.finalize()) == e.sha256
    };
    if !check(&dest) {
        fetch(&join(&l.src, &e.package), &dest, e.size, Some(e.size))?;
        if !check(&dest) {
            let _ = fs::remove_file(&dest);
            return Err("the download doesn't match the catalogue - it may have been tampered with; nothing was installed".into());
        }
    }
    progress(e.size, e.size);
    Ok(json!({"path": dest, "id": id, "version": e.version, "verified": matches!(l.trust, Trust::Trusted(_))}))
}

fn confine(conf: &Conf, registry: &Path) -> Result<(), String> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let cache = cache_dir();
    fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let mut rules = vec![
        ("/usr".to_string(), sandbox::READ_EXEC), ("/lib".to_string(), sandbox::READ_EXEC),
        ("/etc".to_string(), sandbox::READ), ("/proc".to_string(), sandbox::READ),
        ("/dev/null".to_string(), sandbox::READ_WRITE), ("/dev/urandom".to_string(), sandbox::READ),
        (s(registry), sandbox::READ), (s(&cache), sandbox::READ_WRITE),
    ];
    // DNS settings are often a link out of /etc (systemd-resolved, VPNs, WSL:
    // /etc/resolv.conf -> /run/... or /mnt/...); Landlock follows the link,
    // so curl couldn't look any name up. Allow where it really is.
    for f in ["/etc/resolv.conf", "/etc/hosts", "/etc/ssl/certs/ca-certificates.crt"] {
        if let Ok(real) = fs::canonicalize(f) {
            if !real.starts_with("/etc") {
                rules.push((s(&real), sandbox::READ));
            }
        }
    }
    let mut https = false;
    for c in &conf.catalogues {
        match source(&c.url) {
            Ok(Source::Local(p)) => rules.push((s(&p), sandbox::READ)),
            Ok(Source::Https(_)) => https = true,
            Err(_) => {}
        }
    }
    if let Some(k) = std::env::var_os("NEXA_STORE_KEYS") {
        rules.push((k.to_string_lossy().into_owned(), sandbox::READ));
    }
    sandbox::restrict(&sandbox::Rules { fs: rules, connect_ports: if https { vec![443] } else { vec![] }, any_network: false })
        .map(|_| ()).map_err(|e| format!("couldn't confine myself: {e}"))
}

fn run(args: &[String]) -> Result<Value, String> {
    let conf_path = env_path("NEXA_STORE_CONF", "/etc/nexa/store.conf");
    let conf: Conf = match fs::read(&conf_path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", conf_path.display()))?,
        Err(_) => Conf::default(),
    };
    let keys = env_path("NEXA_STORE_KEYS", "/etc/nexa/keys");
    let registry = env_path("NEXA_STORE_REGISTRY", "/var/lib/nexa-packages");
    confine(&conf, &registry)?;
    let opt = |name: &str| args.windows(2).find(|w| w[0] == name).map(|w| w[1].to_lowercase());
    match args.first().map(String::as_str) {
        Some("refresh") => Ok(refresh(&conf, &keys)),
        Some("list") | Some("updates") => {
            let loaded = load(&conf, &keys);
            let (search, category) = (opt("--search"), opt("--category"));
            let apps: Vec<Value> = merged(&loaded).values().map(|(l, e)| describe(l, e, &registry)).filter(|a| {
                let text = format!("{} {} {}", a["id"], a["name"], a["description"]).to_lowercase();
                search.as_ref().map_or(true, |q| text.contains(q.as_str()))
                    && category.as_ref().map_or(true, |c| a["categories"].as_str().unwrap_or("").to_lowercase().contains(c.as_str()))
                    && (args[0] != "updates" || a["state"] == "update")
            }).collect();
            let fetched = loaded.iter().filter_map(|l| fs::metadata(l.dir.join("index.json")).ok()?.modified().ok())
                .min().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs());
            Ok(json!({"apps": apps, "catalogues": loaded.len(), "fetched": fetched,
                      "now": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)}))
        }
        Some("info") => {
            let id = args.get(1).ok_or("usage: nexa-store info ID")?;
            let loaded = load(&conf, &keys);
            merged(&loaded).get(id.as_str()).map(|(l, e)| describe(l, e, &registry)).ok_or_else(|| format!("{id} isn't in any catalogue"))
        }
        Some("download") => download(&conf, &keys, args.get(1).ok_or("usage: nexa-store download ID")?),
        Some("installed") => Ok(json!({"apps": installed(&load(&conf, &keys), &registry)})),
        _ => Err("usage: nexa-store refresh | list [--search Q] [--category C] | info ID | updates | download ID | installed".into()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reply = match run(&args) {
        Ok(v) => json!({"ok": true, "result": v}),
        Err(e) => json!({"ok": false, "error": e}),
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{reply}");
    std::process::exit(if reply["ok"] == true { 0 } else { 1 });
}
