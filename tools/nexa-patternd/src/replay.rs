//! Measuring the models: replay a journal (real or synthetic) in order and,
//! before each app switch in the final days, check whether the app opened
//! was among the top predictions. `nexa-patternd replay` prints the result;
//! tests use the synthetic month to keep the models honest.

use crate::context::{self, Ctx};
use crate::journal::{Kind, Record};
use crate::model::Model;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Score {
    pub predictions: usize,
    pub top1: f64,
    pub top3: f64,
}

pub fn evaluate(records: &[Record], test_days: u64) -> Score {
    let Some(last) = records.last().map(|r| r.ts) else { return Score { predictions: 0, top1: 0.0, top3: 0.0 } };
    let test_from = last.saturating_sub(test_days * 86400);
    let mut m = Model::default();
    let (mut n, mut hit1, mut hit3) = (0usize, 0usize, 0usize);
    for r in records {
        if r.ts >= test_from && r.kind.is_open() && m.previous() != Some(r.app.as_str()) && m.ready() {
            let (slot, weekend, _) = context::local(r.ts);
            let p = m.predict(&Ctx { slot, weekend, place: r.place, plugged: r.plugged }, r.ts, 3, &[]);
            n += 1;
            hit1 += (p.first().map(|x| &x.app) == Some(&r.app)) as usize;
            hit3 += p.iter().any(|x| x.app == r.app) as usize;
        }
        m.apply(r);
    }
    let pct = |h: usize| (h as f64 / n.max(1) as f64 * 1000.0).round() / 10.0;
    Score { predictions: n, top1: pct(hit1), top3: pct(hit3) }
}

/// How fast it learns: for each day from the first, how often the app
/// opened next was on the card (top 3) - counting the days before it is
/// ready, or an app it doesn't know yet, as misses, since then the card
/// showed nothing useful.
pub fn curve(records: &[Record], days: u64) -> Vec<f64> {
    let Some(first) = records.first().map(|r| r.ts / 86400 * 86400) else { return vec![] };
    let mut m = Model::default();
    let mut hits = vec![0usize; days as usize];
    let mut all = vec![0usize; days as usize];
    for r in records {
        let d = ((r.ts - first) / 86400) as usize;
        if d >= days as usize {
            break;
        }
        if r.kind.is_open() && m.previous() != Some(r.app.as_str()) {
            all[d] += 1;
            if m.ready() {
                let (slot, weekend, _) = context::local(r.ts);
                let p = m.predict(&Ctx { slot, weekend, place: r.place, plugged: r.plugged }, r.ts, 3, &[]);
                hits[d] += p.iter().any(|x| x.app == r.app) as usize;
            }
        }
        m.apply(r);
    }
    hits.iter().zip(&all).map(|(h, n)| (*h as f64 / (*n).max(1) as f64 * 1000.0).round() / 10.0).collect()
}

/// A small deterministic random generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A month of a plausible person: weekday mornings mail then browser, a
/// terminal-files-editor work loop, music at lunch, video in the evening;
/// lazier weekends; one place for work, another for home; a fifth of all
/// switches are random. `start` is midnight UTC of day 0.
pub fn synthetic(days: u64, seed: u64) -> Vec<Record> {
    let mut rng = Rng(seed | 1);
    let others = ["nexa-calc", "nexa-notes", "nexa-pulse", "nexa-settings", "nexa-viewer"];
    let start = 1_767_225_600u64; // 2026-01-01 00:00 UTC
    let mut out = Vec::new();
    for d in 0..days {
        let day = start + d * 86400;
        let (_, weekend, _) = context::local(day + 12 * 3600);
        let plan: Vec<(u64, &[&str], u32)> = if weekend {
            vec![(10 * 60, &["nexa-browser", "nexa-music"], 2), (15 * 60, &["video-player"], 2), (21 * 60, &["nexa-browser"], 2)]
        } else {
            vec![
                (8 * 60 + 30, &["nexa-mail", "nexa-browser"], 1),
                (9 * 60 + 15, &["foot", "nexa-files", "nexa-edit", "foot", "nexa-edit"], 1),
                (12 * 60 + 30, &["nexa-music"], 1),
                (14 * 60, &["foot", "nexa-files", "nexa-edit"], 1),
                (19 * 60 + 30, &["video-player", "nexa-browser"], 2),
            ]
        };
        for (minute, apps, place) in plan {
            // That local time, as a Unix time.
            let mut t = (day as i64 + (minute * 60 + rng.below(20 * 60)) as i64 - context::utc_offset(day)) as u64;
            for app in apps {
                let app = if rng.below(5) == 0 { others[rng.below(others.len() as u64) as usize] } else { app };
                out.push(Record { ts: t, kind: Kind::Launch, app: app.to_string(), place, plugged: place == 1 });
                t += 120 + rng.below(900);
            }
        }
    }
    out.sort_by_key(|r| r.ts);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_a_month_of_habits() {
        let month = synthetic(35, 42);
        let s = evaluate(&month, 7);
        assert!(s.predictions > 50, "a week of switches: {s:?}");
        assert!(s.top3 >= 60.0, "top-3 accuracy on the last week: {s:?}");
        assert!(s.top1 >= 30.0, "{s:?}");
    }

    /// `cargo test -p nexa-patternd -- --ignored --nocapture curve_report`:
    /// the learning curve averaged over 20 synthetic people.
    #[test]
    #[ignore]
    fn curve_report() {
        let mut sum = vec![0.0; 14];
        for seed in 1..=20 {
            for (s, v) in sum.iter_mut().zip(curve(&synthetic(14, seed), 14)) {
                *s += v / 20.0;
            }
        }
        let week: Vec<f64> = (1..=20).map(|seed| evaluate(&synthetic(35, seed), 7).top3).collect();
        println!("curve {:?}", sum.iter().map(|v| v.round() as i64).collect::<Vec<_>>());
        println!("month top3 avg {:.1}", week.iter().sum::<f64>() / 20.0);
    }

    #[test]
    fn random_use_isnt_overclaimed() {
        // Pure noise: nothing to learn, so accuracy stays near chance (3 of 10).
        let mut rng = Rng(7);
        let apps = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        let recs: Vec<Record> = (0..3000).map(|i| Record { ts: 1_767_225_600 + i * 900, kind: Kind::Launch,
            app: apps[rng.below(10) as usize].into(), place: 1, plugged: true }).collect();
        let s = evaluate(&recs, 7);
        assert!(s.top3 < 45.0, "{s:?}");
    }
}
