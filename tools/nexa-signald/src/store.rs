//! What NexaOS's notifications are and what shows: ids, replacing,
//! timeouts and holds, Quiet, muted apps, critical ones, the history.
//! No I/O here: the daemon feeds it the clock.

use crate::text;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const HISTORY: usize = 50;
pub const MAX_ACTIVE: usize = 64;
pub const MAX_ACTIONS: usize = 4;
/// After a hold ends, a Signal stays at least this long.
pub const AFTER_HOLD: Duration = Duration::from_millis(1500);

/// Why a Signal went away (the spec's NotificationClosed reasons).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// seconds; 0 = until dismissed
    pub timeout_s: u32,
    pub muted: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { timeout_s: 6, muted: vec![] }
    }
}

impl Prefs {
    pub fn from_json(v: &Value) -> Prefs {
        let mut p = Prefs::default();
        if let Some(t) = v.get("timeout").and_then(Value::as_u64).filter(|t| *t <= 60) {
            p.timeout_s = t as u32;
        }
        if let Some(list) = v.get("muted_apps").and_then(Value::as_array) {
            p.muted = list.iter().filter_map(Value::as_str).take(100).map(String::from).collect();
        }
        p
    }
}

/// What a Notify call brought, before cleaning.
#[derive(Debug, Clone, Default)]
pub struct Incoming {
    pub app: String,
    pub replaces: u32,
    pub icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<String>,
    pub urgency: u8,
    pub desktop: String,
    pub timeout_ms: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub id: u32,
    pub app: String,
    pub desktop: String,
    pub icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<(String, String)>,
    pub critical: bool,
    pub at: u64,
    /// shown as a pop-up (not muted, not held back by Quiet)
    pub popup: bool,
    total: Option<Duration>,
    deadline: Option<Instant>,
    /// while held: what was left
    left: Option<Duration>,
}

impl Signal {
    pub fn left(&self, now: Instant) -> Option<Duration> {
        self.left.or_else(|| self.deadline.map(|d| d.saturating_duration_since(now)))
    }
}

#[derive(Debug, Default)]
pub struct Store {
    pub active: Vec<Signal>,
    pub history: VecDeque<Signal>,
    pub quiet: bool,
    pub held: bool,
    pub prefs: Prefs,
    /// the mood of the layer on screen (Layer moods): set before each Notify
    pub mood: crate::layer::Mood,
    next: u32,
}

fn clean_id(s: &str, max: usize) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/')).take(max).collect()
}

impl Store {
    pub fn new(prefs: Prefs) -> Store {
        Store { prefs, next: 1, ..Default::default() }
    }

    fn allocate(&mut self) -> u32 {
        let id = self.next.max(1);
        self.next = id.checked_add(1).unwrap_or(1);
        id
    }

    /// A Notify call: the id it gets (the replaced one's, when replacing).
    /// Also returns any Signal pushed out because too many are open.
    pub fn notify(&mut self, n: Incoming, now: Instant) -> (u32, Vec<(u32, Reason)>) {
        let app = text::line(&n.app, 64);
        let critical = n.urgency >= 2;
        let mut actions = vec![];
        for pair in n.actions.chunks(2).take(MAX_ACTIONS) {
            if let [key, label] = pair {
                let key: String = key.chars().filter(|c| !c.is_control()).take(64).collect();
                let label = text::line(label, 40);
                if !key.is_empty() && !(label.is_empty() && key != "default") {
                    actions.push((key, label));
                }
            }
        }
        let total = if critical {
            None
        } else {
            match n.timeout_ms {
                t if t < 0 => (self.prefs.timeout_s > 0).then(|| Duration::from_secs(self.prefs.timeout_s as u64)),
                0 => None,
                t => Some(Duration::from_millis(t.min(10 * 60 * 1000) as u64)),
            }
        };
        let replacing = n.replaces != 0 && self.active.iter().any(|s| s.id == n.replaces);
        let id = if replacing { n.replaces } else { self.allocate() };
        let popup = critical
            || (!self.quiet && !self.prefs.muted.iter().any(|m| *m == app) && !self.mood.quiet && !self.mood.mutes(&app));
        let held = self.held && popup;
        let sig = Signal {
            id,
            desktop: clean_id(&n.desktop, 128),
            icon: n.icon.chars().filter(|c| !c.is_control()).take(512).collect(),
            summary: text::line(&n.summary, 200),
            body: text::body(&n.body, 600),
            app,
            actions,
            critical,
            at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            popup,
            total,
            deadline: if held { None } else { total.map(|t| now + t) },
            left: if held { total } else { None },
        };
        self.history.retain(|h| h.id != id);
        self.history.push_front(sig.clone());
        self.history.truncate(HISTORY);
        let mut gone = vec![];
        if replacing {
            if let Some(s) = self.active.iter_mut().find(|s| s.id == id) {
                *s = sig;
            }
        } else {
            self.active.insert(0, sig);
            while self.active.len() > MAX_ACTIVE {
                let old = self.active.pop().unwrap();
                gone.push((old.id, Reason::Dismissed));
            }
        }
        (id, gone)
    }

