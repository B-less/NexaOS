//! nexa-starmap against a fake /proc and home folder.
use nexa_starmap::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

struct Sky {
    dir: PathBuf,
}

impl Sky {
    fn new(name: &str) -> Sky {
        let dir = std::env::temp_dir().join(format!("starmap-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("home")).unwrap();
        fs::create_dir_all(dir.join("proc")).unwrap();
        Sky { dir }
    }
    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }
    fn proc_root(&self) -> PathBuf {
        self.dir.join("proc")
    }
    /// A file or folder (ending in /) under home.
    fn make(&self, rel: &str) -> PathBuf {
        let p = self.home().join(rel.trim_end_matches('/'));
        if rel.ends_with('/') {
            fs::create_dir_all(&p).unwrap();
        } else {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, "x").unwrap();
        }
        p
    }
    /// A process: its parent, working folder and open files.
    fn process(&self, pid: i32, ppid: i32, cwd: &Path, files: &[&Path]) {
        let d = self.proc_root().join(pid.to_string());
        fs::create_dir_all(d.join("fd")).unwrap();
        fs::write(d.join("stat"), format!("{pid} (a (tricky) name) S {ppid} 1 1 0 -1\n")).unwrap();
        symlink(cwd, d.join("cwd")).unwrap();
        symlink("/dev/null", d.join("fd/0")).unwrap();
        symlink("socket:[1234]", d.join("fd/1")).unwrap();
        for (i, f) in files.iter().enumerate() {
            symlink(f, d.join("fd").join((i + 3).to_string())).unwrap();
        }
    }
    fn env(&self, known: Vec<PathBuf>) -> Env {
        Env { proc_root: self.proc_root(), home: self.home(), uid: unsafe { libc::geteuid() }, known }
    }
}

