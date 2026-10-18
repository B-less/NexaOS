//! What Nexa Patterns knows, and how it predicts the next app.
//!
//! Everything is a count that fades (half-life 14 days), so new habits take
//! over from old ones:
//! * time habits: opens per (app, 30-min slot, weekday/weekend, place);
//! * sequences: 1st- and 2nd-order Markov chains over app switches;
//! * a ranker: online logistic regression over those signals plus recency,
//!   overall use and dismissals. Its handful of weights learn, from what you
//!   actually open next, how much each signal matters for you.
//!
//! The model is a pure function of the records applied in order, so
//! forgetting is "remove the records, rebuild".

use crate::context::{self, Ctx, SLOTS};
use crate::journal::{Kind, Record};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const HALF_LIFE: f64 = 14.0 * 86400.0;
/// An app is suggested only after it has been opened this many times...
pub const MIN_SEEN: u32 = 5;
/// ...and only once there are this many days of history.
pub const MIN_DAYS: u64 = 3;
pub const FEATURES: usize = 7;
const LEARNING_RATE: f64 = 0.05;

/// A count that halves every HALF_LIFE seconds.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Decayed {
    pub v: f64,
    pub t: u64,
}

impl Decayed {
    pub fn at(&self, now: u64) -> f64 {
        self.v * 0.5f64.powf(now.saturating_sub(self.t) as f64 / HALF_LIFE)
    }

    pub fn add(&mut self, now: u64, x: f64) {
        self.v = self.at(now) + x;
        self.t = self.t.max(now);
    }
}

fn key(parts: &[&str]) -> String {
    parts.join("\t")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    time: HashMap<String, Decayed>,
    ctx_total: HashMap<String, Decayed>,
    app_total: HashMap<String, Decayed>,
    markov1: HashMap<String, Decayed>,
    m1_total: HashMap<String, Decayed>,
    markov2: HashMap<String, Decayed>,
    m2_total: HashMap<String, Decayed>,
    dismissed: HashMap<String, Decayed>,
    pub last_used: HashMap<String, u64>,
    pub seen: HashMap<String, u32>,
    pub first_ts: u64,
    pub last_ts: u64,
    pub weights: [f64; FEATURES],
    pub bias: f64,
    history: Vec<String>,
    pending_used: Option<(String, u64)>,
    opens: u64,
}

