//! nexa-starmap: Constellation's graph. Reads one window per line on stdin,
//!
//!   <id>\t<pid>\t<app id>\t<title>
//!
//! and answers each window's project on one line:
//!
//!   {"windows": [{"id": "94", "project": "/home/u/code/NexaOS",
//!                 "name": "NexaOS", "place": "~/code/NexaOS", "why": "terminal"},
//!                {"id": "97", "project": null}]}
//!
//! "why": the strongest clue (title, recent, files, terminal, folder). How
//! a project is found: lib.rs. Run by Orbit (tools-c/nexa-constellation) as
//! the user; after reading its input and the windows' processes in /proc
//! it confines itself with Landlock to reading the home folder, with no
//! network.
use nexa_sys::sandbox;
use std::io::{BufRead, Write};
use std::path::PathBuf;

fn main() {
    let home = match std::env::var_os("HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h),
        _ => {
            eprintln!("nexa-starmap: no HOME");
            std::process::exit(2);
        }
    };
    let mut wins = Vec::new();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        if let Some(w) = nexa_starmap::parse_line(&line) {
            wins.push(w);
        }
        if wins.len() >= nexa_starmap::MAX_WINDOWS {
            break;
        }
    }
    // /proc first: once confined, other processes' cwd and fd links can't be read
    let base = nexa_starmap::Env { proc_root: "/proc".into(), home: home.clone(), uid: unsafe { libc::geteuid() }, known: Vec::new() };
    let procs = nexa_starmap::snapshot(&base, &wins);
    let rules = sandbox::Rules {
        fs: vec![(home.to_string_lossy().into_owned(), sandbox::READ)],
        connect_ports: vec![],
        any_network: false,
    };
    let _ = sandbox::restrict(&rules);  // older kernels: unconfined, still read-only by design
    let mut known = nexa_starmap::recent_files(&home);
    known.extend(nexa_starmap::trail_files(&home));
    let env = nexa_starmap::Env { known, ..base };
    let found = nexa_starmap::find_in(&env, &procs, &wins);
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", nexa_starmap::to_json(&found, &home));
}