impl Drop for Sky {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn win(id: &str, pid: i32, title: &str) -> Win {
    Win { id: id.into(), pid, app: "app".into(), title: title.into() }
}

#[test]
fn lines_from_orbit() {
    let w = parse_line("94\t1200\tnexa-editor\tnotes.txt — Editor\n").unwrap();
    assert_eq!((w.id.as_str(), w.pid, w.app.as_str(), w.title.as_str()), ("94", 1200, "nexa-editor", "notes.txt — Editor"));
    assert!(parse_line("94\t0\tapp\tt").is_none(), "pid 0");
    assert!(parse_line("9 4\t1\tapp\tt").is_none(), "an id is a number or hex address");
    assert!(parse_line("94\tabc\tapp\tt").is_none());
    assert!(parse_line("").is_none());
    assert_eq!(parse_line("1\t2\tapp\ta\u{7}b").unwrap().title, "ab", "control characters dropped");
    let long = "é".repeat(400);
    let t = parse_line(&format!("1\t2\tapp\t{long}")).unwrap().title;
    assert!(t.len() <= 512 && t.chars().all(|c| c == 'é'), "cut on a character boundary");
    assert_eq!(parse_line("1\t2\tapp").unwrap().title, "", "no title");
}

#[test]
fn roots() {
    let s = Sky::new("roots");
    let h = s.home();
    s.make("code/NexaOS/.git/");
    s.make("code/NexaOS/tools/src/");
    s.make("code/NexaOS/tools/Cargo.toml");
    s.make("Documents/Thesis/figures/");
    s.make("notes/");
    s.make("Downloads/");
    s.make(".config/nexa/");
    assert_eq!(root_of(&h.join("code/NexaOS/tools/src"), &h), Some(h.join("code/NexaOS/tools")), "the nearest marker");
    assert_eq!(root_of(&h.join("code/NexaOS"), &h), Some(h.join("code/NexaOS")));
    assert_eq!(root_of(&h.join("Documents/Thesis/figures"), &h), Some(h.join("Documents/Thesis")), "two levels under home");
    assert_eq!(root_of(&h.join("notes"), &h), Some(h.join("notes")), "a folder of your own");
    assert_eq!(root_of(&h.join("Downloads"), &h), None, "a standard folder alone");
    assert_eq!(root_of(&h.join("Documents"), &h), None);
    assert_eq!(root_of(&h, &h), None, "home itself");
    assert_eq!(root_of(&h.join(".config/nexa"), &h), None, "hidden");
    assert_eq!(root_of(Path::new("/etc/ssl"), &h), None, "outside home");
    assert_eq!(root_of(&h.join("code/../../etc"), &h), None);
}

#[test]
fn paths_in_titles() {
    let s = Sky::new("titles");
    let h = s.home();
    let f = s.make("code/web/src/app.js");
    assert_eq!(title_paths("~/code/web/src/app.js — Editor", &h), vec![f.clone()]);
    assert_eq!(title_paths(&format!("Files: {}/code/web/src (3 items)", h.display()), &h), vec![h.join("code/web/src")]);
    assert_eq!(title_paths("~/code/web/src/gone.js - Editor", &h), vec![h.join("code/web/src")], "shortened until it exists");
    assert_eq!(title_paths("see x~/code/web", &h), Vec::<PathBuf>::new(), "not the start of a word");
    assert_eq!(title_paths("~/", &h), Vec::<PathBuf>::new(), "home itself isn't a clue");
    assert!(title_paths("Inbox — Mail", &h).is_empty());
    let spaced = s.make("Documents/My Thesis/ch 1.odt");
    assert_eq!(title_paths("~/Documents/My Thesis/ch 1.odt — Writer", &h), vec![spaced], "spaces in a path");
}

#[test]
fn names() {
    assert!(names_in("notes.txt — Editor", "notes.txt"));
    assert!(names_in("● notes.txt — Editor", "notes.txt"));
    assert!(names_in("Saved notes.txt.", "notes.txt"), "at the end of a sentence");
    assert!(!names_in("data.txt — Editor", "a.txt"), "inside a longer name");
    assert!(!names_in("notes.txt.bak — Editor", "notes.txt"));
    assert!(!names_in("my-notes.txt", "notes.txt"));
    assert!(!names_in("ab — Editor", "ab"), "too short");
    assert!(!names_in("Read me", "me"));
    assert!(names_in("Makefile — Editor", "Makefile"), "five letters without a dot");
    assert!(!names_in("Home — Files", "Home"), "a short word without a dot");
}

#[test]
fn recent_xbel() {
    let s = Sky::new("xbel");
    let share = s.home().join(".local/share");
    fs::create_dir_all(&share).unwrap();
    fs::write(
        share.join("recently-used.xbel"),
        r#"<?xml version="1.0"?><xbel version="1.0">
<bookmark href="file:///home/u/Documents/My%20Thesis/ch%C3%A91.odt" added="x"></bookmark>
<bookmark href="file:///home/u/R&amp;D/plan.md"></bookmark>
<bookmark href="https://example.com/x"></bookmark>
<bookmark href="file:///home/u/bad%zz"></bookmark>
</xbel>"#,
    )
    .unwrap();
    std::env::remove_var("XDG_DATA_HOME");
    let got = recent_files(&s.home());
    assert_eq!(got, vec![PathBuf::from("/home/u/Documents/My Thesis/ché1.odt"), PathBuf::from("/home/u/R&D/plan.md")]);
}

#[test]
fn projects() {
    let s = Sky::new("projects");
    let h = s.home();
    s.make("code/NexaOS/.git/");
    let src = s.make("code/NexaOS/src/");
    let readme_a = s.make("code/NexaOS/README.md");
    let readme_b = s.make("code/web/README.md");
    s.make("code/web/package.json");
    let ch1 = s.make("Documents/Thesis/ch1.odt");
    let data = s.make("Documents/Thesis/data.csv");
    let other = s.make("notes/todo.txt");

    // a terminal: foot in home, its shell in ~/code/NexaOS/src
    s.process(100, 1, &h, &[]);
    s.process(101, 100, &src, &[]);
    // an editor showing ch1.odt (by name; a recent file)
    s.process(200, 1, &h, &[]);
    // a spreadsheet holding data.csv open
    s.process(300, 1, &h, &[&data]);
    // a viewer whose title names README.md (in two projects: ambiguous)
    s.process(400, 1, &h, &[]);
    // nothing to go on
    s.process(500, 1, &h, &[]);
    // its own folder, but the title's path is stronger
    s.process(600, 1, &h.join("notes"), &[]);
    // one process, two windows (a browser): the same clues for both
    s.process(700, 1, &src, &[]);
    let env = s.env(vec![ch1.clone(), readme_a, readme_b, other]);
    let wins = vec![
        win("1", 100, "ama@pc: ~"),
        win("2", 200, "ch1.odt — Writer"),
        win("3", 300, "Sheet"),
        win("4", 400, "README.md — Viewer"),
        win("5", 500, "Inbox — Mail"),
        win("6", 600, "~/Documents/Thesis/ch1.odt — Editor"),
        win("7", 700, "Tab one"),
        win("8", 700, "Tab two"),
    ];
    let found = find(&env, &wins);
    let p = |i: usize| found[i].project.clone();
    assert_eq!((p(0), found[0].why), (Some(h.join("code/NexaOS")), "terminal"), "a shell's folder");
    assert_eq!((p(1), found[1].why), (Some(h.join("Documents/Thesis")), "recent"), "a recent file's name");
    assert_eq!((p(2), found[2].why), (Some(h.join("Documents/Thesis")), "files"), "an open file");
    assert_eq!(p(3), None, "README.md is in two projects: no guess");
    assert_eq!(p(4), None);
    assert_eq!((p(5), found[5].why), (Some(h.join("Documents/Thesis")), "title"), "the title's path over its own folder");
    assert_eq!(p(6), Some(h.join("code/NexaOS")));
    assert_eq!(p(7), p(6));
    assert_eq!(found.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["1", "2", "3", "4", "5", "6", "7", "8"], "in order");

    let json: serde_json::Value = serde_json::from_str(&to_json(&found, &h)).unwrap();
    assert_eq!(json["windows"][0]["name"], "NexaOS");
    assert_eq!(json["windows"][0]["place"], "~/code/NexaOS");
    assert_eq!(json["windows"][0]["why"], "terminal");
    assert!(json["windows"][4]["project"].is_null());
}

#[test]
fn only_your_own_processes() {
    let s = Sky::new("privacy");
    let h = s.home();
    let secret = s.make("work/secret/plan.txt");
    s.process(100, 1, &h.join("work/secret"), &[&secret]);
    s.process(101, 100, &h.join("work/secret"), &[]);
    let mut env = s.env(vec![]);
    env.uid += 1;  // these processes belong to someone else
    let found = find(&env, &[win("1", 100, "Window")]);
    assert_eq!(found[0].project, None, "another user's processes aren't read");
}

#[test]
fn a_gone_process() {
    let s = Sky::new("gone");
    let env = s.env(vec![]);
    let found = find(&env, &[win("1", 4242, "Window")]);
    assert_eq!(found[0].project, None);
}

#[test]
fn a_cycle_in_the_process_table() {
    // a damaged table (or pid reuse mid-read) mustn't loop
    let s = Sky::new("cycle");
    let h = s.home();
    s.make("code/a/.git/");
    s.process(10, 11, &h.join("code/a"), &[]);
    s.process(11, 10, &h, &[]);
    let found = find(&s.env(vec![]), &[win("1", 10, "x")]);
    assert_eq!(found[0].project, Some(h.join("code/a")));
}