impl Default for Model {
    fn default() -> Model {
        Model {
            time: HashMap::new(), ctx_total: HashMap::new(), app_total: HashMap::new(), markov1: HashMap::new(),
            m1_total: HashMap::new(), markov2: HashMap::new(), m2_total: HashMap::new(), dismissed: HashMap::new(),
            last_used: HashMap::new(), seen: HashMap::new(), first_ts: 0, last_ts: 0,
            // Sensible priors, refined by learning: time and sequence matter most.
            weights: [4.0, 2.0, 4.0, 2.0, 1.5, 2.0, 3.0],
            bias: -3.0,
            history: Vec::new(), pending_used: None, opens: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Prediction {
    pub app: String,
    /// Chance this is the next app you open (0..1).
    pub p: f64,
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

impl Model {
    /// Opens of `app` near `slot` (neighbouring half-hours count half).
    fn time_count(&self, app: &str, ctx: &Ctx, any_place: bool, now: u64) -> (f64, f64) {
        let mut hits = 0.0;
        let mut total = 0.0;
        for (offset, w) in [(-1i16, 0.5), (0, 1.0), (1, 0.5)] {
            let slot = ((ctx.slot as i16 + offset).rem_euclid(SLOTS as i16)) as u8;
            let (s, we) = (slot.to_string(), ctx.weekend.to_string());
            if any_place {
                hits += w * self.time.get(&key(&[app, &s, &we, "*"])).map_or(0.0, |d| d.at(now));
                total += w * self.ctx_total.get(&key(&[&s, &we, "*"])).map_or(0.0, |d| d.at(now));
            } else {
                let place = ctx.place.to_string();
                hits += w * self.time.get(&key(&[app, &s, &we, &place])).map_or(0.0, |d| d.at(now));
                total += w * self.ctx_total.get(&key(&[&s, &we, &place])).map_or(0.0, |d| d.at(now));
            }
        }
        (hits, total)
    }

    /// The signals for "is `app` next?", each roughly 0..1.
    pub fn features(&self, app: &str, ctx: &Ctx, now: u64) -> [f64; FEATURES] {
        let (h, t) = self.time_count(app, ctx, false, now);
        let (ha, ta) = self.time_count(app, ctx, true, now);
        let prev1 = self.history.last().map(String::as_str).unwrap_or("");
        let m1 = self.markov1.get(&key(&[prev1, app])).map_or(0.0, |d| d.at(now))
            / (self.m1_total.get(prev1).map_or(0.0, |d| d.at(now)) + 1.0);
        let m2 = match self.history.len() {
            n if n >= 2 => {
                let (p2, p1) = (&self.history[n - 2], &self.history[n - 1]);
                self.markov2.get(&key(&[p2, p1, app])).map_or(0.0, |d| d.at(now))
                    / (self.m2_total.get(&key(&[p2, p1])).map_or(0.0, |d| d.at(now)) + 1.0)
            }
            _ => 0.0,
        };
        let recency = self.last_used.get(app).map_or(0.0, |t| (-(now.saturating_sub(*t) as f64) / 3600.0).exp());
        let all: f64 = self.app_total.values().map(|d| d.at(now)).sum();
        let freq = self.app_total.get(app).map_or(0.0, |d| d.at(now)) / (all + 1.0);
        let dismissed = -(self.dismissed.get(app).map_or(0.0, |d| d.at(now)) / 3.0).min(1.0);
        [h / (t + 1.0), ha / (ta + 1.0), m1, m2, recency, freq, dismissed]
    }

    fn score(&self, f: &[f64; FEATURES]) -> f64 {
        sigmoid(self.bias + f.iter().zip(self.weights.iter()).map(|(x, w)| x * w).sum::<f64>())
    }

    pub fn apps(&self) -> Vec<String> {
        let mut v: Vec<String> = self.seen.keys().cloned().collect();
        v.sort();
        v
    }

    /// Has it watched long enough to say anything?
    pub fn ready(&self) -> bool {
        self.last_ts.saturating_sub(self.first_ts) >= MIN_DAYS * 86400
    }

    /// The likeliest next apps, best first (only apps seen MIN_SEEN times).
    pub fn predict(&self, ctx: &Ctx, now: u64, k: usize, skip: &[String]) -> Vec<Prediction> {
        let mut v: Vec<Prediction> = self.seen.iter()
            .filter(|(app, n)| **n >= MIN_SEEN && !skip.contains(app))
            .map(|(app, _)| Prediction { app: app.clone(), p: self.score(&self.features(app, ctx, now)) })
            .collect();
        v.sort_by(|a, b| b.p.total_cmp(&a.p).then(a.app.cmp(&b.app)));
        v.truncate(k);
        v
    }

    /// Learn from what was actually opened: one SGD step of logistic
    /// regression for every candidate (1 for the app opened, 0 for the rest).
    fn train(&mut self, ctx: &Ctx, now: u64, actual: &str) {
        let candidates: Vec<String> = self.seen.iter().filter(|(_, n)| **n >= 2).map(|(a, _)| a.clone()).collect();
        if candidates.len() < 2 {
            return;
        }
        let examples: Vec<([f64; FEATURES], f64)> = candidates.iter()
            .map(|a| (self.features(a, ctx, now), if a == actual { 1.0 } else { 0.0 })).collect();
        for (f, y) in examples {
            let err = self.score(&f) - y;
            for (w, x) in self.weights.iter_mut().zip(f.iter()) {
                *w -= LEARNING_RATE * (err * x + 0.0005 * *w);
            }
            self.bias -= LEARNING_RATE * err;
        }
    }

    pub fn apply(&mut self, r: &Record) {
        let (slot, weekend, _) = context::local(r.ts);
        let ctx = Ctx { slot, weekend, place: r.place, plugged: r.plugged };
        let now = r.ts;
        if self.first_ts == 0 {
            self.first_ts = now;
        }
        self.last_ts = self.last_ts.max(now);
        match r.kind {
            Kind::Dismiss => self.dismissed.entry(r.app.clone()).or_default().add(now, 1.0),
            Kind::Used => self.pending_used = Some((r.app.clone(), now)),
            Kind::Launch | Kind::Switch => {
                if self.history.last() == Some(&r.app) {
                    return;
                }
                self.opens += 1;
                if self.opens > 20 {
                    self.train(&ctx, now, &r.app);
                }
                // Opened from a suggestion: counts half (no self-fulfilling loop).
                let w = match self.pending_used.take() {
                    Some((app, t)) if app == r.app && now.saturating_sub(t) < 60 => 0.5,
                    _ => 1.0,
                };
                let (s, we, place) = (slot.to_string(), weekend.to_string(), r.place.to_string());
                for p in [place.as_str(), "*"] {
                    self.time.entry(key(&[&r.app, &s, &we, p])).or_default().add(now, w);
                    self.ctx_total.entry(key(&[&s, &we, p])).or_default().add(now, w);
                }
                self.app_total.entry(r.app.clone()).or_default().add(now, w);
                let n = self.history.len();
                if n >= 1 {
                    let p1 = self.history[n - 1].clone();
                    self.markov1.entry(key(&[&p1, &r.app])).or_default().add(now, w);
                    self.m1_total.entry(p1.clone()).or_default().add(now, w);
                    if n >= 2 {
                        let p2 = self.history[n - 2].clone();
                        self.markov2.entry(key(&[&p2, &p1, &r.app])).or_default().add(now, w);
                        self.m2_total.entry(key(&[&p2, &p1])).or_default().add(now, w);
                    }
                }
                self.last_used.insert(r.app.clone(), now);
                *self.seen.entry(r.app.clone()).or_default() += 1;
                self.history.push(r.app.clone());
                if self.history.len() > 2 {
                    self.history.remove(0);
                }
            }
        }
    }

    /// Drop everything known about one app.
    pub fn forget_app(&mut self, app: &str) {
        let mentions = |k: &String| k.split('\t').any(|p| p == app);
        for m in [&mut self.time, &mut self.markov1, &mut self.markov2, &mut self.m2_total] {
            m.retain(|k, _| !mentions(k));
        }
        for m in [&mut self.app_total, &mut self.m1_total, &mut self.dismissed] {
            m.remove(app);
        }
        self.last_used.remove(app);
        self.seen.remove(app);
        self.history.retain(|a| a != app);
    }

    pub fn previous(&self) -> Option<&str> {
        self.history.last().map(String::as_str)
    }
}

/// Why an app is suggested, from the raw records of the last 14 days.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Why {
    /// Opened around `around` on `days` of the last `of` weekdays/weekends.
    Time { days: usize, of: usize, around: String, weekend: bool },
    /// Opened after `after` `times` times.
    After { after: String, times: usize },
    /// Simply used a lot lately.
    Often { times: usize },
}

pub fn explain(records: &[Record], app: &str, previous: Option<&str>, now: u64) -> Why {
    let (now_slot, now_we, now_min) = context::local(now);
    let recent: Vec<&Record> = records.iter().filter(|r| r.kind.is_open() && now.saturating_sub(r.ts) < 14 * 86400).collect();
    let day = |r: &Record| (r.ts as i64 + context::utc_offset(r.ts)) / 86400;
    let mut active_days = std::collections::BTreeSet::new();
    let mut hit_days = std::collections::BTreeSet::new();
    for r in &recent {
        let (slot, we, _) = context::local(r.ts);
        if we == now_we {
            active_days.insert(day(r));
            if r.app == app && context::slot_distance(slot, now_slot) <= 1 {
                hit_days.insert(day(r));
            }
        }
    }
    let mut after = 0;
    if let Some(prev) = previous {
        for w in recent.windows(2) {
            if w[0].app == prev && w[1].app == app {
                after += 1;
            }
        }
    }
    let time_share = hit_days.len() as f64 / active_days.len().max(1) as f64;
    if after >= 3 && (after as f64 / 14.0) > time_share {
        Why::After { after: previous.unwrap_or_default().to_string(), times: after }
    } else if hit_days.len() >= 2 {
        Why::Time { days: hit_days.len(), of: active_days.len(), around: context::hhmm(now_min / 30 * 30), weekend: now_we }
    } else {
        Why::Often { times: recent.iter().filter(|r| r.app == app).count() }
    }
}

impl Why {
    /// In words, with the app's display name filled in by the caller.
    pub fn words(&self, name: &str) -> String {
        match self {
            Why::Time { days, of, around, weekend } => format!(
                "You opened {name} around {around} on {days} of the last {of} {}.", if *weekend { "weekend days" } else { "weekdays" }),
            Why::After { after, times } => format!("You often open {name} after {after} ({times} times in the last two weeks)."),
            Why::Often { times } => format!("You've used {name} a lot lately ({times} times in the last two weeks)."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(ts: u64, app: &str) -> Record {
        Record { ts, kind: Kind::Launch, app: app.into(), place: 1, plugged: true }
    }

    #[test]
    fn counts_fade() {
        let mut d = Decayed::default();
        d.add(0, 1.0);
        assert!((d.at(HALF_LIFE as u64) - 0.5).abs() < 1e-9);
        d.add(HALF_LIFE as u64, 1.0);
        assert!((d.at(HALF_LIFE as u64) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn sequences_and_cold_start() {
        let mut m = Model::default();
        let mut t = 1_700_000_000;
        for _ in 0..8 {
            for app in ["foot", "nexa-files", "nexa-edit"] {
                m.apply(&open(t, app));
                t += 600;
            }
        }
        assert!(!m.ready(), "less than 3 days of history");
        m.apply(&open(t + 4 * 86400, "foot"));
        assert!(m.ready());
        let ctx = Ctx { place: 1, ..Default::default() };
        let p = m.predict(&ctx, t + 4 * 86400, 3, &[]);
        assert_eq!(p[0].app, "nexa-files", "after foot comes files: {p:?}");
        assert!(m.predict(&ctx, t, 3, &["nexa-files".into()]).iter().all(|x| x.app != "nexa-files"), "skip list honoured");
        m.forget_app("nexa-files");
        assert!(m.apps() == vec!["foot", "nexa-edit"] && m.predict(&ctx, t, 3, &[]).iter().all(|x| x.app != "nexa-files"));
    }

    #[test]
    fn dismissals_push_down() {
        let mut m = Model::default();
        let t0 = 1_700_000_000;
        for i in 0..10 {
            m.apply(&open(t0 + i * 400, if i % 2 == 0 { "a" } else { "b" }));
        }
        let ctx = Ctx { place: 1, ..Default::default() };
        let before = m.predict(&ctx, t0 + 5000, 3, &[]).into_iter().find(|p| p.app == "a").unwrap().p;
        for i in 0..3 {
            m.apply(&Record { kind: Kind::Dismiss, ..open(t0 + 5000 + i, "a") });
        }
        let after = m.predict(&ctx, t0 + 5003, 3, &[]).into_iter().find(|p| p.app == "a").unwrap().p;
        assert!(after < before, "{after} < {before}");
    }

    #[test]
    fn explanations() {
        let base = 1_700_000_000u64 / 86400 * 86400;
        let recs: Vec<Record> = (0..10).map(|d| open(base + d * 86400, "music")).collect();
        let now = base + 10 * 86400;
        match explain(&recs, "music", None, now) {
            Why::Time { days, .. } => assert!(days >= 2),
            Why::Often { .. } => {} // a weekend/weekday boundary in local time can split them
            other => panic!("{other:?}"),
        }
        let seq: Vec<Record> = (0..6).flat_map(|i| [open(now - 5000 + i * 100, "foot"), open(now - 4950 + i * 100, "files")]).collect();
        assert_eq!(explain(&seq, "files", Some("foot"), now), Why::After { after: "foot".into(), times: 6 });
        assert_eq!(Why::After { after: "Terminal".into(), times: 6 }.words("Files"),
                   "You often open Files after Terminal (6 times in the last two weeks).");
    }
}
