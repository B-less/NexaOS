//! The socket commands for the user-facing features (ZeroSearch,
//! PocketWorkspace, HabitCore) and the Learning agent.
//!
//!   {"cmd": "ask", "query": "open downloads"}            ZeroSearch results
//!   {"cmd": "answer", "topic": "slow" | "battery" | ...}   a plain-language answer
//!   {"cmd": "why", "topic": "slow" | "crash" | ...}        Why: proven / possible findings (why.rs)
//!   {"cmd": "rewind"} / {"cmd": "rewind-to", "id"}          Rewind: settings timeline (rewind.rs)
//!   {"cmd": "workspaces"}                                  the saved workspaces
//!   {"cmd": "workspace-save", "name", "windows", "folder"} save what's on screen
//!   {"cmd": "workspace-plan", "name"}                      what restoring does
//!   {"cmd": "workspace-delete", "name"}
//!   {"cmd": "suggestions"}                                 HabitCore's ideas
//!   {"cmd": "suggestion-dismiss", "apps": [...]}           never suggest it again

use crate::agents::{Agent, Coordinator, Ctx, Event, Notice};
use crate::config::Config;
use crate::habit::{self, Habits};
use crate::resources::Level;
use crate::store::Store;
use crate::workspace::{self, Window, Workspaces};
use crate::{home, now, AppCache};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn gb(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

/// "Why is my computer slow?" from what MicroBrain sees right now.
pub fn slow_answer(c: &Coordinator) -> String {
    let Some(s) = c.monitor.last.as_ref() else {
        return "MicroBrain has only just started; ask again in a few seconds.".into();
    };
    let mut lines = Vec::new();
    let used = s.mem_total_mb.saturating_sub(s.mem_avail_mb);
    let mem_word = match c.monitor.mem_level {
        Level::Normal => "no pressure",
        Level::Elevated => "some pressure",
        Level::High => "high pressure",
        Level::Critical => "very high pressure",
    };
    lines.push(format!("Memory: {} of {} in use ({mem_word}).", gb(used), gb(s.mem_total_mb)));
    if let Some((app, mb)) = crate::agents::top_memory(&c.proc_root, c.uid) {
        lines.push(format!("Using the most memory: {app}, about {mb} MB."));
    }
    if s.swap_total_mb > 0 {
        let swap = s.swap_total_mb.saturating_sub(s.swap_free_mb);
        if swap > 64 {
            lines.push(format!("{} has been moved to swap, which makes switching apps slower.", gb(swap)));
        }
    }
    lines.push(format!("Processor: {:.0} % busy{}.", s.cpu_busy,
                       if c.monitor.cpu_level >= Level::High { ", programs are waiting for it" } else { "" }));
    if s.psi_io >= 10.0 {
        lines.push(format!("The disk is busy (programs waited for it {:.0} % of the last 10 s).", s.psi_io));
    }
    if c.monitor.disk_low {
        lines.push(format!("Storage is low: {} free.", gb(s.disk_free_mb)));
    }
    if let Some(f) = c.monitor.memory_forecast() {
        if let Some(m) = f.minutes_left.filter(|_| f.confidence >= 0.6) {
            lines.push(format!("At this rate memory runs short in about {m:.0} minutes."));
        }
    }
    let calm = c.monitor.mem_level == Level::Normal && c.monitor.cpu_level <= Level::Elevated && !c.monitor.disk_low;
    lines.push(if calm {
        "Nothing is short right now. If it still feels slow, the app itself may be busy.".into()
    } else {
        format!("NexaOS: {}.", c.budget.why)
    });
    lines.join("\n")
}

/// Battery health from /sys (charge now, wear, cycles).
pub fn battery_answer(sys: &Path) -> String {
    let read = |p: &Path, f: &str| std::fs::read_to_string(p.join(f)).ok().map(|s| s.trim().to_string());
    let num = |p: &Path, f: &str| read(p, f).and_then(|v| v.parse::<f64>().ok());
    for e in std::fs::read_dir(sys.join("class/power_supply")).into_iter().flatten().flatten() {
        let p = e.path();
        if read(&p, "type").as_deref() != Some("Battery") {
            continue;
        }
        let mut out = vec![format!("Battery {} %, {}.", read(&p, "capacity").unwrap_or_else(|| "?".into()),
                                   read(&p, "status").unwrap_or_else(|| "state unknown".into()).to_lowercase())];
        let full = num(&p, "energy_full").or_else(|| num(&p, "charge_full"));
        let design = num(&p, "energy_full_design").or_else(|| num(&p, "charge_full_design"));
        if let (Some(f), Some(d)) = (full, design.filter(|d| *d > 0.0)) {
            out.push(format!("It holds {:.0} % of the charge it held when new.", 100.0 * f / d));
        }
        if let Some(cycles) = num(&p, "cycle_count").filter(|c| *c > 0.0) {
            out.push(format!("{cycles:.0} charge cycles so far."));
        }
        return out.join(" ");
    }
    "No battery found: this computer runs on mains power.".into()
}

fn sets_to_skip(store: &Store) -> Vec<BTreeSet<String>> {
    let h: Habits = store.load();
    let ws: Workspaces = store.load();
    h.dismissed.into_iter().map(|v| v.into_iter().collect())
        .chain(ws.0.iter().map(|w| w.apps.iter().map(|a| a.app.clone()).collect()))
        .collect()
}

pub fn handle(req: &Value, c: &mut Coordinator, cfg: &mut Config, store: &Store, cache: &mut AppCache) -> Option<(Value, Vec<Notice>)> {
    let home = home();
    let reply = match req["cmd"].as_str()? {
        "ask" if cfg.features.search => {
            let q: String = req["query"].as_str().unwrap_or("").chars().take(200).collect();
            let ws: Workspaces = store.load();
            let r = crate::search::ask(&q, cache.get(&home), &ws.0, &home);
            json!({"ok": true, "results": r})
        }
        "ask" => json!({"ok": false, "error": "ZeroSearch is switched off in Settings > Intelligence."}),
        "answer" => match req["topic"].as_str().unwrap_or("") {
            "slow" => json!({"ok": true, "text": slow_answer(c)}),
            "battery" => json!({"ok": true, "text": battery_answer(&crate::resources::Paths::real().sys_root)}),
            t @ ("crash" | "memory" | "network" | "stopped") if crate::magic::enabled(&home, "why") => {
                let f = crate::why::answer(t, c, &crate::why::Sources::real(home.clone()), now()).unwrap_or_default();
                json!({"ok": true, "text": crate::why::as_text(&f)})
            }
            _ => json!({"ok": false, "error": "No answer for that."}),
        },
        // Why (Magic Layer): findings marked proven or possible.
        "why" if crate::magic::enabled(&home, "why") => {
            let topic = req["topic"].as_str().unwrap_or("");
            match crate::why::answer(topic, c, &crate::why::Sources::real(home.clone()), now()) {
                Some(f) => {
                    let title = crate::why::TOPICS.iter().find(|(t, _)| *t == topic).map(|(_, n)| *n).unwrap_or("");
                    json!({"ok": true, "topic": topic, "title": title, "findings": f, "at": now()})
                }
                None => json!({"ok": false, "error": "Why can answer: slow, crash, memory, network, stopped."}),
            }
        }
        "why" => json!({"ok": false, "error": "Why is switched off in Settings > Magic Layer."}),
        // Rewind (Magic Layer): settings changes, and going back.
        "rewind" | "rewind-to" if !crate::magic::enabled(&home, "rewind") =>
            json!({"ok": false, "error": "Rewind is switched off in Settings > Magic Layer."}),
        "rewind" => match crate::REWIND.get() {
            Some(r) => {
                let mut g = r.lock().unwrap_or_else(|e| e.into_inner());
                g.scan(now()); // anything the watcher hasn't seen yet
                json!({"ok": true, "moments": g.list(req["limit"].as_u64().unwrap_or(50) as usize)})
            }
            None => json!({"ok": false, "error": "Rewind isn't running."}),
        },
        "rewind-to" => match crate::REWIND.get() {
            Some(r) => {
                let mut g = r.lock().unwrap_or_else(|e| e.into_inner());
                g.scan(now());
                match g.rewind_to(req["id"].as_u64().unwrap_or(0), now()) {
                    Ok(done) => json!({"ok": true, "restored": done.iter().map(|(l, a)| json!({"label": l, "apply": a})).collect::<Vec<_>>()}),
                    Err(e) => json!({"ok": false, "error": e}),
                }
            }
            None => json!({"ok": false, "error": "Rewind isn't running."}),
        },
        "workspaces" => {
            let ws: Workspaces = store.load();
            json!({"ok": true, "workspaces": ws.0})
        }
        "workspace-save" if cfg.features.workspaces => {
            let windows: Vec<Window> = serde_json::from_value(req["windows"].clone()).unwrap_or_default();
            match workspace::from_windows(req["name"].as_str().unwrap_or(""), &windows, req["folder"].as_str(),
                                          cache.get(&home), &home, now()) {
                Err(e) => json!({"ok": false, "error": e}),
                Ok((ws, problems)) => {
                    let mut all: Workspaces = store.load();
                    all.0.retain(|w| w.name != ws.name);
                    if all.0.len() >= workspace::MAX_WORKSPACES {
                        return Some((json!({"ok": false, "error": format!("You have {} workspaces already; delete one first.", workspace::MAX_WORKSPACES)}), vec![]));
                    }
                    all.0.push(ws.clone());
                    match store.save(&all) {
                        Ok(()) => {
                            c.log.push_back(crate::agents::Decision { at: now(), agent: "learning", what: format!("workspace saved: {}", ws.name),
                                                                      why: format!("{} apps", ws.apps.len()) });
                            json!({"ok": true, "workspace": ws, "problems": problems})
                        }
                        Err(e) => json!({"ok": false, "error": e.to_string()}),
                    }
                }
            }
        }
        "workspace-plan" if cfg.features.workspaces => {
            let name = req["name"].as_str().unwrap_or("");
            let mut all: Workspaces = store.load();
            match all.0.iter_mut().find(|w| w.name == name) {
                None => json!({"ok": false, "error": format!("No workspace called \u{201c}{name}\u{201d}.")}),
                Some(ws) => {
                    ws.used = now();
                    let (launches, problems) = workspace::plan(ws, cache.get(&home), &home);
                    let _ = store.save(&all);
                    json!({"ok": true, "launches": launches, "problems": problems})
                }
            }
        }
        "workspace-delete" => {
            let mut all: Workspaces = store.load();
            let before = all.0.len();
            all.0.retain(|w| Some(w.name.as_str()) != req["name"].as_str());
            json!({"ok": before != all.0.len() && store.save(&all).is_ok()})
        }
        "workspace-save" | "workspace-plan" => json!({"ok": false, "error": "Workspaces are switched off in Settings > Intelligence."}),
        "suggestions" => {
            if !cfg.features.habits {
                json!({"ok": true, "suggestions": [], "note": "Habit learning is off (Settings > Intelligence > Learning)."})
            } else {
                let apps = cache.get(&home).clone();
                let list = habit::suggestions(&habit::launches(&habit::journal_path(&home)), &sets_to_skip(store));
                let named: Vec<Value> = list.iter().map(|s| json!({
                    "apps": s.apps, "names": s.apps.iter().map(|a| apps.get(a).map(|x| x.name.clone()).unwrap_or_else(|| a.clone())).collect::<Vec<_>>(),
                    "days": s.days, "confidence": s.confidence, "why": s.why})).collect();
                json!({"ok": true, "suggestions": named})
            }
        }
        "suggestion-dismiss" => {
            let mut set: Vec<String> = req["apps"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).take(8).collect()).unwrap_or_default();
            set.sort();
            let mut h: Habits = store.load();
            if !set.is_empty() && !h.dismissed.contains(&set) {
                h.dismissed.push(set);
            }
            json!({"ok": store.save(&h).is_ok()})
        }
        "forget-habits" => {
            // Only MicroBrain's part; Patterns' own journal is cleared in Settings > Privacy.
            json!({"ok": store.forget::<Habits>().is_ok()})
        }
        _ => return None,
    };
    Some((reply, Vec::new()))
}

