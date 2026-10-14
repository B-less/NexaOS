//! People on this computer: change your own password, add a person
//! (optionally an administrator, i.e. in the wheel group), remove one.
//! Passwords go to chpasswd on stdin (never argv) and are stored as SHA-512.
//! Removing someone keeps their files (/home/<name> stays), and the broker
//! refuses to remove the person asking or the last administrator.

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The groups the desktop account is in (seat, video, audio, input, ...).
pub const DESKTOP_GROUPS: &[&str] = &["seat", "video", "audio", "input", "nexa-mind"];
pub const HOME_FOLDERS: &[&str] = &["Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos"];

fn run(prog: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(prog).args(args).stdin(Stdio::null()).output().map_err(|e| format!("couldn't run {prog}: {e}"))?;
    if out.status.success() { Ok(()) } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().lines().last().unwrap_or("failed").to_string())
    }
}

fn set_password(user: &str, password: &str) -> Result<(), String> {
    let mut child = Command::new("chpasswd").args(["-c", "sha512"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("couldn't run chpasswd: {e}"))?;
    child.stdin.take().unwrap().write_all(format!("{user}:{password}\n").as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

pub fn users(etc: &Path) -> Vec<(String, u32)> {
    fs::read_to_string(etc.join("passwd")).unwrap_or_default().lines().filter_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 3).then(|| (f[0].to_string(), f[2].parse().unwrap_or(0)))
    }).collect()
}

pub fn admins(etc: &Path) -> Vec<String> {
    fs::read_to_string(etc.join("group")).unwrap_or_default().lines()
        .find(|l| l.starts_with("wheel:")).and_then(|l| l.split(':').nth(3))
        .map(|m| m.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

pub fn change_password(user: &str, new_password: &str) -> Result<Value, String> {
    set_password(user, new_password)?;
    Ok(json!({"done": "Your password was changed"}))
}

pub fn add(etc: &Path, name: &str, full_name: &str, password: &str, admin: bool) -> Result<Value, String> {
    if users(etc).iter().any(|(n, _)| n == name) {
        return Err(format!("there is already an account called {name}"));
    }
    run("adduser", &["-D", "-h", &format!("/home/{name}"), "-s", "/bin/sh", "-g", full_name, name])?;
    if let Err(e) = set_password(name, password) {
        let _ = run("deluser", &["--remove-home", name]);
        return Err(e);
    }
    let groups = fs::read_to_string(etc.join("group")).unwrap_or_default();
    for g in DESKTOP_GROUPS.iter().chain(if admin { &["wheel"][..] } else { &[][..] }) {
        if groups.lines().any(|l| l.starts_with(&format!("{g}:"))) {
            run("addgroup", &[name, g])?;
        }
    }
    let home = Path::new("/home").join(name);
    for d in HOME_FOLDERS {
        let _ = fs::create_dir_all(home.join(d));
    }
    if let Some((_, uid)) = users(etc).into_iter().find(|(n, _)| n == name) {
        let _ = run("chown", &["-R", &format!("{uid}:{uid}"), &home.to_string_lossy()]);
    }
    Ok(json!({"done": format!("Added {} {name}", if admin { "the administrator" } else { "the user" })}))
}

pub fn remove(etc: &Path, caller: &str, name: &str) -> Result<Value, String> {
    let Some((_, uid)) = users(etc).into_iter().find(|(n, _)| n == name) else {
        return Err(format!("there is no account called {name}"));
    };
    if name == caller {
        return Err("you can't remove your own account while using it".into());
    }
    if !(1000..60000).contains(&uid) {
        return Err(format!("{name} is a system account"));
    }
    let admins = admins(etc);
    if admins.iter().any(|a| a == name) && admins.len() <= 1 {
        return Err("that's the last administrator; make someone else an administrator first".into());
    }
    // Their files stay in /home/<name>.
    run("deluser", &[name])?;
    Ok(json!({"done": format!("Removed {name} (their files are kept in /home/{name})")}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_is_who() {
        let d = std::env::temp_dir().join(format!("nexa-broker-users-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("passwd"), "root:x:0:0::/root:/bin/sh\nuser:x:1000:1000::/home/user:/bin/sh\nama:x:1001:1001::/home/ama:/bin/sh\n").unwrap();
        fs::write(d.join("group"), "wheel:x:10:user\n").unwrap();
        assert_eq!(admins(&d), vec!["user"]);
        assert!(remove(&d, "user", "user").unwrap_err().contains("your own account"));
        assert!(remove(&d, "ama", "user").unwrap_err().contains("last administrator"));
        assert!(remove(&d, "user", "root").unwrap_err().contains("system account"));
        assert!(remove(&d, "user", "zed").unwrap_err().contains("no account"));
        assert!(add(&d, "ama", "", "secret1", false).unwrap_err().contains("already"));
    }
}
