//! Undo and the trash.
//!
//! Every action that changes something gets an id and a journal entry in
//! ~/.local/share/nexa-mind/undo/<id>/journal.json listing its steps; an
//! overwritten file's previous content is kept beside it. Undo replays the
//! steps backwards. Deleting moves files to the desktop Trash
//! (freedesktop.org Trash spec: files/ + info/<name>.trashinfo), so they can
//! also be restored from the Files app. Every change is appended to
//! ~/.local/share/nexa-mind/activity.jsonl (one JSON object per line).

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Step {
    /// A file or folder was created at `path` (undo: remove it if unchanged).
    Created { path: PathBuf },
    /// `path` was overwritten; its previous content is saved as `backup`.
    Replaced { path: PathBuf, backup: PathBuf },
    /// Moved from `from` to `to` (undo: move back).
    Moved { from: PathBuf, to: PathBuf },
    /// Moved to the trash (undo: restore from `trashed`).
    Trashed { path: PathBuf, trashed: PathBuf, info: PathBuf },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub time: u64,
    pub summary: String,
    pub steps: Vec<Step>,
    pub undone: bool,
}

pub struct Journal {
    pub home: PathBuf,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Journal {
    pub fn mind_dir(&self) -> PathBuf {
        self.home.join(".local/share/nexa-mind")
    }
    pub fn undo_dir(&self) -> PathBuf {
        self.mind_dir().join("undo")
    }
    pub fn trash_dir(&self) -> PathBuf {
        self.home.join(".local/share/Trash")
    }

    /// A new, unique id: time plus a counter if needed.
    pub fn new_id(&self) -> std::io::Result<(String, PathBuf)> {
        fs::create_dir_all(self.undo_dir())?;
        let base = now();
        for n in 0..1000 {
            let id = format!("{base}-{n}");
            let dir = self.undo_dir().join(&id);
            if fs::create_dir(&dir).is_ok() {
                return Ok((id, dir));
            }
        }
        Err(std::io::Error::other("couldn't allocate an undo id"))
    }

    pub fn save(&self, entry: &Entry) -> std::io::Result<()> {
        let dir = self.undo_dir().join(&entry.id);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("journal.json"), serde_json::to_vec_pretty(entry)?)?;
        self.log(&serde_json::json!({"time": entry.time, "id": entry.id, "summary": entry.summary,
                                     "undone": entry.undone}))
    }

    pub fn load(&self, id: &str) -> Result<Entry, String> {
        if !id.chars().all(|c| c.is_ascii_digit() || c == '-') || id.is_empty() {
            return Err("invalid undo id".into());
        }
        let text = fs::read(self.undo_dir().join(id).join("journal.json")).map_err(|_| format!("no action {id} to undo"))?;
        serde_json::from_slice(&text).map_err(|e| e.to_string())
    }

    /// Most recent first.
    pub fn recent(&self, limit: usize) -> Vec<Entry> {
        let mut ids: Vec<String> = fs::read_dir(self.undo_dir()).into_iter().flatten().flatten()
            .filter_map(|e| e.file_name().into_string().ok()).collect();
        ids.sort_by(|a, b| {
            let key = |s: &str| s.split_once('-').map(|(t, n)| (t.parse::<u64>().unwrap_or(0), n.parse::<u64>().unwrap_or(0))).unwrap_or((0, 0));
            key(b).cmp(&key(a))
        });
        ids.iter().take(limit).filter_map(|id| self.load(id).ok()).collect()
    }

    pub fn log(&self, record: &serde_json::Value) -> std::io::Result<()> {
        fs::create_dir_all(self.mind_dir())?;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(self.mind_dir().join("activity.jsonl"))?;
        writeln!(f, "{record}")
    }

    pub fn entry(id: String, summary: String, steps: Vec<Step>) -> Entry {
        Entry { id, time: now(), summary, steps, undone: false }
    }

    /// Move `path` to the Trash; returns the step to undo it.
    pub fn trash(&self, path: &Path) -> std::io::Result<Step> {
        let files = self.trash_dir().join("files");
        let info = self.trash_dir().join("info");
        fs::create_dir_all(&files)?;
        fs::create_dir_all(&info)?;
        let name = path.file_name().ok_or_else(|| std::io::Error::other("nothing to trash"))?.to_string_lossy().to_string();
        let mut target = name.clone();
        let mut n = 2;
        while files.join(&target).exists() || info.join(format!("{target}.trashinfo")).exists() {
            target = format!("{name}.{n}");
            n += 1;
        }
        let info_path = info.join(format!("{target}.trashinfo"));
        let date = trash_date(now());
        fs::write(&info_path, format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", percent_encode(&path.to_string_lossy())))?;
        if let Err(e) = fs::rename(path, files.join(&target)) {
            fs::remove_file(&info_path).ok();
            return Err(e);
        }
        Ok(Step::Trashed { path: path.into(), trashed: files.join(&target), info: info_path })
    }

    /// Reverse an entry's steps, last first. Refuses to overwrite anything
    /// that appeared at a path since (it reports instead).
    pub fn undo(&self, id: &str) -> Result<Entry, String> {
        let mut entry = self.load(id)?;
        if entry.undone {
            return Err("that action was already undone".into());
        }
        for step in entry.steps.iter().rev() {
            match step {
                Step::Created { path } => {
                    if path.is_dir() {
                        fs::remove_dir(path).map_err(|e| format!("{} isn't empty any more, so it was kept ({e})", path.display()))?;
                    } else if path.exists() {
                        let trashed = self.trash(path).map_err(|e| e.to_string())?;
                        let _ = trashed; // created files go to the trash, not oblivion
                    }
                }
                Step::Replaced { path, backup } => {
                    fs::copy(backup, path).map_err(|e| format!("couldn't restore {}: {e}", path.display()))?;
                }
                Step::Moved { from, to } => {
                    if from.exists() {
                        return Err(format!("{} exists again, so {} was left where it is", from.display(), to.display()));
                    }
                    fs::rename(to, from).map_err(|e| format!("couldn't move {} back: {e}", to.display()))?;
                }
                Step::Trashed { path, trashed, info } => {
                    if path.exists() {
                        return Err(format!("{} exists again, so the trashed copy stays in the Trash", path.display()));
                    }
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    fs::rename(trashed, path).map_err(|e| format!("couldn't restore {}: {e}", path.display()))?;
                    fs::remove_file(info).ok();
                }
            }
        }
        entry.undone = true;
        self.save(&entry).map_err(|e| e.to_string())?;
        Ok(entry)
    }
}

/// "2026-10-13T14:05:09" in UTC (the spec asks for local time without a
/// zone; UTC keeps this free of time-zone code and stays sortable).
pub fn trash_date(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn percent_encode(s: &str) -> String {
    s.bytes().map(|b| {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(trash_date(0), "1970-01-01T00:00:00");
        assert_eq!(trash_date(1_791_878_400), "2026-10-13T08:00:00");
        assert_eq!(trash_date(951_782_400), "2000-02-29T00:00:00");
    }

    #[test]
    fn trash_spec_paths_are_encoded() {
        assert_eq!(percent_encode("/home/a/My File#1.txt"), "/home/a/My%20File%231.txt");
    }
}
