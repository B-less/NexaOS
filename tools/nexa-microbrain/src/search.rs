//! ZeroSearch: plain-language commands without an AI model. Words are
//! normalised, matched against a vocabulary of intents with synonyms and a
//! little typo tolerance, and every result is one of a few typed actions
//! the desktop knows how to carry out (open a folder, launch an app,
//! brightness, volume, lock, power, a mode, a workspace, a settings page,
//! an answer). Anything else gets "I can't do that yet", never a guess.
//!
//! The tiny command model (nexa-intent, about 1 MB, built in) reads the
//! sentence first: which intent, how sure, and the name in it. What it is
//! sure of becomes the same typed actions; the word rules below still run
//! beside it, and the best of both is shown. It can't make anything new
//! happen: only these actions exist, and destroying is never one of them.

use crate::apps::App;
use crate::workspace::Workspace;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Hit {
    pub title: String,
    pub subtitle: String,
    pub action: Value,
    /// Ask before doing it (power actions).
    pub confirm: bool,
    pub score: f64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Results {
    pub hits: Vec<Hit>,
    /// Several results this close: show them as choices, don't pick one.
    pub ambiguous: bool,
    pub note: Option<String>,
}

const STOP: &[&str] = &["the", "my", "a", "an", "please", "to", "me", "can", "you", "i", "want", "would", "like",
                        "could", "for", "of", "is", "it", "this", "that", "just", "now", "some", "go", "let", "lets"];

pub fn tokens(q: &str) -> Vec<String> {
    q.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && !STOP.contains(t))
        .map(str::to_string)
        .take(24)
        .collect()
}

