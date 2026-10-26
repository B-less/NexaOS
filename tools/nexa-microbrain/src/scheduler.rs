//! The queue for MicroBrain's own background work (storage scans, index
//! updates, preloading, maintenance). Jobs run only when AdaptivePulse's
//! budget allows; a job that has to wait says why, and starts by itself
//! when the reason goes away. Bounded: a full queue refuses new jobs and
//! counts them, it never grows without limit.

use crate::pulse::Budget;
use serde::Serialize;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Must run (the person asked, or safety): ignores the budget.
    Essential,
    // The budget already rules on these (tests cover it); no job of these
    // kinds is submitted outside the tests yet.
    #[allow(dead_code)]
    StorageScan,
    #[allow(dead_code)]
    Index,
    Maintenance,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    pub queued_at: u64,
    /// Why it's waiting, if it is.
    pub waiting: Option<String>,
}

pub struct Scheduler {
    queue: VecDeque<Job>,
    running: Vec<Job>,
    cap: usize,
    next_id: u64,
    pub refused: u64,
}

fn allowed(kind: Kind, b: &Budget) -> Result<(), String> {
    let ok = match kind {
        Kind::Essential => true,
        Kind::StorageScan => b.scans,
        Kind::Index => b.indexing,
        Kind::Maintenance => b.jobs > 0,
    };
    if ok { Ok(()) } else { Err(b.why.clone()) }
}

impl Scheduler {
    pub fn new(cap: usize) -> Scheduler {
        Scheduler { queue: VecDeque::new(), running: Vec::new(), cap: cap.max(1), next_id: 1, refused: 0 }
    }

    /// Queue a job; the same job queued twice is one job. None if full.
    pub fn submit(&mut self, name: &str, kind: Kind, now: u64) -> Option<u64> {
        if let Some(j) = self.queue.iter().chain(self.running.iter()).find(|j| j.name == name) {
            return Some(j.id);
        }
        if self.queue.len() >= self.cap {
            self.refused += 1;
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.queue.push_back(Job { id, name: name.into(), kind, queued_at: now, waiting: None });
        Some(id)
    }

    /// The jobs to start now under `budget`. Essential jobs always start;
    /// others while fewer than `budget.jobs` are running.
    pub fn next(&mut self, budget: &Budget) -> Vec<Job> {
        let mut start = Vec::new();
        let mut keep = VecDeque::new();
        let mut busy = self.running.iter().filter(|j| j.kind != Kind::Essential).count();
        while let Some(mut job) = self.queue.pop_front() {
            match allowed(job.kind, budget) {
                Ok(()) if job.kind == Kind::Essential || busy < budget.jobs => {
                    if job.kind != Kind::Essential {
                        busy += 1;
                    }
                    job.waiting = None;
                    self.running.push(job.clone());
                    start.push(job);
                }
                Ok(()) => {
                    job.waiting = Some("waiting for a free slot".into());
                    keep.push_back(job);
                }
                Err(why) => {
                    job.waiting = Some(why);
                    keep.push_back(job);
                }
            }
        }
        self.queue = keep;
        start
    }

    pub fn finish(&mut self, id: u64) -> Option<Job> {
        let i = self.running.iter().position(|j| j.id == id)?;
        Some(self.running.remove(i))
    }

    pub fn cancel(&mut self, id: u64) -> bool {
        let before = self.queue.len();
        self.queue.retain(|j| j.id != id);
        before != self.queue.len()
    }

    pub fn waiting(&self) -> impl Iterator<Item = &Job> {
        self.queue.iter()
    }

    pub fn running(&self) -> impl Iterator<Item = &Job> {
        self.running.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Profile;
    use crate::pulse::{decide, Conditions, Mode};
    use crate::resources::Level;

    fn budget(memory: Level) -> Budget {
        decide(Mode::Auto, &Conditions { memory, cpu: Level::Normal, on_battery: false, battery: None }, Profile::Medium)
    }

    #[test]
    fn under_pressure_scans_wait_and_resume_later() {
        let mut s = Scheduler::new(8);
        let scan = s.submit("storage-scan", Kind::StorageScan, 1).unwrap();
        assert!(s.next(&budget(Level::High)).is_empty(), "memory pressure: the scan waits");
        let w: Vec<_> = s.waiting().collect();
        assert_eq!(w.len(), 1);
        assert!(w[0].waiting.as_deref().unwrap().contains("memory"), "{:?}", w[0].waiting);
        let started = s.next(&budget(Level::Normal));
        assert_eq!(started.iter().map(|j| j.id).collect::<Vec<_>>(), vec![scan], "pressure gone: it starts");
        assert!(s.finish(scan).is_some());
    }

    #[test]
    fn essential_jobs_always_run_and_slots_are_respected() {
        let mut s = Scheduler::new(8);
        s.submit("backup asked for", Kind::Essential, 1);
        s.submit("a", Kind::Maintenance, 1);
        s.submit("b", Kind::Maintenance, 1);
        s.submit("c", Kind::Maintenance, 1);
        let high = s.next(&budget(Level::High));
        assert_eq!(high.iter().map(|j| j.name.as_str()).collect::<Vec<_>>(), vec!["backup asked for"]);
        let normal = s.next(&budget(Level::Normal));
        assert_eq!(normal.len(), 2, "4 GB profile: two at once");
        assert_eq!(s.waiting().next().unwrap().waiting.as_deref(), Some("waiting for a free slot"));
    }

    #[test]
    fn the_queue_is_bounded_and_deduplicated() {
        let mut s = Scheduler::new(2);
        assert_eq!(s.submit("x", Kind::Index, 1), s.submit("x", Kind::Index, 2), "same job, same id");
        s.submit("y", Kind::Index, 1);
        assert_eq!(s.submit("z", Kind::Index, 1), None);
        assert_eq!(s.refused, 1);
        assert!(s.cancel(1));
    }
}
