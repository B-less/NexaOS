//! NanoAgents: small specialised parts of MicroBrain, in one process.
//!
//! Each agent looks at the events that concern it and makes decisions
//! (recorded with the reason, shown on Settings > Intelligence > Agents)
//! and, where useful, a notice. Agents never run commands: they queue
//! jobs, change the budget's inputs, or ask the action engine, which
//! checks what's allowed. No agent loops on its own; they only run when
//! an event arrives.

use crate::config::Profile;
use crate::pulse::{self, Budget, Mode};
use crate::resources::{Change, Level, Monitor, Sample};
use crate::scheduler::{Kind, Scheduler};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum Event {
    Sample(Sample),
    /// Levels changed (computed by the coordinator from a sample).
    Levels(Vec<Change>),
    /// The budget changed: (before, after).
    Budget(Box<Budget>, Box<Budget>),
    JobDone { id: u64, name: String, ok: bool, note: String },
    /// Once a minute while running, for slow housekeeping.
    Minute,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Decision {
    pub at: u64,
    pub agent: &'static str,
    pub what: String,
    pub why: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Notice {
    /// Notices with the same key are not repeated within the quiet period.
    pub key: String,
    pub title: String,
    pub body: String,
}

pub struct Ctx<'a> {
    pub now: u64,
    pub monitor: &'a Monitor,
    pub scheduler: &'a mut Scheduler,
    pub decisions: Vec<Decision>,
    pub notices: Vec<Notice>,
    pub proc_root: &'a Path,
    pub uid: u32,
}

impl Ctx<'_> {
    pub fn decide(&mut self, agent: &'static str, what: impl Into<String>, why: impl Into<String>) {
        self.decisions.push(Decision { at: self.now, agent, what: what.into(), why: why.into() });
    }
    pub fn notify(&mut self, key: &str, title: impl Into<String>, body: impl Into<String>) {
        self.notices.push(Notice { key: key.into(), title: title.into(), body: body.into() });
    }
}

pub trait Agent: Send {
    fn name(&self) -> &'static str;
    fn duty(&self) -> &'static str;
    fn handle(&mut self, ev: &Event, ctx: &mut Ctx);
    fn status(&self) -> Value {
        json!({})
    }
}

/// The app using the most memory among this user's processes, by name, MB.
pub fn top_memory(proc_root: &Path, uid: u32) -> Option<(String, u64)> {
    use std::os::unix::fs::MetadataExt;
    let mut by_name: HashMap<String, u64> = HashMap::new();
    for e in std::fs::read_dir(proc_root).ok()?.flatten() {
        if !e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if e.metadata().map(|m| m.uid() != uid).unwrap_or(true) {
            continue;
        }
        let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default().trim().to_string();
        let rss_kb = std::fs::read_to_string(e.path().join("status")).ok().and_then(|s| {
            s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok())
        });
        if let (false, Some(kb)) = (comm.is_empty(), rss_kb) {
            *by_name.entry(comm).or_default() += kb / 1024;
        }
    }
    by_name.into_iter().max_by_key(|(_, mb)| *mb)
}

// ---- Performance agent -----------------------------------------------------

#[derive(Default)]
pub struct Performance {
    warned_forecast_at: u64,
    last_level: Option<Level>,
}

impl Agent for Performance {
    fn name(&self) -> &'static str {
        "performance"
    }
    fn duty(&self) -> &'static str {
        "Watches memory, processor and disk pressure; explains slowdowns and holds background work back"
    }
    fn handle(&mut self, ev: &Event, ctx: &mut Ctx) {
        match ev {
            Event::Levels(changes) => {
                for c in changes {
                    match c {
                        Change::Memory(level) if *level >= Level::High => {
                            let culprit = top_memory(ctx.proc_root, ctx.uid)
                                .map(|(app, mb)| format!(" {app} is using about {mb} MB."))
                                .unwrap_or_default();
                            ctx.decide(self.name(), "memory is short: background work paused", format!("memory pressure reached {level:?}.{culprit}"));
                            ctx.notify("memory-short", "Memory is running short",
                                       format!("NexaOS paused its own background work to keep things responsive.{culprit}"));
                            self.last_level = Some(*level);
                        }
                        Change::Memory(Level::Normal) if self.last_level.is_some_and(|l| l >= Level::High) => {
                            ctx.decide(self.name(), "memory pressure eased", "background work may run again");
                            self.last_level = Some(Level::Normal);
                        }
                        Change::Cpu(level) if *level >= Level::Critical => {
                            ctx.decide(self.name(), "the processor is saturated", format!("CPU pressure {level:?}: background work waits"));
                        }
                        Change::DiskLow(true) => {
                            let free = ctx.monitor.last.as_ref().map(|s| s.disk_free_mb).unwrap_or(0);
                            ctx.decide(self.name(), "storage space is low", format!("{:.1} GB left in your home folder", free as f64 / 1024.0));
                            ctx.notify("disk-low", "Storage space is running low",
                                       format!("{:.1} GB left. Settings > Intelligence > Storage shows what can be cleared safely.", free as f64 / 1024.0));
                        }
                        _ => {}
                    }
                }
            }
            Event::Sample(_) => {
                // A confident forecast that memory runs out soon, before it's short.
                if ctx.monitor.mem_level >= Level::High || ctx.now.saturating_sub(self.warned_forecast_at) < 1800 {
                    return;
                }
                if let Some(f) = ctx.monitor.memory_forecast() {
                    if let Some(min) = f.minutes_left.filter(|m| *m < 10.0 && f.confidence >= 0.75) {
                        self.warned_forecast_at = ctx.now;
                        ctx.decide(self.name(), "memory forecast: short soon",
                                   format!("free memory falls {:.0} MB a minute; about {:.0} minutes left (confidence {:.0} %)",
                                           -f.per_minute, min, f.confidence * 100.0));
                        ctx.notify("memory-forecast", "Memory is filling up",
                                   format!("At this rate it runs short in about {:.0} minutes. Closing apps you don't need will help.", min.max(1.0)));
                    }
                }
            }
            _ => {}
        }
    }
}