    pub fn close(&mut self, id: u32, why: Reason) -> Option<(u32, Reason)> {
        let i = self.active.iter().position(|s| s.id == id)?;
        self.active.remove(i);
        Some((id, why))
    }

    pub fn dismiss_all(&mut self) -> Vec<(u32, Reason)> {
        self.active.drain(..).map(|s| (s.id, Reason::Dismissed)).collect()
    }

    /// The person chose an action: the key to send back, and the Signal
    /// closes. "default" is a click on the Signal itself, even when the app
    /// didn't name it (then nothing is sent, it just closes).
    /// The desktop entry of a Signal still showing ("" if none).
    pub fn desktop_of(&self, id: u32) -> Option<String> {
        self.active.iter().find(|s| s.id == id).map(|s| s.desktop.clone())
    }

    pub fn invoke(&mut self, id: u32, key: &str) -> Option<(Option<String>, (u32, Reason))> {
        let s = self.active.iter().find(|s| s.id == id)?;
        let send = s.actions.iter().find(|(k, _)| k == key).map(|(k, _)| k.clone());
        if send.is_none() && key != "default" {
            return None;
        }
        let closed = self.close(id, Reason::Dismissed)?;
        Some((send, closed))
    }

    /// The pointer is on the Signals: every timer waits.
    pub fn hold(&mut self, on: bool, now: Instant) {
        if on == self.held {
            return;
        }
        self.held = on;
        for s in &mut self.active {
            if on {
                s.left = s.deadline.map(|d| d.saturating_duration_since(now));
                s.deadline = None;
            } else if let Some(left) = s.left.take() {
                s.deadline = Some(now + left.max(AFTER_HOLD));
            }
        }
    }

    pub fn expire(&mut self, now: Instant) -> Vec<(u32, Reason)> {
        let (gone, keep): (Vec<Signal>, Vec<Signal>) = self.active.drain(..).partition(|s| s.deadline.is_some_and(|d| d <= now));
        self.active = keep;
        gone.into_iter().map(|s| (s.id, Reason::Expired)).collect()
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.active.iter().filter_map(|s| s.deadline).min()
    }

    /// Quiet on: what shows now is held back too (critical ones stay).
    /// Quiet off: nothing held back comes back (as mako did).
    pub fn set_quiet(&mut self, on: bool) {
        self.quiet = on;
        if on {
            for s in &mut self.active {
                s.popup = s.popup && s.critical;
            }
        }
    }

    pub fn set_prefs(&mut self, p: Prefs) {
        for s in &mut self.active {
            if p.muted.iter().any(|m| *m == s.app) && !s.critical {
                s.popup = false;
            }
        }
        self.prefs = p;
    }

    /// For nexa-signals: what shows, newest first.
    pub fn state(&self, now: Instant) -> Value {
        let ms = |d: Option<Duration>| d.map(|d| d.as_millis() as u64);
        let signals: Vec<Value> = self.active.iter().filter(|s| s.popup).map(|s| json!({
            "id": s.id, "app": s.app, "desktop": s.desktop, "icon": s.icon, "summary": s.summary, "body": s.body,
            "actions": s.actions.iter().map(|(k, l)| json!([k, l])).collect::<Vec<_>>(), "critical": s.critical,
            "left_ms": ms(s.left(now)), "total_ms": ms(s.total),
        })).collect();
        json!({"event": "state", "quiet": self.quiet, "held": self.held, "signals": signals})
    }

