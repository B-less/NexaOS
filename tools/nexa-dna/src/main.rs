//! nexa-dna: Interface DNA for NexaOS apps (TODOnexa_living.md, phase 6).
//!
//! Each first-party app ships a contract (/usr/share/nexa/interface/<app>.json)
//! saying which layouts and traits it supports. The person's choices live in
//! ~/.config/nexa/interface (global.json, apps/<app>.json), each change
//! keeping the previous version for Undo. Every change is validated against
//! the contract first; a rejected one comes back with the reasons.
//!
//!   nexa-dna apps                         the apps and what they support
//!   nexa-dna show APP|global              contract, saved choices, what's in effect
//!   nexa-dna css APP                      the CSS the app loads
//!   nexa-dna check APP|global JSON        dry run: problems, changes, who's affected, CSS
//!   nexa-dna set APP|global KEY VALUE...  change some traits
//!   nexa-dna apply APP|global JSON        replace the whole profile
//!   nexa-dna undo | reset APP|global
//!   nexa-dna presets | preset NAME APP|global
//!   nexa-dna export APP|global FILE | import FILE APP|global

mod css;
mod model;

use model::{Contract, Dna};
use serde_json::{json, Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const UNDO_DEPTH: usize = 10;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into())
}

fn config_dir() -> PathBuf {
    std::env::var_os("NEXA_DNA_HOME").map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config")).join("nexa/interface")
    })
}

fn contract_dirs() -> Vec<PathBuf> {
    match std::env::var_os("NEXA_DNA_CONTRACTS") {
        Some(d) => vec![PathBuf::from(d)],
        None => vec![home().join(".local/share/nexa/interface"), "/usr/share/nexa/interface".into()],
    }
}

fn valid_app(app: &str) -> bool {
    app == "global" || (!app.is_empty() && app.len() <= 64 && app.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && !app.starts_with('.'))
}