// ---- Maintenance agent -----------------------------------------------------

#[derive(Default)]
pub struct Maintenance {
    trimmed_at: u64,
}

impl Agent for Maintenance {
    fn name(&self) -> &'static str {
        "maintenance"
    }
    fn duty(&self) -> &'static str {
        "Schedules NexaOS's own maintenance and postpones it while the computer is busy"
    }
    fn handle(&mut self, ev: &Event, ctx: &mut Ctx) {
        match ev {
            Event::Budget(before, after) => {
                let waiting: Vec<String> = ctx.scheduler.waiting().filter(|j| j.kind != Kind::Essential).map(|j| j.name.clone()).collect();
                if before.scans && !after.scans && !waiting.is_empty() {
                    ctx.decide(self.name(), format!("postponed: {}", waiting.join(", ")), after.why.clone());
                } else if !before.scans && after.scans && !waiting.is_empty() {
                    ctx.decide(self.name(), format!("resuming: {}", waiting.join(", ")), after.why.clone());
                }
            }
            Event::Minute if ctx.now.saturating_sub(self.trimmed_at) >= 6 * 3600 => {
                // Keep MicroBrain's own memory small: a real, cheap maintenance job.
                self.trimmed_at = ctx.now;
                ctx.scheduler.submit("trim-history", Kind::Maintenance, ctx.now);
            }
            Event::JobDone { name, ok, note, .. } => {
                ctx.decide(self.name(), format!("{name}: {}", if *ok { "done" } else { "failed" }), note.clone());
            }
            _ => {}
        }
    }
}

// ---- the coordinator -------------------------------------------------------

pub struct Coordinator {
    pub agents: Vec<Box<dyn Agent>>,
    pub monitor: Monitor,
    pub scheduler: Scheduler,
    pub budget: Budget,
    pub mode: Mode,
    pub profile: Profile,
    pub log: VecDeque<Decision>,
    log_cap: usize,
    noticed: HashMap<String, u64>,
    pub quiet_secs: u64,
    pub proc_root: PathBuf,
    pub uid: u32,
}

impl Coordinator {
    pub fn new(profile: Profile, mode: Mode, agents: Vec<Box<dyn Agent>>, proc_root: PathBuf, uid: u32) -> Coordinator {
        let calm = pulse::Conditions { memory: Level::Normal, cpu: Level::Normal, on_battery: false, battery: None };
        Coordinator {
            agents, monitor: Monitor::new(profile.history()), scheduler: Scheduler::new(32),
            budget: pulse::decide(mode, &calm, profile), mode, profile, log: VecDeque::new(), log_cap: profile.log_lines(),
            noticed: HashMap::new(), quiet_secs: 900, proc_root, uid,
        }
    }

    fn conditions(&self) -> pulse::Conditions {
        let last = self.monitor.last.as_ref();
        pulse::Conditions { memory: self.monitor.mem_level, cpu: self.monitor.cpu_level,
                            on_battery: last.is_some_and(|s| s.on_battery), battery: last.and_then(|s| s.battery) }
    }

