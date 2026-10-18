//! The habit journal: what was used when, as compact binary records in an
//! append-only file (about 20 bytes plus the app id each). Only the last 14
//! days are kept raw; older records are folded into the base model's counts
//! and dropped. Never stores window titles, file names or anything typed.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

const MAGIC: &[u8; 6] = b"NXPJ1\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An app came to the front for the first time (a new process).
    Launch = 1,
    /// Back to an app that was already open.
    Switch = 2,
    /// You dismissed a suggestion of this app.
    Dismiss = 3,
    /// You opened this app from a suggestion (counts half as a habit).
    Used = 4,
}

impl Kind {
    fn from(b: u8) -> Option<Kind> {
        Some(match b { 1 => Kind::Launch, 2 => Kind::Switch, 3 => Kind::Dismiss, 4 => Kind::Used, _ => return None })
    }

    pub fn is_open(self) -> bool {
        matches!(self, Kind::Launch | Kind::Switch)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub ts: u64,
    pub kind: Kind,
    pub app: String,
    pub place: u32,
    pub plugged: bool,
}

fn encode(r: &Record, out: &mut Vec<u8>) {
    let app = &r.app.as_bytes()[..r.app.len().min(255)];
    out.extend_from_slice(&r.ts.to_le_bytes());
    out.push(r.kind as u8);
    out.push(r.plugged as u8);
    out.extend_from_slice(&r.place.to_le_bytes());
    out.push(app.len() as u8);
    out.extend_from_slice(app);
}

pub fn decode(bytes: &[u8]) -> Vec<Record> {
    let mut out = Vec::new();
    let mut b = bytes.strip_prefix(MAGIC.as_slice()).unwrap_or(&[]);
    // A torn last record (power cut mid-write) is simply dropped.
    while b.len() >= 15 {
        let len = b[14] as usize;
        if b.len() < 15 + len {
            break;
        }
        if let (Some(kind), Ok(app)) = (Kind::from(b[8]), std::str::from_utf8(&b[15..15 + len])) {
            out.push(Record {
                ts: u64::from_le_bytes(b[0..8].try_into().unwrap()),
                kind,
                plugged: b[9] != 0,
                place: u32::from_le_bytes(b[10..14].try_into().unwrap()),
                app: app.to_string(),
            });
        }
        b = &b[15 + len..];
    }
    out
}

pub fn read(path: &Path) -> Vec<Record> {
    let mut bytes = Vec::new();
    match fs::File::open(path) {
        Ok(mut f) => {
            let _ = f.read_to_end(&mut bytes);
            decode(&bytes)
        }
        Err(_) => Vec::new(),
    }
}

pub fn append(path: &Path, r: &Record) -> io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    let mut buf = Vec::new();
    if f.metadata()?.len() == 0 {
        buf.extend_from_slice(MAGIC);
    }
    encode(r, &mut buf);
    f.write_all(&buf)
}

/// Replace the whole journal (forgetting, pruning), atomically.
pub fn write(path: &Path, records: &[Record]) -> io::Result<()> {
    let mut buf = MAGIC.to_vec();
    for r in records {
        encode(r, &mut buf);
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, buf)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ts: u64, app: &str) -> Record {
        Record { ts, kind: Kind::Launch, app: app.into(), place: 7, plugged: true }
    }

    #[test]
    fn round_trip_and_torn_tail() {
        let d = std::env::temp_dir().join(format!("nexa-patternd-j-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let p = d.join("journal.bin");
        append(&p, &rec(100, "foot")).unwrap();
        append(&p, &Record { kind: Kind::Dismiss, ..rec(200, "nexa-files") }).unwrap();
        assert_eq!(read(&p), vec![rec(100, "foot"), Record { kind: Kind::Dismiss, ..rec(200, "nexa-files") }]);
        let mut bytes = fs::read(&p).unwrap();
        bytes.truncate(bytes.len() - 3);
        assert_eq!(decode(&bytes), vec![rec(100, "foot")], "half a record is dropped, not misread");
        write(&p, &[rec(5, "a")]).unwrap();
        assert_eq!(read(&p), vec![rec(5, "a")]);
        assert!(decode(b"garbage").is_empty());
    }
}
