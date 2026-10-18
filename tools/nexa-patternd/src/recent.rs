//! "The file you usually pick up now": read live from the desktop's own
//! recent-files list (~/.local/share/recently-used.xbel, kept by GTK apps),
//! never copied into the journal. A file scores for how often it was used,
//! how recently, and how close its last use's time of day is to now.

use crate::context;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Recent {
    pub path: PathBuf,
    pub app: String,
    pub modified: u64,
    pub count: u32,
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[at..].find('"')?;
    Some(tag[at..at + end].to_string())
}

/// "2026-10-17T08:31:02.123456Z" -> Unix time.
pub fn iso_time(s: &str) -> Option<u64> {
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    u64::try_from(days * 86400 + h * 3600 + mi * 60 + se).ok()
}

fn unescape(s: &str) -> String {
    let s = s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Each local file in the list with the app that used it most.
pub fn parse(xml: &str) -> Vec<Recent> {
    let mut out = Vec::new();
    for chunk in xml.split("<bookmark ").skip(1) {
        let head = chunk.split('>').next().unwrap_or("");
        let Some(href) = attr(&format!(" {head}"), "href") else { continue };
        let Some(path) = href.strip_prefix("file://") else { continue };
        let body = chunk.split("</bookmark>").next().unwrap_or("");
        let mut best: Option<(String, u64, u32)> = None;
        for app in body.split("<bookmark:application ").skip(1) {
            let tag = format!(" {}", app.split("/>").next().unwrap_or(""));
            let (Some(name), Some(modified)) = (attr(&tag, "name"), attr(&tag, "modified").and_then(|m| iso_time(&m))) else { continue };
            let count = attr(&tag, "count").and_then(|c| c.parse().ok()).unwrap_or(1);
            if best.as_ref().map_or(true, |b| count > b.2) {
                best = Some((unescape(&name), modified, count));
            }
        }
        if let Some((app, modified, count)) = best {
            out.push(Recent { path: PathBuf::from(unescape(path)), app, modified, count });
        }
    }
    out
}

pub fn list(home: &Path) -> Vec<Recent> {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    parse(&fs::read_to_string(data.join("recently-used.xbel")).unwrap_or_default())
}

/// The file most likely wanted now, if one stands out.
pub fn suggest(files: &[Recent], now: u64) -> Option<Recent> {
    let (now_slot, _, _) = context::local(now);
    files.iter().filter(|f| f.path.is_file()).map(|f| {
        let (slot, _, _) = context::local(f.modified);
        let closeness = 1.0 / (1.0 + context::slot_distance(slot, now_slot) as f64 / 2.0);
        let age_days = now.saturating_sub(f.modified) as f64 / 86400.0;
        (f, (f.count as f64).ln_1p() * closeness * 0.5f64.powf(age_days / 7.0))
    }).filter(|(_, s)| *s > 0.15).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(f, _)| f.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gtk_recent_files() {
        assert_eq!(iso_time("1970-01-02T00:00:01Z"), Some(86401));
        assert_eq!(iso_time("2026-10-17T08:31:02.123456Z"), Some(1792225862));
        let xml = r#"<?xml version="1.0"?><xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks">
  <bookmark href="file:///home/u/Documents/Q3%20report.odt" added="2026-10-01T08:00:00Z" modified="2026-10-17T08:30:00Z" visited="2026-10-17T08:30:00Z">
    <info><metadata owner="http://freedesktop.org"><mime:mime-type type="application/vnd.oasis.opendocument.text"/>
      <bookmark:applications>
        <bookmark:application name="Editor" exec="&apos;nexa-edit %u&apos;" modified="2026-10-17T08:30:00Z" count="12"/>
        <bookmark:application name="Files" exec="&apos;nexa-files&apos;" modified="2026-10-10T08:30:00Z" count="2"/>
      </bookmark:applications></metadata></info>
  </bookmark>
  <bookmark href="https://example.com/" added="2026-10-01T08:00:00Z" modified="2026-10-17T08:30:00Z" visited="2026-10-17T08:30:00Z"></bookmark>
</xbel>"#;
        let r = parse(xml);
        assert_eq!(r.len(), 1, "web links aren't files");
        assert_eq!((r[0].path.to_str().unwrap(), r[0].app.as_str(), r[0].count), ("/home/u/Documents/Q3 report.odt", "Editor", 12));
    }

    #[test]
    fn suggests_existing_files_only() {
        let d = std::env::temp_dir().join(format!("nexa-patternd-recent-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("today.txt"), "x").unwrap();
        let now = 1_792_225_862;
        let files = vec![
            Recent { path: d.join("gone.txt"), app: "Editor".into(), modified: now - 60, count: 50 },
            Recent { path: d.join("today.txt"), app: "Editor".into(), modified: now - 86400, count: 8 },
        ];
        assert_eq!(suggest(&files, now).unwrap().path, d.join("today.txt"));
        assert_eq!(suggest(&files[..1], now), None);
    }
}