    /// One event in; the notices to show (already rate-limited) out.
    pub fn handle(&mut self, ev: Event, now: u64) -> Vec<Notice> {
        let mut events = vec![ev.clone()];
        if let Event::JobDone { id, .. } = &ev {
            self.scheduler.finish(*id);
        }
        if let Event::Sample(s) = ev {
            let changes = self.monitor.push(s);
            if !changes.is_empty() {
                events.push(Event::Levels(changes));
            }
            let next = pulse::decide(self.mode, &self.conditions(), self.profile);
            if next != self.budget {
                let before = std::mem::replace(&mut self.budget, next.clone());
                events.push(Event::Budget(Box::new(before), Box::new(next)));
            }
        }
        let mut notices = Vec::new();
        for ev in &events {
            let mut ctx = Ctx { now, monitor: &self.monitor, scheduler: &mut self.scheduler,
                                decisions: Vec::new(), notices: Vec::new(), proc_root: &self.proc_root, uid: self.uid };
            for a in self.agents.iter_mut() {
                a.handle(ev, &mut ctx);
            }
            for d in ctx.decisions {
                if self.log.len() == self.log_cap {
                    self.log.pop_front();
                }
                self.log.push_back(d);
            }
            for n in ctx.notices {
                let last = self.noticed.get(&n.key).copied();
                if last.map_or(true, |t| now.saturating_sub(t) >= self.quiet_secs) {
                    self.noticed.insert(n.key.clone(), now);
                    notices.push(n);
                }
            }
        }
        notices
    }

    pub fn set_mode(&mut self, mode: Mode, now: u64) -> Vec<Notice> {
        self.mode = mode;
        let next = pulse::decide(mode, &self.conditions(), self.profile);
        if next == self.budget {
            return Vec::new();
        }
        let before = std::mem::replace(&mut self.budget, next.clone());
        let mut out = Vec::new();
        let mut ctx = Ctx { now, monitor: &self.monitor, scheduler: &mut self.scheduler,
                            decisions: Vec::new(), notices: Vec::new(), proc_root: &self.proc_root, uid: self.uid };
        let ev = Event::Budget(Box::new(before), Box::new(next));
        for a in self.agents.iter_mut() {
            a.handle(&ev, &mut ctx);
        }
        self.log.extend(ctx.decisions);
        while self.log.len() > self.log_cap {
            self.log.pop_front();
        }
        out.extend(ctx.notices);
        out
    }

    pub fn agents_status(&self) -> Value {
        Value::Array(self.agents.iter().map(|a| {
            let recent: Vec<&Decision> = self.log.iter().rev().filter(|d| d.agent == a.name()).take(5).collect();
            json!({"name": a.name(), "duty": a.duty(), "status": a.status(), "recent": recent})
        }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coordinator() -> Coordinator {
        let agents: Vec<Box<dyn Agent>> = vec![Box::<Performance>::default(), Box::<Maintenance>::default()];
        Coordinator::new(Profile::Medium, Mode::Auto, agents, std::env::temp_dir().join("no-such-proc"), 1000)
    }

    fn sample(at: u64, psi: f64, avail: u64) -> Sample {
        Sample { at, psi_mem: psi, mem_total_mb: 4000, mem_avail_mb: avail, disk_total_mb: 100_000, disk_free_mb: 50_000, ..Default::default() }
    }

    #[test]
    fn low_memory_defers_a_scan_and_it_resumes_afterwards() {
        let mut c = coordinator();
        c.scheduler.submit("storage-scan", Kind::StorageScan, 0);
        c.handle(Event::Sample(sample(1, 0.0, 3000)), 1);
        assert_eq!(c.scheduler.next(&c.budget).len(), 1, "calm: the scan runs");
        c.scheduler.finish(1);
        c.scheduler.submit("storage-scan", Kind::StorageScan, 2);
        let notices = c.handle(Event::Sample(sample(10, 40.0, 150)), 10);
        assert_eq!(c.budget.mode, Mode::LowResource);
        assert!(notices.iter().any(|n| n.key == "memory-short"), "{notices:?}");
        assert!(c.scheduler.next(&c.budget).is_empty(), "the scan waits");
        assert!(c.log.iter().any(|d| d.agent == "performance" && d.what.contains("paused")));
        assert!(c.log.iter().any(|d| d.agent == "maintenance" && d.what.contains("postponed: storage-scan")), "{:?}", c.log);
        for t in 11..20 {
            c.handle(Event::Sample(sample(t * 10, 0.0, 3000)), t * 10);
        }
        assert_eq!(c.budget.mode, Mode::Balanced);
        assert!(c.log.iter().any(|d| d.what.contains("resuming: storage-scan")));
        assert_eq!(c.scheduler.next(&c.budget).len(), 1, "and runs once memory is back");
    }

    #[test]
    fn notices_are_not_repeated_within_the_quiet_period() {
        let mut c = coordinator();
        let mut count = 0;
        for i in 0..20u64 {
            // Pressure up and down every few samples.
            let psi = if (i / 3) % 2 == 0 { 60.0 } else { 0.0 };
            count += c.handle(Event::Sample(sample(i * 10, psi, 2000)), i * 10).iter().filter(|n| n.key == "memory-short").count();
        }
        assert_eq!(count, 1, "one notice, not a storm");
    }

    #[test]
    fn a_chosen_mode_is_followed() {
        let mut c = coordinator();
        c.set_mode(Mode::BatterySaver, 5);
        assert_eq!(c.budget.mode, Mode::BatterySaver);
        assert!(!c.budget.scans);
        let status = c.agents_status();
        assert_eq!(status.as_array().unwrap().len(), 2);
    }
}
