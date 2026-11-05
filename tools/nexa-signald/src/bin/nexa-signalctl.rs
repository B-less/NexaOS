//! nexa-signalctl: Nexa Signals from the command line, for Deck, the bar
//! and Settings. Its output is what they read from makoctl before.
//!
//!   nexa-signalctl list | history          JSON (makoctl's shape)
//!   nexa-signalctl dismiss --all | [ID]    (no ID: the newest)
//!   nexa-signalctl mode [-a|-r|-t do-not-disturb]
//!   nexa-signalctl invoke ID [ACTION]
//!   nexa-signalctl reload                  after Settings > Notifications changes

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn ask(req: Value) -> Result<Value, String> {
    let path = nexa_signald::socket_path();
    let mut s = UnixStream::connect(&path).map_err(|e| format!("Nexa Signals isn't running ({}: {e})", path.display()))?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(format!("{req}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|_| "no answer from Nexa Signals".to_string())?;
    if v.get("ok") == Some(&Value::Bool(true)) {
        Ok(v)
    } else {
        Err(v.get("error").and_then(Value::as_str).unwrap_or("refused").to_string())
    }
}

fn modes(quiet: bool) {
    println!("default");
    if quiet {
        println!("do-not-disturb");
    }
}

fn usage() -> ! {
    eprintln!("usage: nexa-signalctl list | history | dismiss [--all | ID] | mode [-a|-r|-t do-not-disturb] | invoke ID [ACTION] | reload");
    std::process::exit(2);
}

fn run(args: &[String]) -> Result<(), String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["list"] | ["history"] => {
            let v = ask(json!({"cmd": a[0]}))?;
            println!("{}", v["result"]);
        }
        ["dismiss", "--all"] | ["dismiss", "-a"] => {
            ask(json!({"cmd": "dismiss_all"}))?;
        }
        ["dismiss"] => {
            let v = ask(json!({"cmd": "list"}))?;
            if let Some(id) = v["result"]["data"][0][0]["id"]["data"].as_u64() {
                ask(json!({"cmd": "dismiss", "id": id}))?;
            }
        }
        ["dismiss", id] => {
            let id: u32 = id.parse().map_err(|_| format!("not an id: {id}"))?;
            ask(json!({"cmd": "dismiss", "id": id}))?;
        }
        ["mode"] => {
            let v = ask(json!({"cmd": "mode"}))?;
            modes(v["quiet"] == Value::Bool(true));
        }
        ["mode", flag, "do-not-disturb"] => {
            let now = ask(json!({"cmd": "mode"}))?["quiet"] == Value::Bool(true);
            let on = match *flag {
                "-a" => true,
                "-r" => false,
                "-t" => !now,
                _ => usage(),
            };
            let v = ask(json!({"cmd": "set_quiet", "on": on}))?;
            modes(v["quiet"] == Value::Bool(true));
        }
        ["invoke", id] | ["invoke", id, _] => {
            let id: u32 = id.parse().map_err(|_| format!("not an id: {id}"))?;
            let action = a.get(2).copied().unwrap_or("default");
            ask(json!({"cmd": "invoke", "id": id, "action": action}))?;
        }
        ["reload"] => {
            ask(json!({"cmd": "reload"}))?;
        }
        _ => usage(),
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("nexa-signalctl: {e}");
        std::process::exit(1);
    }
}