/// The Learning agent: once an hour (when learning is on), looks for apps
/// opened together and offers each new set once.
pub struct Learning {
    dir: PathBuf,
    home: PathBuf,
    checked_at: u64,
}

impl Learning {
    pub fn new(dir: PathBuf, home: PathBuf) -> Learning {
        Learning { dir, home, checked_at: 0 }
    }
}

impl Agent for Learning {
    fn name(&self) -> &'static str {
        "learning"
    }
    fn duty(&self) -> &'static str {
        "Finds apps you open together (when learning is on) and offers to save them as a workspace"
    }
    fn status(&self) -> Value {
        json!({"last_check": self.checked_at})
    }
    fn handle(&mut self, ev: &Event, ctx: &mut Ctx) {
        if !matches!(ev, Event::Minute) || ctx.now.saturating_sub(self.checked_at) < 3600 {
            return;
        }
        self.checked_at = ctx.now;
        let store = Store { dir: self.dir.clone() };
        let cfg: Config = store.load();
        if !cfg.features.habits || ctx.monitor.mem_level >= Level::High {
            return; // off, or the computer needs its memory for you
        }
        let found = habit::suggestions(&habit::launches(&habit::journal_path(&self.home)), &sets_to_skip(&store));
        let mut h: Habits = store.load();
        let apps = crate::apps::index(&self.home);
        for s in found.into_iter().filter(|s| s.confidence >= 0.4) {
            if h.offered.contains(&s.apps) {
                continue;
            }
            let names: Vec<String> = s.apps.iter().map(|a| apps.get(a).map(|x| x.name.clone()).unwrap_or_else(|| a.clone())).collect();
            ctx.decide("learning", format!("suggested a workspace: {}", names.join(", ")), s.why.clone());
            ctx.notify(&format!("workspace:{}", s.apps.join(",")), "Save these as a workspace?",
                       format!("{} are {}. Settings > Intelligence > Workspaces saves them as one.", names.join(", "), s.why));
            h.offered.push(s.apps);
            break; // one idea at a time
        }
        let _ = store.save(&h);
    }
}