fn contracts() -> Vec<Contract> {
    let mut out: Vec<Contract> = Vec::new();
    for dir in contract_dirs() {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            if let Ok(c) = fs::read_to_string(e.path()).map_err(|_| ()).and_then(|t| serde_json::from_str::<Contract>(&t).map_err(|_| ())) {
                if valid_app(&c.app) && !out.iter().any(|o| o.app == c.app) {
                    out.push(c);
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn contract(app: &str) -> Option<Contract> {
    contracts().into_iter().find(|c| c.app == app)
}

fn path(app: &str) -> PathBuf {
    if app == "global" { config_dir().join("global.json") } else { config_dir().join("apps").join(format!("{app}.json")) }
}

fn load(app: &str) -> Dna {
    fs::read_to_string(path(app)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(model::migrate).and_then(|v| v.as_object().cloned()).map(|mut m| { m.remove("version"); m }).unwrap_or_default()
}

fn write(p: &Path, v: &Value) -> io::Result<()> {
    fs::create_dir_all(p.parent().unwrap_or(Path::new(".")))?;
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    fs::rename(tmp, p)
}

fn undo_dir(app: &str) -> PathBuf {
    config_dir().join("undo").join(app)
}

/// Saves `dna` for `app`, keeping what was there for Undo.
fn save(app: &str, dna: &Dna) -> io::Result<()> {
    let before = load(app);
    let dir = undo_dir(app);
    fs::create_dir_all(&dir)?;
    let mut stack: Vec<PathBuf> = fs::read_dir(&dir)?.flatten().map(|e| e.path()).collect();
    stack.sort();
    while stack.len() >= UNDO_DEPTH {
        fs::remove_file(stack.remove(0)).ok();
    }
    let n = stack.last().and_then(|p| p.file_stem()?.to_str()?.parse::<u64>().ok()).unwrap_or(0) + 1;
    write(&dir.join(format!("{n:06}.json")), &Value::Object(before))?;
    let mut v = dna.clone();
    v.insert("version".into(), json!(model::VERSION));
    write(&path(app), &Value::Object(v))
}

fn undo(app: &str) -> io::Result<bool> {
    let dir = undo_dir(app);
    let mut stack: Vec<PathBuf> = fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    stack.sort();
    let Some(last) = stack.pop() else { return Ok(false) };
    let prev: Value = serde_json::from_str(&fs::read_to_string(&last)?).unwrap_or(json!({}));
    let mut v = prev.as_object().cloned().unwrap_or_default();
    v.insert("version".into(), json!(model::VERSION));
    write(&path(app), &Value::Object(v))?;
    fs::remove_file(last)?;
    Ok(true)
}

fn effective(app: &str) -> Dna {
    let c = if app == "global" { None } else { contract(app) };
    model::resolve(&load("global"), &if app == "global" { Dna::new() } else { load(app) }, c.as_ref())
}

/// Dry run of `proposed` (the whole profile) for `app`.
fn check(app: &str, proposed: &Dna) -> Value {
    let c = if app == "global" { None } else { contract(app) };
    if app != "global" && c.is_none() {
        return json!({"ok": false, "problems": [{"trait_name": "app",
            "why": format!("{app} has no Interface DNA contract: it only follows the system theme")}]});
    }
    let problems = model::validate(proposed, c.as_ref());
    let before = effective(app);
    let after = if app == "global" { model::resolve(proposed, &Dna::new(), None) } else { model::resolve(&load("global"), proposed, c.as_ref()) };
    let changes: Vec<Value> = after.iter().filter(|(k, v)| before.get(*k) != Some(*v))
        .map(|(k, v)| json!({"trait": k, "from": before.get(k).cloned().unwrap_or(Value::Null), "to": v})).collect();
    // Who a global change reaches: every app that hasn't chosen that trait itself.
    let affected: Vec<String> = if app == "global" {
        contracts().into_iter().filter(|c| {
            let own = load(&c.app);
            changes.iter().any(|ch| ch["trait"].as_str().is_some_and(|t| !own.contains_key(t) && c.traits.iter().any(|x| x == t)))
        }).map(|c| c.name).collect()
    } else {
        c.iter().map(|c| c.name.clone()).collect()
    };
    json!({"ok": problems.is_empty(), "problems": problems, "changes": changes, "affected": affected,
           "css": css::css(&after), "undo": true})
}

fn parse_dna(text: &str) -> Result<Dna, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let v = model::migrate(v);
    let mut m = v.as_object().cloned().ok_or("a profile is a JSON object")?;
    m.remove("version");
    Ok(m)
}

fn out(v: Value) -> i32 {
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    if v["ok"] == false { 1 } else { 0 }
}

fn apply(app: &str, dna: Dna) -> i32 {
    let r = check(app, &dna);
    if r["ok"] != true {
        return out(r);
    }
    match save(app, &dna) {
        Ok(()) => out(json!({"ok": true, "changes": r["changes"], "effective": effective(app)})),
        Err(e) => out(json!({"ok": false, "problems": [{"trait_name": "file", "why": e.to_string()}]})),
    }
}

fn run(args: &[String]) -> i32 {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Some(app) = match a.as_slice() {
        [_, app, ..] => Some(*app),
        _ => None,
    } {
        if !["presets", "export", "import"].contains(&a[0]) && !valid_app(app) {
            return out(json!({"ok": false, "problems": [{"trait_name": "app", "why": "not an app name"}]}));
        }
    }
    match a.as_slice() {
        ["apps"] => out(json!({"ok": true, "apps": contracts(), "layouts": model::LAYOUTS,
                               "traits": model::TRAITS.iter().map(|(k, v, d)| json!({"name": k, "values": v, "default": d})).collect::<Vec<_>>()})),
        ["show", app] => {
            let c = if *app == "global" { None } else { contract(app) };
            let saved = load(app);
            out(json!({"ok": true, "app": app, "contract": c, "saved": saved, "effective": effective(app),
                       "problems": model::validate(&saved, c.as_ref())}))
        }
        ["css", app] => {
            print!("{}", css::css(&effective(app)));
            0
        }
        ["check", app, json] => match parse_dna(json) {
            Ok(d) => out(check(app, &d)),
            Err(e) => out(json!({"ok": false, "problems": [{"trait_name": "profile", "why": e}]})),
        },
        ["apply", app, json] => match parse_dna(json) {
            Ok(d) => apply(app, d),
            Err(e) => out(json!({"ok": false, "problems": [{"trait_name": "profile", "why": e}]})),
        },
        ["set", app, pairs @ ..] if !pairs.is_empty() && pairs.len() % 2 == 0 => {
            let mut d = load(app);
            for kv in pairs.chunks(2) {
                d.insert(kv[0].to_string(), Value::String(kv[1].to_string()));
            }
            apply(app, d)
        }
        ["undo", app] => match undo(app) {
            Ok(true) => out(json!({"ok": true, "effective": effective(app)})),
            Ok(false) => out(json!({"ok": false, "problems": [{"trait_name": "undo", "why": "nothing to undo"}]})),
            Err(e) => out(json!({"ok": false, "problems": [{"trait_name": "undo", "why": e.to_string()}]})),
        },
        ["reset", app] => apply(app, Dna::new()),
        ["presets"] => out(json!({"ok": true, "presets": model::presets().iter().map(|(k, t, d)| json!({"name": k, "title": t, "dna": d})).collect::<Vec<_>>()})),
        ["preset", name, app] => match model::presets().into_iter().find(|(k, _, _)| k == name) {
            Some((_, _, d)) => {
                // Keep the app's layout; a preset is about how it looks.
                let mut merged: Map<String, Value> = load(app).into_iter().filter(|(k, _)| k == "layout" || k == "navigation").collect();
                merged.extend(d);
                apply(app, merged)
            }
            None => out(json!({"ok": false, "problems": [{"trait_name": "preset", "why": format!("no preset called {name}")}]})),
        },
        ["export", app, file] => {
            let mut v = load(app);
            v.insert("version".into(), json!(model::VERSION));
            v.insert("profile".into(), json!(format!("{app} interface")));
            match write(Path::new(file), &Value::Object(v)) {
                Ok(()) => out(json!({"ok": true, "file": file})),
                Err(e) => out(json!({"ok": false, "problems": [{"trait_name": "file", "why": e.to_string()}]})),
            }
        }
        ["import", file, app] if valid_app(app) => match fs::read_to_string(file).map_err(|e| e.to_string()).and_then(|t| parse_dna(&t)) {
            Ok(mut d) => {
                d.remove("profile");
                if *app == "global" {
                    d.remove("layout");
                }
                apply(app, d)
            }
            Err(e) => out(json!({"ok": false, "problems": [{"trait_name": "file", "why": e}]})),
        },
        _ => {
            eprintln!("usage: nexa-dna apps | show APP | css APP | check APP JSON | apply APP JSON | set APP KEY VALUE... |\n       \
                       undo APP | reset APP | presets | preset NAME APP | export APP FILE | import FILE APP   (APP may be \"global\")");
            2
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}
