//! Search: the items under a folder whose names contain every word typed
//! (any order, any case). Breadth-first, so things near the folder come
//! first; links aren't followed and other drives mounted inside aren't
//! entered (like `find -xdev`). Matches go out while it looks, as
//! {"found": [...]} lines, so the window fills as results arrive.
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant};

pub const MAX_RESULTS: usize = 10_000;
const MAX_WORDS: usize = 8;

pub fn words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).take(MAX_WORDS).collect()
}

pub fn matches(name: &str, words: &[String]) -> bool {
    let n = name.to_lowercase();
    !words.is_empty() && words.iter().all(|w| n.contains(w.as_str()))
}

fn emit(out: &mut dyn Write, batch: &mut Vec<Value>) {
    if !batch.is_empty() {
        let _ = writeln!(out, "{}", json!({"found": std::mem::take(batch)}));
        let _ = out.flush();
    }
}

pub fn search(dir: &Path, query: &str, hidden: bool, limit: usize, out: &mut dyn Write) -> Value {
    let words = words(query);
    let root_dev = fs::metadata(dir).map(|m| m.dev()).unwrap_or(0);
    let mut queue = VecDeque::from([dir.to_path_buf()]);
    let (mut found, mut folders, mut unreadable) = (0usize, 0usize, 0usize);
    let mut batch = Vec::new();
    let mut last = Instant::now();
    let mut truncated = false;
    'walk: while let Some(d) = queue.pop_front() {
        let Ok(rd) = fs::read_dir(&d) else {
            unreadable += 1;
            continue;
        };
        folders += 1;
        let mut kids: Vec<fs::DirEntry> = rd.filter_map(Result::ok).collect();
        kids.sort_by_key(|e| e.file_name());
        for e in kids {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if !hidden && name.starts_with('.') {
                continue;
            }
            // DirEntry::metadata doesn't follow symbolic links.
            let Ok(md) = e.metadata() else { continue };
            if matches(&name, &words) {
                batch.push(json!({
                    "path": e.path(), "name": name, "is_dir": md.is_dir(), "size": md.len(),
                    "mtime": md.mtime(), "is_exec": md.is_file() && md.mode() & 0o111 != 0,
                }));
                found += 1;
                if found >= limit {
                    truncated = true;
                    break 'walk;
                }
            }
            if md.is_dir() && md.dev() == root_dev {
                queue.push_back(e.path());
            }
        }
        if batch.len() >= 100 || (!batch.is_empty() && last.elapsed() > Duration::from_millis(120)) {
            emit(out, &mut batch);
            last = Instant::now();
        }
    }
    emit(out, &mut batch);
    let done = match found {
        0 => "Nothing found".to_string(),
        1 => "1 found".to_string(),
        n if truncated => format!("The first {n} found"),
        n => format!("{n} found"),
    };
    json!({"done": done, "count": found, "truncated": truncated, "folders": folders, "unreadable": unreadable})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_match_in_any_order_and_case() {
        let w = words("Tax 2025");
        assert!(matches("2025 TAX return.pdf", &w));
        assert!(!matches("tax 2024.pdf", &w));
        assert!(!matches("anything", &words("   ")));
    }

    #[test]
    fn nearer_first_hidden_skipped_links_not_followed() {
        let d = std::env::temp_dir().join(format!("fileops-search-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("a/b")).unwrap();
        fs::create_dir_all(d.join(".secret")).unwrap();
        fs::write(d.join("a/b/report-deep.txt"), "").unwrap();
        fs::write(d.join("report.txt"), "").unwrap();
        fs::write(d.join(".secret/report-hidden.txt"), "").unwrap();
        std::os::unix::fs::symlink(&d, d.join("loop")).unwrap();
        let mut out = Vec::new();
        let r = search(&d, "REPORT", false, 100, &mut out);
        let lines = String::from_utf8(out).unwrap();
        let names: Vec<String> = lines
            .lines()
            .flat_map(|l| serde_json::from_str::<Value>(l).unwrap()["found"].as_array().unwrap().clone())
            .map(|v| v["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["report.txt", "report-deep.txt"], "{lines}");
        assert_eq!(r["count"], 2);
        let mut out = Vec::new();
        let r = search(&d, "report", true, 1, &mut out);
        assert_eq!(r["truncated"], true);
        fs::remove_dir_all(&d).unwrap();
    }
}