/// Edit distance where swapping two neighbouring letters ("redcue") is
/// one typo, as people make them (optimal string alignment).
pub fn lev(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// A word matches if it's the same, or one typo off (two for long words).
/// Short words must be exact: "show" isn't a typo of "slow".
pub fn like(token: &str, word: &str) -> bool {
    if token == word {
        return true;
    }
    let n = word.chars().count().min(token.chars().count());
    (n >= 5 && lev(token, word) <= 1) || (n >= 8 && lev(token, word) <= 2)
}

/// Requests to destroy something: never answered with a lookalike action.
const DESTROY: &[&str] = &["delete", "remove", "erase", "wipe", "format", "destroy", "uninstall", "kill", "rm"];

fn any(t: &[String], words: &[&str]) -> bool {
    t.iter().any(|x| words.iter().any(|w| like(x, w)))
}

fn number(t: &[String]) -> Option<u32> {
    t.iter().find_map(|x| x.trim_end_matches('%').parse::<u32>().ok()).filter(|n| *n <= 100)
}

const OPEN: &[&str] = &["open", "show", "launch", "start", "run", "display", "view", "browse"];

fn hit(title: impl Into<String>, subtitle: impl Into<String>, action: Value, score: f64) -> Hit {
    Hit { title: title.into(), subtitle: subtitle.into(), action, confirm: false, score }
}

fn folders(t: &[String], home: &Path, out: &mut Vec<Hit>) {
    let table: &[(&[&str], &str, &str)] = &[
        (&["downloads", "download"], "Downloads", "Downloads"),
        (&["documents", "docs", "document"], "Documents", "Documents"),
        (&["pictures", "photos", "images", "pics", "picture"], "Pictures", "Pictures"),
        (&["music", "songs"], "Music", "Music"),
        (&["videos", "movies", "video"], "Videos", "Videos"),
        (&["desktop"], "Desktop", "Desktop"),
        (&["home"], "", "Home folder"),
    ];
    let verb = any(t, OPEN) || any(t, &["folder", "files"]);
    for (words, dir, label) in table {
        if any(t, words) {
            let path = if dir.is_empty() { home.to_path_buf() } else { home.join(dir) };
            out.push(hit(format!("Open {label}"), path.to_string_lossy(), json!({"type": "open_path", "path": path}),
                         if verb { 0.9 } else { 0.75 }));
        }
    }
    if any(t, &["trash", "bin", "recycle", "deleted"]) {
        out.push(hit("Open the Trash", "Files that were deleted, ready to restore", json!({"type": "open_trash"}), 0.8));
    }
}

fn apps(t: &[String], apps: &BTreeMap<String, App>, out: &mut Vec<Hit>) {
    let verb = any(t, OPEN);
    let content: Vec<&String> = t.iter().filter(|x| !OPEN.iter().any(|w| like(x, w)) && *x != "app").collect();
    if content.is_empty() {
        return;
    }
    for app in apps.values().filter(|a| !a.hidden) {
        let name_words = tokens(&app.name);
        let mut words: Vec<String> = name_words.clone();
        words.extend(app.keywords.iter().cloned());
        words.push(app.id.to_lowercase());
        let matched = content.iter().filter(|c| words.iter().any(|w| like(c, w))).count();
        if matched == 0 {
            continue;
        }
        let name_hits = content.iter().filter(|c| name_words.iter().any(|w| like(c, w))).count();
        // Mostly the app's own name: strong; only a keyword: weaker.
        let mut score = 0.45 + 0.3 * matched as f64 / content.len() as f64 + if name_hits > 0 { 0.1 } else { 0.0 };
        if verb {
            score += 0.1;
        }
        out.push(hit(format!("Open {}", app.name), "App", json!({"type": "launch", "app": app.id}), score.min(0.95)));
    }
}

fn controls(t: &[String], out: &mut Vec<Hit>) {
    // The direction comes from the other words, not the one naming the thing
    // ("brightnes" is brightness, not "brighten").
    let objects = ["brightness", "bright", "screen", "display", "backlight", "volume", "sound", "audio"];
    let rest: Vec<String> = t.iter().filter(|x| !objects.iter().any(|o| like(x, o))).cloned().collect();
    let down = any(&rest, &["down", "lower", "reduce", "decrease", "dim", "darker", "less", "dimmer"]);
    let up = any(&rest, &["up", "raise", "increase", "brighter", "brighten", "more", "higher"]);
    if any(t, &["brightness", "bright", "screen", "display", "backlight"]) || any(t, &["dim", "brighter", "brighten", "darker"]) {
        let n = number(t);
        let (title, action) = match (n, down, up) {
            (Some(v), _, _) => (format!("Set brightness to {v} %"), json!({"type": "brightness", "value": v})),
            (None, true, _) => ("Lower the brightness".to_string(), json!({"type": "brightness", "delta": -10})),
            (None, _, true) => ("Raise the brightness".to_string(), json!({"type": "brightness", "delta": 10})),
            _ => ("Brightness settings".to_string(), json!({"type": "settings", "page": "power"})),
        };
        out.push(hit(title, "Screen", action, if n.is_some() || down || up { 0.9 } else { 0.6 }));
    }
    if any(t, &["volume", "sound", "audio", "louder", "quieter", "mute", "unmute", "speaker"]) {
        let action = if any(t, &["unmute"]) {
            json!({"type": "volume", "mute": false})
        } else if any(t, &["mute", "silence"]) {
            json!({"type": "volume", "mute": true})
        } else if let Some(v) = number(t) {
            json!({"type": "volume", "value": v})
        } else if down || any(t, &["quieter"]) {
            json!({"type": "volume", "delta": -10})
        } else if up || any(t, &["louder"]) {
            json!({"type": "volume", "delta": 10})
        } else {
            json!({"type": "launch", "app": "nexa-volume"})
        };
        let title = match (&action["mute"], &action["delta"], &action["value"]) {
            (Value::Bool(true), _, _) => "Mute the sound".to_string(),
            (Value::Bool(false), _, _) => "Unmute the sound".to_string(),
            (_, Value::Number(d), _) if d.as_i64() < Some(0) => "Turn the volume down".to_string(),
            (_, Value::Number(_), _) => "Turn the volume up".to_string(),
            (_, _, Value::Number(v)) => format!("Set the volume to {v} %"),
            _ => "Open the volume control".to_string(),
        };
        out.push(hit(title, "Sound", action, 0.88));
    }
    if any(t, &["lock"]) {
        out.push(hit("Lock the screen", "Asks for your password to come back", json!({"type": "lock"}), 0.9));
    }
    let power: &[(&[&str], &str, &str)] = &[
        (&["sleep", "suspend", "hibernate"], "suspend", "Put the computer to sleep"),
        (&["restart", "reboot"], "reboot", "Restart the computer"),
        (&["shutdown", "poweroff", "shut", "off"], "poweroff", "Shut down the computer"),
    ];
    for (words, what, title) in power {
        // "off" alone is too vague (lights off? Wi-Fi off?): needs a power word.
        let strong = t.iter().any(|x| words.iter().filter(|w| **w != "off" && **w != "shut").any(|w| like(x, w)));
        let shut_down = any(t, &["shut"]) && any(t, &["down", "off"]);
        let power_off = any(t, &["power", "turn", "switch"]) && any(t, &["off"]) && any(t, &["computer", "pc", "laptop", "power"]);
        if strong || (*what == "poweroff" && (shut_down || power_off)) {
            let mut h = hit(*title, "Asks first", json!({"type": "power", "what": what}), 0.9);
            h.confirm = true;
            out.push(h);
        }
    }
}

fn modes(t: &[String], out: &mut Vec<Hit>) {
    let table: &[(&str, &str, bool)] = &[
        ("low-resource", "Low resource mode: hold back background work", any(t, &["pause", "stop", "hold"]) && any(t, &["background", "scans", "scan", "indexing", "work"])
            || (any(t, &["low"]) && any(t, &["resource", "resources", "memory", "ram"]))),
        ("battery-saver", "Battery saver mode", any(t, &["battery"]) && any(t, &["saver", "save", "saving"])),
        ("performance", "Performance mode", any(t, &["performance", "fast", "faster", "speed", "turbo"]) && any(t, &["mode", "max", "more", "full", "boost"])),
        ("balanced", "Balanced mode", any(t, &["balanced", "normal"]) && any(t, &["mode"])),
        ("auto", "Automatic mode: NexaOS decides", (any(t, &["automatic", "auto"]) && any(t, &["mode"]))
            || (any(t, &["resume", "restart", "continue"]) && any(t, &["background", "scans", "work"]))),
    ];
    for (mode, title, yes) in table {
        if *yes {
            out.push(hit(*title, "Nexa MicroBrain", json!({"type": "mode", "mode": mode}), 0.92));
        }
    }
}

fn answers(t: &[String], out: &mut Vec<Hit>) {
    if any(t, &["slow", "slowly", "lag", "laggy", "lagging", "sluggish", "freezing", "frozen", "hanging", "stuck"])
        || (any(t, &["why"]) && any(t, &["busy", "hot", "fan"])) {
        out.push(hit("Why is the computer slow?", "What's using it, and what NexaOS already did", json!({"type": "answer", "topic": "slow"}), 0.93));
    }
    if any(t, &["battery"]) && !any(t, &["saver", "save", "saving"]) {
        out.push(hit("Battery health", "Charge, wear and whether it's charging", json!({"type": "answer", "topic": "battery"}), 0.85));
    }
    if any(t, &["memory", "ram"]) && any(t, &["using", "used", "free", "left", "much", "full"]) {
        out.push(hit("What's using the memory?", "The biggest programs, and whether memory ran out", json!({"type": "answer", "topic": "memory"}), 0.86));
    }
    // Why (Magic Layer).
    if any(t, &["crash", "crashed", "crashing", "closed", "disappeared", "vanished", "quit"])
        || (any(t, &["why"]) && any(t, &["close", "closes", "died"])) {
        out.push(hit("Why did an app close?", "Crashes, memory running out, or something ending it", json!({"type": "answer", "topic": "crash"}), 0.9));
    }
    if (any(t, &["internet", "online", "wifi", "network", "connection", "connected", "offline", "dns"])
        && any(t, &["no", "not", "isn't", "cant", "can't", "cannot", "why", "broken", "down", "working", "offline"]))
        || any(t, &["offline"]) {
        out.push(hit("Why is there no internet?", "Adapter, address, route and DNS, checked now", json!({"type": "answer", "topic": "network"}), 0.9));
    }
    // Rewind (Magic Layer): the settings timeline.
    if any(t, &["rewind", "revert"]) || (any(t, &["undo"]) && any(t, &["settings", "setting", "change", "changes", "theme"])) {
        out.push(hit("Rewind settings changes", "Go back to before a change \u{b7} Magic Layer", json!({"type": "settings", "page": "magic"}), 0.9));
    }
    if any(t, &["stopped", "paused", "frozen"]) || (any(t, &["why"]) && any(t, &["stop", "waiting"])) {
        out.push(hit("Why did something stop?", "Paused programs, waiting jobs and what MicroBrain did", json!({"type": "answer", "topic": "stopped"}), 0.84));
    }
}

/// FileTrail (Magic Layer): "where did I put report", "where is the invoice".
fn trail(t: &[String], home: &Path, out: &mut Vec<Hit>) {
    if !any(t, &["where", "moved", "went", "lost", "missing"]) || !crate::magic::enabled(home, "filetrail") {
        return;
    }
    const FILLER: &[&str] = &["where", "did", "do", "moved", "move", "went", "put", "find", "lost", "missing", "file", "files",
                              "folder", "gone", "are", "last", "has", "have", "was", "in"];
    let words: Vec<&str> = t.iter().map(String::as_str).filter(|w| !FILLER.contains(w)).collect();
    let Some(longest) = words.iter().max_by_key(|w| w.len()) else { return };
    let place = |p: &str| {
        let folder = Path::new(p).parent().map(|d| d.to_path_buf()).unwrap_or_default();
        if folder == home { "your home folder".to_string() } else {
            folder.strip_prefix(home).map(|r| r.display().to_string()).unwrap_or_else(|_| folder.display().to_string())
        }
    };
    let found = nexa_sys::filetrail::trail(home, longest, 50).into_iter()
        .filter(|i| { let n = i["name"].as_str().unwrap_or("").to_lowercase(); words.iter().all(|w| n.contains(w)) });
    for item in found.take(2) {
        let (name, kind, at) = (item["name"].as_str().unwrap_or(""), item["kind"].as_str().unwrap_or(""), item["at"].as_u64().unwrap_or(0));
        let now = item["now"].as_str().unwrap_or("");
        let when = format!("{kind} {}", crate::why::clock(at));
        match item["status"].as_str() {
            Some("here") => {
                let folder = Path::new(now).parent().map(|d| d.display().to_string()).unwrap_or_default();
                out.push(hit(format!("{name} is in {}", place(now)), format!("{when} \u{b7} FileTrail"),
                             json!({"type": "open_path", "path": folder}), 0.95));
            }
            Some("in-trash") => out.push(hit(format!("{name} is in the Trash"), format!("{when} \u{b7} FileTrail"),
                                             json!({"type": "open_path", "path": home.join(".local/share/Trash/files")}), 0.9)),
            _ => out.push(hit(format!("{name}: not where Files last put it"),
                              format!("Last {when} to {} \u{b7} FileTrail", place(item["to"].as_str().unwrap_or(""))),
                              json!({"type": "open_path", "path": Path::new(item["to"].as_str().unwrap_or("/")).parent().unwrap_or(home)}), 0.7)),
        }
    }
}

fn workspaces(t: &[String], list: &[Workspace], out: &mut Vec<Hit>) {
    let asked = any(t, &["workspace", "workspaces", "activate", "restore", "switch", "project"]);
    if any(t, &["save", "new", "create", "make"]) && any(t, &["workspace", "workspaces"]) {
        out.push(hit("Save these apps as a workspace", "Settings > Intelligence > Workspaces",
                     json!({"type": "settings", "page": "intelligence", "section": "workspaces"}), 0.9));
        return;
    }
    let content: Vec<&String> = t.iter().filter(|x| !["workspace", "workspaces", "activate", "restore", "switch", "open", "start", "mode"].contains(&x.as_str())).collect();
    for ws in list {
        let name = tokens(&ws.name);
        let matched = content.iter().filter(|c| name.iter().any(|w| like(c, w) || (c.len() >= 4 && w.starts_with(c.as_str())))).count();
        if matched == 0 && !(asked && content.is_empty()) {
            continue;
        }
        let score = if matched == 0 { 0.5 } else { 0.6 + 0.3 * matched as f64 / name.len().max(1) as f64 + if asked { 0.08 } else { 0.0 } };
        out.push(hit(format!("Activate \u{201c}{}\u{201d}", ws.name), format!("{} apps", ws.apps.len()),
                     json!({"type": "workspace", "name": ws.name}), score.min(0.97)));
    }
}

fn settings(t: &[String], out: &mut Vec<Hit>) {
    let table: &[(&[&str], &str, &str)] = &[
        (&["wifi", "wireless", "internet", "network"], "network", "Wi-Fi and network"),
        (&["bluetooth", "headphones", "earbuds"], "bluetooth", "Bluetooth"),
        (&["theme", "wallpaper", "appearance", "personalization", "personalise", "personalize", "dark", "colours", "colors"], "personalization", "Personalization"),
        (&["privacy", "patterns"], "privacy", "Privacy"),
        (&["keyboard", "time", "date", "timezone", "clock", "layout"], "time", "Time & Keyboard"),
        (&["users", "accounts", "account", "password"], "users", "Users"),
        (&["updates", "update", "upgrade"], "updates", "Updates"),
        (&["link", "phone", "pair", "nexa link", "drop"], "nexa", "Nexa Link"),
        (&["intelligence", "microbrain", "brain", "agents"], "intelligence", "Intelligence"),
        (&["power", "sleep", "screensaver"], "power", "Power"),
        (&["vpn", "wireguard"], "vpn", "VPN"),
        (&["startup", "services"], "startup", "Startup"),
        (&["magic", "living", "background", "animation", "animated"], "magic", "Magic Layer"),
    ];
    let explicit = any(t, &["settings", "setting", "preferences", "options", "configure", "settings"]);
    for (words, page, label) in table {
        if any(t, words) {
            let action = if *page == "network" { json!({"type": "launch", "app": "nexa-network"}) } else { json!({"type": "settings", "page": page}) };
            out.push(hit(format!("{label} settings"), "Settings", action, if explicit { 0.85 } else { 0.55 }));
        }
    }
    if explicit && out.iter().all(|h| h.action["type"] != "settings") {
        out.push(hit("Open Settings", "Settings", json!({"type": "settings", "page": ""}), 0.8));
    }
}

/// What the model is sure of, as actions (score: how sure, 0.55 to 0.95).
/// Also: how many words the sentence leans on that its likeliest command
/// never uses (2 or more: it's about something else, "lock the front door").
fn model(query: &str, all_apps: &BTreeMap<String, App>, list: &[Workspace], home: &Path, out: &mut Vec<Hit>) -> Option<(String, usize)> {
    let p = nexa_intent::builtin().predict(query);
    let off = nexa_intent::vocab::unfamiliar(&p.raw, &p.words, p.arg.as_deref()).len();
    Some((model_hits(&p, all_apps, list, home, out)?, off))
}

fn model_hits(p: &nexa_intent::Prediction, all_apps: &BTreeMap<String, App>, list: &[Workspace], home: &Path, out: &mut Vec<Hit>) -> Option<String> {
    let intent = p.intent.as_str();
    let score = 0.55 + 0.4 * p.p as f64;
    let num = nexa_intent::number(&p.words);
    let mut push = |title: &str, sub: &str, action: Value| out.push(hit(title, sub, action, score));
    let folder = |dir: &str| if dir.is_empty() { home.to_path_buf() } else { home.join(dir) };
    let open = |dir: &str, label: &str, out: &mut Vec<Hit>| {
        let path = folder(dir);
        out.push(hit(format!("Open {label}"), path.to_string_lossy(), json!({"type": "open_path", "path": path}), score));
    };
    match intent {
        "open_downloads" => open("Downloads", "Downloads", out),
        "open_documents" => open("Documents", "Documents", out),
        "open_pictures" => open("Pictures", "Pictures", out),
        "open_music_folder" => open("Music", "Music", out),
        "open_videos" => open("Videos", "Videos", out),
        "open_desktop" => open("Desktop", "Desktop", out),
        "open_home" => open("", "Home folder", out),
        "open_trash" => push("Open the Trash", "Files that were deleted, ready to restore", json!({"type": "open_trash"})),
        "launch" | "find_file" | "workspace_go" => {
            // The name the model found, looked up the usual way (typos and all).
            let Some(name) = p.arg.as_deref() else { return None };
            let mut found = Vec::new();
            match intent {
                "launch" => {
                    let mut t = tokens(name);
                    t.insert(0, "open".into());
                    apps(&t, all_apps, &mut found);
                }
                "find_file" => {
                    let mut t = tokens(name);
                    t.insert(0, "where".into());
                    trail(&t, home, &mut found);
                }
                _ => {
                    let mut t = tokens(name);
                    t.push("workspace".into());
                    workspaces(&t, list, &mut found);
                }
            }
            for mut h in found {
                h.score = h.score.max(score.min(0.95) * if h.score >= 0.7 { 1.0 } else { 0.9 });
                out.push(h);
            }
        }
        "workspace_save" => push("Save these apps as a workspace", "Settings > Intelligence > Workspaces",
                                 json!({"type": "settings", "page": "intelligence", "section": "workspaces"})),
        "brightness_down" => push("Lower the brightness", "Screen", json!({"type": "brightness", "delta": -10})),
        "brightness_up" => push("Raise the brightness", "Screen", json!({"type": "brightness", "delta": 10})),
        "brightness_set" => if let Some(v) = num.filter(|v| *v <= 100) {
            push(&format!("Set brightness to {v} %"), "Screen", json!({"type": "brightness", "value": v.max(1)}))
        },
        "volume_down" => push("Turn the volume down", "Sound", json!({"type": "volume", "delta": -10})),
        "volume_up" => push("Turn the volume up", "Sound", json!({"type": "volume", "delta": 10})),
        "volume_set" => if let Some(v) = num.filter(|v| *v <= 100) {
            push(&format!("Set the volume to {v} %"), "Sound", json!({"type": "volume", "value": v}))
        },
        "mute" => push("Mute the sound", "Sound", json!({"type": "volume", "mute": true})),
        "unmute" => push("Unmute the sound", "Sound", json!({"type": "volume", "mute": false})),
        "lock" => push("Lock the screen", "Asks for your password to come back", json!({"type": "lock"})),
        "suspend" | "reboot" | "poweroff" => {
            let (what, title) = match intent {
                "suspend" => ("suspend", "Put the computer to sleep"),
                "reboot" => ("reboot", "Restart the computer"),
                _ => ("poweroff", "Shut down the computer"),
            };
            let mut h = hit(title, "Asks first", json!({"type": "power", "what": what}), score);
            h.confirm = true;
            out.push(h);
        }
        "mode_low_resource" => push("Low resource mode: hold back background work", "Nexa MicroBrain", json!({"type": "mode", "mode": "low-resource"})),
        "mode_battery_saver" => push("Battery saver mode", "Nexa MicroBrain", json!({"type": "mode", "mode": "battery-saver"})),
        "mode_performance" => push("Performance mode", "Nexa MicroBrain", json!({"type": "mode", "mode": "performance"})),
        "mode_balanced" => push("Balanced mode", "Nexa MicroBrain", json!({"type": "mode", "mode": "balanced"})),
        "mode_auto" => push("Automatic mode: NexaOS decides", "Nexa MicroBrain", json!({"type": "mode", "mode": "auto"})),
        "answer_slow" => push("Why is the computer slow?", "What's using it, and what NexaOS already did", json!({"type": "answer", "topic": "slow"})),
        "answer_battery" => push("Battery health", "Charge, wear and whether it's charging", json!({"type": "answer", "topic": "battery"})),
        "answer_memory" => push("What's using the memory?", "The biggest programs, and whether memory ran out", json!({"type": "answer", "topic": "memory"})),
        "answer_crash" => push("Why did an app close?", "Crashes, memory running out, or something ending it", json!({"type": "answer", "topic": "crash"})),
        "answer_network" => push("Why is there no internet?", "Adapter, address, route and DNS, checked now", json!({"type": "answer", "topic": "network"})),
        "answer_stopped" => push("Why did something stop?", "Paused programs, waiting jobs and what MicroBrain did", json!({"type": "answer", "topic": "stopped"})),
        "rewind" => push("Rewind settings changes", "Go back to before a change \u{b7} Magic Layer", json!({"type": "settings", "page": "magic"})),
        "settings_network" => push("Wi-Fi and network settings", "Settings", json!({"type": "launch", "app": "nexa-network"})),
        "layer_go" => if let Some(n) = num.filter(|n| (1..=5).contains(n)) {
            push(&format!("Go to layer {n}"), "Strata", json!({"type": "layer", "to": n}))
        },
        "layer_next" => push("One layer deeper", "Strata", json!({"type": "layer", "step": 1})),
        "layer_prev" => push("One layer back up", "Strata", json!({"type": "layer", "step": -1})),
        "quiet_on" => push("Turn on Do not disturb", "Notifications wait in Deck's Signals card", json!({"type": "quiet", "on": true})),
        "quiet_off" => push("Turn off Do not disturb", "Notifications pop up again", json!({"type": "quiet", "on": false})),
        "windows" => push("Show all your windows", "Constellation \u{b7} Super+Tab", json!({"type": "windows"})),
        "mail_find" | "mail_write" => {
            // Mail is a Store app: without it, the Store's Mail page instead.
            if !all_apps.contains_key("nexa-mail") {
                push("Get Mail from the Store", "Mail isn't installed yet", json!({"type": "launch", "app": "nexa-store"}));
                return Some(intent.into());
            }
            let name = p.arg.as_deref().map(str::trim).filter(|n| !n.is_empty());
            match (intent, name) {
                ("mail_find", Some(n)) => push(&format!("Find mail: {n}"), "Mail searches every message", json!({"type": "mail", "search": n})),
                ("mail_find", None) => push("Open Mail", "Your email", json!({"type": "mail"})),
                (_, Some(n)) => push(&format!("Write to {n}"), "Mail finds the address in mail they sent", json!({"type": "mail", "write": n})),
                (_, None) => push("Write an email", "Mail", json!({"type": "mail", "write": ""})),
            }
        }
        "destroy" => return Some("destroy".into()),
        s if s.starts_with("settings") => {
            let page = s.strip_prefix("settings_").unwrap_or("");
            let (page, label) = match page {
                "" => ("", "Settings"),
                "bluetooth" => ("bluetooth", "Bluetooth settings"),
                "personalization" => ("personalization", "Personalization settings"),
                "privacy" => ("privacy", "Privacy settings"),
                "time" => ("time", "Time & Keyboard settings"),
                "users" => ("users", "Users settings"),
                "updates" => ("updates", "Updates settings"),
                "link" => ("nexa", "Nexa Link settings"),
                "intelligence" => ("intelligence", "Intelligence settings"),
                "power" => ("power", "Power settings"),
                "vpn" => ("vpn", "VPN settings"),
                "startup" => ("startup", "Startup settings"),
                "magic" => ("magic", "Magic Layer settings"),
                "layers" => ("layers", "Layers settings"),
                "notifications" => ("notifications", "Notifications settings"),
                "display" => ("display", "Display settings"),
                "sound" => ("sound", "Sound settings"),
                "accessibility" => ("accessibility", "Accessibility settings"),
                _ => return None,
            };
            push(if page.is_empty() { "Open Settings" } else { label }, "Settings", json!({"type": "settings", "page": page}));
        }
        _ => {}
    }
    Some(intent.to_string())
}

pub fn ask(query: &str, all_apps: &BTreeMap<String, App>, list: &[Workspace], home: &Path) -> Results {
    let t = tokens(query);
    let mut out = Vec::new();
    let (understood, off) = if t.is_empty() { (None, 0) } else {
        model(query, all_apps, list, home, &mut out).map(|(i, o)| (Some(i), o)).unwrap_or((None, 0))
    };
    if any(&t, DESTROY) || understood.as_deref() == Some("destroy") {
        return Results { hits: Vec::new(), ambiguous: false,
                         note: Some("I can't delete or remove things from here. Files' Move to Trash does it, and can be undone.".into()) };
    }
    if !t.is_empty() {
        // About something else ("lock the front door"): the word rules for
        // the computer's own controls stand back; names are still looked up.
        let elsewhere = off >= 2 && understood.as_deref() == Some("unknown");
        if !elsewhere {
            folders(&t, home, &mut out);
        }
        apps(&t, all_apps, &mut out);
        if !elsewhere {
            controls(&t, &mut out);
            modes(&t, &mut out);
            answers(&t, &mut out);
        }
        trail(&t, home, &mut out);
        workspaces(&t, list, &mut out);
        if !elsewhere {
            settings(&t, &mut out);
        }
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    // The same action twice (two routes to it): keep the best.
    let mut seen = Vec::new();
    out.retain(|h| {
        let key = h.action.to_string();
        let new = !seen.contains(&key);
        seen.push(key);
        new
    });
    out.truncate(5);
    let ambiguous = out.len() >= 2 && (out[0].score - out[1].score).abs() < 0.05;
    let note = out.is_empty().then(|| if understood.as_deref() == Some("unknown") && t.len() > 1 {
        "That sounds like a question, not a command: the Assistant can answer it.".to_string()
    } else {
        "I can't do that yet. Try: \u{201c}open Downloads\u{201d}, \u{201c}reduce brightness\u{201d}, \u{201c}why is my computer slow\u{201d}, \
         \u{201c}battery health\u{201d}, \u{201c}pause background work\u{201d}, or the name of an app or workspace.".to_string()
    });
    Results { hits: out, ambiguous, note }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps;

    fn setup() -> (BTreeMap<String, App>, Vec<Workspace>) {
        let mut a = BTreeMap::new();
        for (id, text) in [("nexa-edit", "[Desktop Entry]\nName=Nexa Editor\nExec=x %F\nKeywords=text;code;notes;\n"),
                           ("nexa-files", "[Desktop Entry]\nName=Files\nExec=x %U\n"),
                           ("nexa-music", "[Desktop Entry]\nName=Nexa Music\nExec=x\n"),
                           ("nexa-calc", "[Desktop Entry]\nName=Calculator\nExec=x\n"),
                           ("nexa-volume", "[Desktop Entry]\nName=Volume\nExec=x\nNoDisplay=true\n")] {
            a.insert(id.to_string(), apps::parse(id, text).unwrap());
        }
        let ws = vec![Workspace { name: "Coding".into(), apps: vec![], folder: None, created: 0, used: 0 },
                      Workspace { name: "Android Development".into(), apps: vec![], folder: None, created: 0, used: 0 }];
        (a, ws)
    }

    fn top(q: &str) -> Value {
        let (a, w) = setup();
        let r = ask(q, &a, &w, Path::new("/home/u"));
        r.hits.first().map(|h| h.action.clone()).unwrap_or(Value::Null)
    }

    #[test]
    fn everyday_commands_route_correctly() {
        assert_eq!(top("Open Downloads"), json!({"type": "open_path", "path": "/home/u/Downloads"}));
        assert_eq!(top("reduce brightness"), json!({"type": "brightness", "delta": -10}));
        assert_eq!(top("brightness 40"), json!({"type": "brightness", "value": 40}));
        assert_eq!(top("why is my computer slow?"), json!({"type": "answer", "topic": "slow"}));
        assert_eq!(top("show battery health"), json!({"type": "answer", "topic": "battery"}));
        assert_eq!(top("why did files crash"), json!({"type": "answer", "topic": "crash"}));
        assert_eq!(top("why is there no internet"), json!({"type": "answer", "topic": "network"}));
        assert_eq!(top("wifi not working"), json!({"type": "answer", "topic": "network"}));
        assert_eq!(top("undo my theme change"), json!({"type": "settings", "page": "magic"}));
        assert_eq!(top("how much memory is used"), json!({"type": "answer", "topic": "memory"}));
        assert_eq!(top("activate coding workspace"), json!({"type": "workspace", "name": "Coding"}));
        assert_eq!(top("pause background scans"), json!({"type": "mode", "mode": "low-resource"}));
        assert_eq!(top("open calculator"), json!({"type": "launch", "app": "nexa-calc"}));
        assert_eq!(top("mute"), json!({"type": "volume", "mute": true}));
        assert_eq!(top("lock the screen"), json!({"type": "lock"}));
        assert_eq!(top("bluetooth settings"), json!({"type": "settings", "page": "bluetooth"}));
        assert_eq!(top("save this workspace"), json!({"type": "settings", "page": "intelligence", "section": "workspaces"}));
    }

    #[test]
    fn where_did_it_go_asks_filetrail() {
        let home = std::env::temp_dir().join(format!("zs-trail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Work/Docs")).unwrap();
        std::fs::write(home.join("Work/Docs/report.pdf"), b"x").unwrap();
        if std::env::var_os("XDG_STATE_HOME").is_some() || std::env::var_os("XDG_CONFIG_HOME").is_some() {
            return;
        }
        let mut f = nexa_sys::filetrail::open(&home).unwrap();
        let p = |s: &str| home.join(s).display().to_string();
        nexa_sys::filetrail::append(&mut f, "move", &[json!({"op": "moved", "from": p("Downloads/report.pdf"), "to": p("Work/Docs/report.pdf")})], None, 100);
        drop(f);
        let (apps, ws) = setup();
        let r = ask("where did I put the report", &apps, &ws, &home);
        assert_eq!(r.hits[0].title, "report.pdf is in Work/Docs", "{:?}", r.hits);
        assert_eq!(r.hits[0].action, json!({"type": "open_path", "path": p("Work/Docs")}));
        assert!(ask("where is the invoice", &apps, &ws, &home).hits.iter().all(|h| !h.subtitle.contains("FileTrail")));
    }

    #[test]
    fn mail_from_nexa_mind() {
        assert_eq!(top("find the email from kofi"), json!({"type": "launch", "app": "nexa-store"}), "no Mail yet: the Store");
        let (mut a, w) = setup();
        a.insert("nexa-mail".into(), apps::parse("nexa-mail", "[Desktop Entry]\nName=Mail\nExec=x %U\nMimeType=x-scheme-handler/mailto;\n").unwrap());
        let act = |q: &str| ask(q, &a, &w, Path::new("/home/u")).hits.first().map(|h| h.action.clone()).unwrap_or(Value::Null);
        assert_eq!(act("find the email from kofi"), json!({"type": "mail", "search": "kofi"}));
        assert_eq!(act("search my inbox for school fees"), json!({"type": "mail", "search": "school fees"}));
        assert_eq!(act("email my landlord"), json!({"type": "mail", "write": "my landlord"}));
        assert_eq!(act("write a new email"), json!({"type": "mail", "write": ""}));
        let open = act("open my mail");
        assert!(open == json!({"type": "mail"}) || open == json!({"type": "launch", "app": "nexa-mail"}), "opening Mail either way: {open}");
    }

    #[test]
    fn the_model_understands_other_wordings() {
        assert_eq!(top("the screen is way too bright"), json!({"type": "brightness", "delta": -10}));
        assert_eq!(top("i can barely hear it turn it up"), json!({"type": "volume", "delta": 10}));
        assert_eq!(top("could you fire up the calculater for me"), json!({"type": "launch", "app": "nexa-calc"}));
        assert_eq!(top("take me to layer 3"), json!({"type": "layer", "to": 3}));
        assert_eq!(top("dive deeper"), json!({"type": "layer", "step": 1}));
        assert_eq!(top("no notifications for now"), json!({"type": "quiet", "on": true}));
        assert_eq!(top("show my notifications again"), json!({"type": "quiet", "on": false}));
        assert_eq!(top("show me all my windows"), json!({"type": "windows"}));
        assert_eq!(top("make the text bigger"), json!({"type": "settings", "page": "accessibility"}));
        assert_eq!(top("my pc keeps freezing"), json!({"type": "answer", "topic": "slow"}));
        assert_eq!(top("restore my coding workspace"), json!({"type": "workspace", "name": "Coding"}));
        let (a, w) = setup();
        let r = ask("restart my pc", &a, &w, Path::new("/home/u"));
        assert!(r.hits[0].confirm && r.hits[0].action == json!({"type": "power", "what": "reboot"}), "power still asks first");
    }

    #[test]
    fn the_model_never_turns_chat_or_destroying_into_an_action() {
        let (a, w) = setup();
        // ("write an email to my teacher" is a Mail command now: it opens a draft, nothing is sent)
        for q in ["tell me a joke", "write a poem about my teacher", "who is the president of ghana", "how do i bake bread"] {
            let r = ask(q, &a, &w, Path::new("/home/u"));
            assert!(r.hits.is_empty(), "{q}: {:?}", r.hits);
            assert!(r.note.unwrap().contains("Assistant"), "{q}: a question goes to the Assistant");
        }
        for q in ["erase my documents", "factory reset the laptop", "get rid of all my photos", "uninstall the editor"] {
            let r = ask(q, &a, &w, Path::new("/home/u"));
            assert!(r.hits.is_empty() && r.note.unwrap().starts_with("I can't delete"), "{q}");
        }
    }

    #[test]
    fn typos_are_forgiven() {
        assert_eq!(top("opne donwloads"), json!({"type": "open_path", "path": "/home/u/Downloads"}));
        assert_eq!(top("calculater"), json!({"type": "launch", "app": "nexa-calc"}));
        assert_eq!(top("redcue brightnes"), json!({"type": "brightness", "delta": -10}));
        assert_eq!(top("andriod"), json!({"type": "workspace", "name": "Android Development"}));
    }

    #[test]
    fn power_asks_first_and_off_alone_is_not_shut_down() {
        let (a, w) = setup();
        let r = ask("shut down", &a, &w, Path::new("/home/u"));
        assert_eq!(r.hits[0].action, json!({"type": "power", "what": "poweroff"}));
        assert!(r.hits[0].confirm, "power actions ask first");
        assert!(ask("turn the lights off", &a, &w, Path::new("/home/u")).hits.iter().all(|h| h.action["type"] != "power"));
    }

    #[test]
    fn unsupported_requests_say_so() {
        let (a, w) = setup();
        let r = ask("delete all my files forever", &a, &w, Path::new("/home/u"));
        assert!(r.hits.is_empty(), "{:?}", r.hits);
        assert!(r.note.unwrap().starts_with("I can't delete"), "destroying is never mapped to something else");
        for q in ["knit me a scarf", "lock the front door", "turn down the heating", "start the washing machine"] {
            let r = ask(q, &a, &w, Path::new("/home/u"));
            assert!(r.hits.is_empty(), "{q}: not the computer's, so no action: {:?}", r.hits);
            assert!(r.note.is_some(), "{q}: said so");
        }
        assert!(ask("", &a, &w, Path::new("/home/u")).hits.is_empty());
        assert!(ask("asdfgh qwerty", &a, &w, Path::new("/home/u")).note.is_some());
    }

    #[test]
    fn hidden_apps_and_hits_close_together() {
        let (a, w) = setup();
        let r = ask("music", &a, &w, Path::new("/home/u"));
        let kinds: Vec<&str> = r.hits.iter().map(|h| h.action["type"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"open_path") && kinds.contains(&"launch"), "the Music folder and the Music app: {kinds:?}");
        assert!(r.ambiguous || r.hits.len() > 1, "two meanings are offered as choices");
        assert!(ask("open volume", &a, &w, Path::new("/home/u")).hits.iter().all(|h| h.action != json!({"type": "launch", "app": "nexa-volume"}) || h.title == "Open the volume control"),
                "a hidden app isn't offered by name");
    }
}