    /// makoctl's shape, so Deck, the bar and Settings read it unchanged.
    pub fn mako_json<'a>(list: impl Iterator<Item = &'a Signal>) -> Value {
        let items: Vec<Value> = list.map(|s| json!({
            "app-name": {"type": "s", "data": s.app}, "summary": {"type": "s", "data": s.summary},
            "body": {"type": "s", "data": s.body}, "id": {"type": "u", "data": s.id},
            "urgency": {"type": "y", "data": if s.critical { 2 } else { 1 }},
        })).collect();
        json!({"type": "aa{sv}", "data": [items]})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(app: &str, summary: &str) -> Incoming {
        Incoming { app: app.into(), summary: summary.into(), timeout_ms: -1, ..Default::default() }
    }

    #[test]
    fn ids_replace_and_history() {
        let t = Instant::now();
        let mut s = Store::new(Prefs::default());
        let (a, _) = s.notify(n("Files", "Copied"), t);
        let (b, _) = s.notify(n("Music", "Playing"), t);
        assert_eq!((a, b), (1, 2));
        let (c, _) = s.notify(Incoming { replaces: a, ..n("Files", "Copied 3") }, t);
        assert_eq!(c, a, "replacing keeps the id");
        assert_eq!(s.active.len(), 2);
        assert_eq!(s.active.iter().find(|x| x.id == a).unwrap().summary, "Copied 3");
        let (d, _) = s.notify(Incoming { replaces: 99, ..n("X", "y") }, t);
        assert_eq!(d, 3, "an unknown replaces_id gets a new one");
        assert_eq!(s.history.len(), 3, "a replaced one is in the history once");
        for i in 0..60 {
            s.notify(n("Spam", &i.to_string()), t);
        }
        assert_eq!(s.history.len(), HISTORY);
        assert!(s.active.len() <= MAX_ACTIVE);
    }

    #[test]
    fn timeouts_and_holds() {
        let t = Instant::now();
        let mut s = Store::new(Prefs { timeout_s: 6, muted: vec![] });
        let (def, _) = s.notify(n("A", "default"), t);
        let (own, _) = s.notify(Incoming { timeout_ms: 2000, ..n("A", "own") }, t);
        let (never, _) = s.notify(Incoming { timeout_ms: 0, ..n("A", "never") }, t);
        let (crit, _) = s.notify(Incoming { urgency: 2, timeout_ms: 1000, ..n("A", "critical") }, t);
        assert_eq!(s.expire(t + Duration::from_millis(2500)), vec![(own, Reason::Expired)]);
        s.hold(true, t + Duration::from_secs(3));
        assert!(s.expire(t + Duration::from_secs(60)).is_empty(), "nothing expires while held");
        assert_eq!(s.active.iter().find(|x| x.id == def).unwrap().left(t).unwrap(), Duration::from_secs(3));
        s.hold(false, t + Duration::from_secs(60));
        assert!(s.expire(t + Duration::from_secs(62)).is_empty(), "the 3 s left run after the hold");
        assert_eq!(s.expire(t + Duration::from_secs(64)), vec![(def, Reason::Expired)]);
        let ids: Vec<u32> = s.active.iter().map(|x| x.id).collect();
        assert!(ids.contains(&never) && ids.contains(&crit), "timeout 0 and critical stay");
        assert_eq!(s.next_deadline(), None);
        let mut z = Store::new(Prefs { timeout_s: 0, muted: vec![] });
        z.notify(n("A", "b"), t);
        assert_eq!(z.next_deadline(), None, "Settings' 0: until dismissed");
        let mut h = Store::new(Prefs::default());
        h.hold(true, t);
        h.notify(n("A", "arrives while held"), t);
        assert!(h.expire(t + Duration::from_secs(30)).is_empty());
    }

    #[test]
    fn layer_moods_hold_back() {
        use crate::layer::Mood;
        let t = Instant::now();
        let mut s = Store::new(Prefs::default());
        s.mood = Mood { layer: 3, name: "Work".into(), quiet: true, muted: vec![] };
        s.notify(n("Chat", "lunch?"), t);
        s.notify(Incoming { urgency: 2, ..n("Battery", "5% left") }, t);
        let shown: Vec<String> = s.state(t)["signals"].as_array().unwrap().iter().map(|x| x["app"].as_str().unwrap().to_string()).collect();
        assert_eq!(shown, vec!["Battery"], "a quiet layer: only the important one pops up");
        assert_eq!(s.history.len(), 2, "the other is kept in the history");
        s.mood = Mood { layer: 2, muted: vec!["chat".into()], ..Default::default() };
        s.notify(n("Chat", "again"), t);
        s.notify(n("Files", "copied"), t);
        let shown: Vec<String> = s.state(t)["signals"].as_array().unwrap().iter().map(|x| x["app"].as_str().unwrap().to_string()).collect();
        assert!(shown.contains(&"Files".to_string()) && !shown.contains(&"Chat".to_string()), "an app muted on this layer: {shown:?}");
        s.mood = Mood::default();
        s.notify(n("Chat", "on layer 1"), t);
        assert!(s.state(t)["signals"].as_array().unwrap().iter().any(|x| x["summary"] == "on layer 1"), "another layer: as before");
    }

    #[test]
    fn quiet_muted_critical() {
        let t = Instant::now();
        let mut s = Store::new(Prefs { timeout_s: 6, muted: vec!["Chat".into()] });
        s.notify(n("Files", "shown"), t);
        s.notify(n("Chat", "muted"), t);
        let st = s.state(t);
        assert_eq!(st["signals"].as_array().unwrap().len(), 1, "a muted app doesn't pop up");
        assert_eq!(s.active.len(), 2, "but it is in the list");
        s.set_quiet(true);
        assert_eq!(s.state(t)["signals"].as_array().unwrap().len(), 0, "Quiet holds back what showed");
        s.notify(n("Files", "during quiet"), t);
        s.notify(Incoming { urgency: 2, ..n("Battery", "Battery at 3 %") }, t);
        let shown = s.state(t)["signals"].as_array().unwrap().clone();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0]["summary"], "Battery at 3 %");
        assert_eq!(shown[0]["critical"], true);
        assert_eq!(shown[0]["left_ms"], Value::Null);
        s.set_quiet(false);
        assert_eq!(s.state(t)["quiet"], false);
        s.set_prefs(Prefs { timeout_s: 6, muted: vec!["Files".into(), "Battery".into()] });
        assert_eq!(s.state(t)["signals"].as_array().unwrap().len(), 1, "muting later hides it, but never a critical one");
    }

    #[test]
    fn actions_and_invoke() {
        let t = Instant::now();
        let mut s = Store::new(Prefs::default());
        let acts = ["default", "", "reply", "Reply", "mark", "Mark read", "x", "4th", "y", "5th", "odd"].map(String::from).to_vec();
        let (id, _) = s.notify(Incoming { actions: acts, ..n("Mail", "New") }, t);
        let sig = s.active[0].clone();
        assert_eq!(sig.actions.len(), 4, "at most 4, a dangling key dropped");
        assert_eq!(sig.actions[0], ("default".into(), "".into()));
        assert_eq!(s.invoke(id, "nope"), None, "an action that isn't there does nothing");
        assert_eq!(s.invoke(id, "reply"), Some((Some("reply".into()), (id, Reason::Dismissed))));
        assert!(s.active.is_empty());
        let (id2, _) = s.notify(n("Files", "no actions"), t);
        assert_eq!(s.invoke(id2, "default"), Some((None, (id2, Reason::Dismissed))), "a click closes it, nothing to send");
        assert_eq!(s.close(id2, Reason::Closed), None);
    }

    #[test]
    fn cleaned_and_shaped() {
        let t = Instant::now();
        let mut s = Store::new(Prefs::default());
        s.notify(Incoming { body: "<b>Hi</b> &amp; bye\u{7}".into(), desktop: "nexa-files; rm -rf".into(), ..n("Files\n", "S") }, t);
        let x = &s.active[0];
        assert_eq!((x.app.as_str(), x.body.as_str(), x.desktop.as_str()), ("Files", "Hi & bye", "nexa-filesrm-rf"));
        let m = Store::mako_json(s.history.iter());
        assert_eq!(m["data"][0][0]["app-name"]["data"], "Files");
        assert_eq!(m["data"][0][0]["id"]["data"], 1);
        assert_eq!(s.dismiss_all(), vec![(1, Reason::Dismissed)]);
        let p = Prefs::from_json(&json!({"timeout": 99, "muted_apps": ["A", 3]}));
        assert_eq!(p, Prefs { timeout_s: 6, muted: vec!["A".into()] });
    }
}
