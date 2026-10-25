//! MicroBrain's memory on disk: one small JSON document per kind of thing
//! (config, habits, workspaces, skills, decisions...) in
//! ~/.local/share/nexa-microbrain (0700), each with a schema version.
//!
//! Writes are atomic (temp file, fsync, rename) and private (0600). A
//! document that can't be read (a crash mid-write on an old kernel, a
//! hand edit gone wrong) is kept aside as NAME.broken-<time> and replaced
//! with defaults: MicroBrain never stops because of its own memory.
//! Documents are capped in size; older versions are migrated on load.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Largest document kept (bigger ones are refused, not written).
pub const MAX_DOC: usize = 2 << 20;

pub struct Store {
    pub dir: PathBuf,
}

pub fn default_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("NEXA_MICROBRAIN_HOME") {
        return PathBuf::from(d);
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join(".local/share")
    });
    data.join("nexa-microbrain")
}

/// A stored document's shape: its current version and how to bring older
/// ones up to date (as JSON, before typed parsing).
pub trait Doc: Serialize + DeserializeOwned + Default {
    const NAME: &'static str;
    const VERSION: u32;
    fn migrate(_from: u32, value: Value) -> Value {
        value
    }
}

impl Store {
    pub fn open(dir: PathBuf) -> io::Result<Store> {
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        Ok(Store { dir })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    pub fn load<T: Doc>(&self) -> T {
        let path = self.path(T::NAME);
        let Ok(text) = fs::read_to_string(&path) else { return T::default() };
        let parsed: Option<T> = serde_json::from_str::<Value>(&text).ok().and_then(|mut v| {
            let version = v.get("version").and_then(Value::as_u64).unwrap_or(0) as u32;
            if version > T::VERSION {
                return None; // written by a newer MicroBrain: don't guess
            }
            if version < T::VERSION {
                for from in version..T::VERSION {
                    v = T::migrate(from, v);
                }
            }
            serde_json::from_value(v.get("data").cloned().unwrap_or(Value::Null)).ok()
        });
        match parsed {
            Some(doc) => doc,
            None => {
                let aside = self.dir.join(format!("{}.broken-{}", T::NAME, crate::now()));
                fs::rename(&path, aside).ok();
                T::default()
            }
        }
    }

    pub fn save<T: Doc>(&self, doc: &T) -> io::Result<()> {
        let body = serde_json::to_vec_pretty(&serde_json::json!({"version": T::VERSION, "data": doc}))?;
        if body.len() > MAX_DOC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} would be over {} bytes", T::NAME, MAX_DOC)));
        }
        write_private(&self.path(T::NAME), &body)
    }

    pub fn forget<T: Doc>(&self) -> io::Result<()> {
        match fs::remove_file(self.path(T::NAME)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Bytes on disk, for the Overview page.
    pub fn size(&self) -> u64 {
        fs::read_dir(&self.dir).into_iter().flatten().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()
    }
}

pub fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
    struct Notes {
        items: Vec<String>,
        #[serde(default)]
        count: u32,
    }

    impl Doc for Notes {
        const NAME: &'static str = "notes";
        const VERSION: u32 = 2;
        fn migrate(from: u32, mut v: Value) -> Value {
            // v1 kept "list"; v2 calls it "items" and adds "count".
            if from == 1 {
                if let Some(d) = v.get_mut("data") {
                    if let Some(list) = d.get("list").cloned() {
                        d["items"] = list;
                    }
                    d["count"] = Value::from(d["items"].as_array().map(|a| a.len()).unwrap_or(0));
                }
            }
            v
        }
    }

    fn store(tag: &str) -> Store {
        Store::open(std::env::temp_dir().join(format!("mb-store-{tag}-{}", std::process::id()))).unwrap()
    }

    #[test]
    fn saves_privately_and_loads_back() {
        let s = store("a");
        assert_eq!(s.load::<Notes>(), Notes::default(), "nothing yet: defaults");
        s.save(&Notes { items: vec!["x".into()], count: 1 }).unwrap();
        assert_eq!(s.load::<Notes>().items, vec!["x".to_string()]);
        let mode = fs::metadata(s.dir.join("notes.json")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        s.forget::<Notes>().unwrap();
        assert_eq!(s.load::<Notes>(), Notes::default());
        fs::remove_dir_all(&s.dir).ok();
    }

    #[test]
    fn migrates_an_older_version() {
        let s = store("b");
        fs::write(s.dir.join("notes.json"), r#"{"version": 1, "data": {"list": ["a", "b"]}}"#).unwrap();
        assert_eq!(s.load::<Notes>(), Notes { items: vec!["a".into(), "b".into()], count: 2 });
        fs::remove_dir_all(&s.dir).ok();
    }

    #[test]
    fn a_broken_file_is_set_aside_not_fatal() {
        let s = store("c");
        fs::write(s.dir.join("notes.json"), "{not json").unwrap();
        assert_eq!(s.load::<Notes>(), Notes::default());
        assert!(fs::read_dir(&s.dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("notes.broken-")));
        // A newer version than this build knows: also set aside, never guessed at.
        fs::write(s.dir.join("notes.json"), r#"{"version": 99, "data": {}}"#).unwrap();
        assert_eq!(s.load::<Notes>(), Notes::default());
        fs::remove_dir_all(&s.dir).ok();
    }

    #[test]
    fn oversized_documents_are_refused() {
        let s = store("d");
        let big = Notes { items: vec!["x".repeat(1000); 3000], count: 0 };
        assert!(s.save(&big).is_err());
        assert!(!s.dir.join("notes.json").exists());
        fs::remove_dir_all(&s.dir).ok();
    }
}
