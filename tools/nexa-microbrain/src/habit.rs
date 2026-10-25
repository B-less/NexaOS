//! HabitCore: apps you open together, from Nexa Patterns' journal (opt-in
//! in Settings > Privacy > Patterns, read-only here). Launches that follow
//! each other within 15 minutes form a "sitting"; a set of 2-5 apps that
//! shows up in sittings on at least 3 different days becomes a suggestion:
//! "Save Editor, Terminal and Files as a workspace?". Thin or random data
//! gives nothing. Dismissed suggestions never come back; sets already
//! saved as a workspace aren't suggested.

#[path = "../../nexa-patternd/src/journal.rs"]
#[allow(dead_code)]
mod journal;

use crate::store::Doc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SITTING_GAP: u64 = 15 * 60;
const MIN_DAYS: usize = 3;
const MAX_SET: usize = 5;

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Habits {
    /// App sets the person said no to (sorted ids).
    pub dismissed: Vec<Vec<String>>,
    /// App sets already offered once (not offered again in a pop-up).
    #[serde(default)]
    pub offered: Vec<Vec<String>>,
}

impl Doc for Habits {
    const NAME: &'static str = "habits";
    const VERSION: u32 = 1;
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub apps: Vec<String>,
    /// Different days the set was used together.
    pub days: usize,
    /// Share of active days with this set (0..1).
    pub confidence: f64,
    pub why: String,
}

pub fn journal_path(home: &Path) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"))
        .join("nexa-patterns/journal.bin")
}

/// (timestamp, app) for every launch in the journal.
pub fn launches(path: &Path) -> Vec<(u64, String)> {
    journal::read(path).into_iter().filter(|r| r.kind == journal::Kind::Launch).map(|r| (r.ts, r.app)).collect()
}

/// Sets of apps launched together, with the days they were.
pub fn suggestions(launches: &[(u64, String)], skip: &[BTreeSet<String>]) -> Vec<Suggestion> {
    let mut sorted: Vec<&(u64, String)> = launches.iter().collect();
    sorted.sort_by_key(|(t, _)| *t);
    // Sittings: runs of launches with gaps under SITTING_GAP.
    let mut sittings: Vec<(u64, BTreeSet<String>)> = Vec::new();
    let mut last = 0u64;
    for (t, app) in sorted {
        if sittings.is_empty() || t.saturating_sub(last) > SITTING_GAP {
            sittings.push((*t, BTreeSet::new()));
        }
        sittings.last_mut().unwrap().1.insert(app.clone());
        last = *t;
    }
    let active_days: BTreeSet<u64> = sittings.iter().map(|(t, _)| t / 86400).collect();
    // Every 2..=5 subset of each sitting, counted by distinct day.
    let mut seen: BTreeMap<BTreeSet<String>, BTreeSet<u64>> = BTreeMap::new();
    for (t, apps) in &sittings {
        let apps: Vec<&String> = apps.iter().take(8).collect();
        let n = apps.len();
        for mask in 1u32..(1 << n) {
            let size = mask.count_ones() as usize;
            if !(2..=MAX_SET).contains(&size) {
                continue;
            }
            let set: BTreeSet<String> = (0..n).filter(|i| mask & (1 << i) != 0).map(|i| apps[i].clone()).collect();
            seen.entry(set).or_default().insert(t / 86400);
        }
    }
    let mut found: Vec<Suggestion> = seen.into_iter()
        .filter(|(set, days)| days.len() >= MIN_DAYS && !skip.contains(set))
        .map(|(set, days)| {
            let confidence = days.len() as f64 / active_days.len().max(1) as f64;
            Suggestion { apps: set.into_iter().collect(), days: days.len(), confidence,
                         why: format!("opened together on {} of your last {} active days", days.len(), active_days.len()) }
        })
        .collect();
    // Prefer bigger sets; drop a set when a bigger one that contains it is as frequent.
    found.sort_by(|a, b| b.apps.len().cmp(&a.apps.len()).then(b.days.cmp(&a.days)));
    let mut kept: Vec<Suggestion> = Vec::new();
    for s in found {
        let covered = kept.iter().any(|k| k.days >= s.days && s.apps.iter().all(|a| k.apps.contains(a)));
        if !covered {
            kept.push(s);
        }
    }
    kept.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    kept.truncate(5);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86400;

    fn day(d: u64, hour: u64, apps: &[&str]) -> Vec<(u64, String)> {
        apps.iter().enumerate().map(|(i, a)| (d * DAY + hour * 3600 + i as u64 * 120, a.to_string())).collect()
    }

    #[test]
    fn a_repeated_set_is_suggested() {
        let mut l = Vec::new();
        for d in 0..5 {
            l.extend(day(100 + d, 9, &["nexa-edit", "foot", "nexa-files"]));
            l.extend(day(100 + d, 20, &["nexa-music"]));
        }
        let s = suggestions(&l, &[]);
        assert_eq!(s[0].apps, vec!["foot", "nexa-edit", "nexa-files"]);
        assert_eq!(s[0].days, 5);
        assert!(s[0].confidence > 0.9);
        assert!(!s.iter().any(|x| x.apps == vec!["foot".to_string(), "nexa-edit".to_string()]), "a smaller set inside it isn't repeated");
    }

    #[test]
    fn thin_or_random_data_gives_nothing() {
        let mut l = Vec::new();
        for d in 0..2 {
            l.extend(day(100 + d, 9, &["nexa-edit", "foot"]));
        }
        assert!(suggestions(&l, &[]).is_empty(), "two days isn't a habit");
        // Random apps, each day different.
        let apps = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"];
        let mut r = Vec::new();
        for d in 0..6u64 {
            let i = (d * 5 % 12) as usize;
            r.extend(day(200 + d, 10, &[apps[i], apps[(i + 7) % 12]]));
        }
        assert!(suggestions(&r, &[]).is_empty(), "{:?}", suggestions(&r, &[]));
        // Apps hours apart aren't "together".
        let mut apart = Vec::new();
        for d in 0..5 {
            apart.push(((300 + d) * DAY + 8 * 3600, "nexa-edit".to_string()));
            apart.push(((300 + d) * DAY + 15 * 3600, "foot".to_string()));
        }
        assert!(suggestions(&apart, &[]).is_empty());
    }

    #[test]
    fn dismissed_sets_never_come_back() {
        let mut l = Vec::new();
        for d in 0..4 {
            l.extend(day(100 + d, 9, &["nexa-edit", "foot"]));
        }
        let no: BTreeSet<String> = ["foot", "nexa-edit"].iter().map(|s| s.to_string()).collect();
        assert!(suggestions(&l, &[no]).is_empty());
    }

    #[test]
    fn reads_the_patterns_journal() {
        let dir = std::env::temp_dir().join(format!("mb-habit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.bin");
        for (ts, app, kind) in [(10, "nexa-edit", journal::Kind::Launch), (20, "foot", journal::Kind::Switch), (30, "foot", journal::Kind::Launch)] {
            journal::append(&path, &journal::Record { ts, kind, app: app.into(), place: 0, plugged: true }).unwrap();
        }
        assert_eq!(launches(&path), vec![(10, "nexa-edit".to_string()), (30, "foot".to_string())], "launches only");
        assert!(launches(&dir.join("missing.bin")).is_empty(), "no journal (Patterns off): nothing");
        std::fs::remove_dir_all(dir).ok();
    }
}
