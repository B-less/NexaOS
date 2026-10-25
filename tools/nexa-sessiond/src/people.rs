//! Who may have a desktop: a person's account (uid 1000-59999) with a real
//! shell, in the "seat" group (seatd hands out the screen and input only to
//! it), whose password isn't turned off. /etc is a parameter so tests can
//! use a fake one.

use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub full_name: String,
    pub home: String,
}

fn passwd(etc: &Path) -> Vec<Person> {
    fs::read_to_string(etc.join("passwd")).unwrap_or_default().lines().filter_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        if f.len() < 7 {
            return None;
        }
        let shell = f[6];
        if shell.ends_with("nologin") || shell.ends_with("false") || shell.is_empty() {
            return None;
        }
        let uid: u32 = f[2].parse().ok()?;
        let full = f[4].split(',').next().unwrap_or("").trim();
        Some(Person { name: f[0].into(), uid, gid: f[3].parse().ok()?, full_name: full.into(), home: f[5].into() })
    }).collect()
}

/// (gid, members) of a group.
fn group(etc: &Path, name: &str) -> Option<(u32, Vec<String>)> {
    fs::read_to_string(etc.join("group")).ok()?.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 4 && f[0] == name).then(|| {
            (f[2].parse().unwrap_or(u32::MAX), f[3].split(',').filter(|s| !s.is_empty()).map(str::to_string).collect())
        })
    })
}

/// Why `p` can't have a desktop, or None if they can. Without a readable
/// shadow file (tests, or not root) the password isn't checked.
pub fn refusal(etc: &Path, p: &Person) -> Option<String> {
    if !(1000..60000).contains(&p.uid) {
        return Some(format!("{} is a system account", p.name));
    }
    match group(etc, "seat") {
        Some((gid, members)) if p.gid == gid || members.iter().any(|m| *m == p.name) => {}
        _ => return Some(format!("{} isn't allowed to use the screen (not in the seat group)", p.name)),
    }
    if let Ok(shadow) = fs::read_to_string(etc.join("shadow")) {
        if let Some(pw) = shadow.lines().find_map(|l| l.strip_prefix(&format!("{}:", p.name)).map(|r| r.split(':').next().unwrap_or(""))) {
            if pw.starts_with('!') || pw.starts_with('*') {
                return Some(format!("{}'s account is turned off or has no password yet", p.name));
            }
        }
    }
    None
}

/// Everyone who may have a desktop, by uid.
pub fn people(etc: &Path) -> Vec<Person> {
    let mut v: Vec<Person> = passwd(etc).into_iter().filter(|p| refusal(etc, p).is_none()).collect();
    v.sort_by_key(|p| p.uid);
    v
}

pub fn find(etc: &Path, name: &str) -> Result<Person, String> {
    let p = passwd(etc).into_iter().find(|p| p.name == name).ok_or_else(|| format!("there is no account called {name}"))?;
    match refusal(etc, &p) {
        Some(why) => Err(why),
        None => Ok(p),
    }
}

/// The person whose desktop starts at boot: uid 1000 (whatever the
/// installer named it), else the first person.
pub fn primary(etc: &Path) -> Option<Person> {
    let all = people(etc);
    all.iter().find(|p| p.uid == 1000).cloned().or_else(|| all.into_iter().next())
}

pub fn display_name(p: &Person) -> String {
    if p.full_name.is_empty() {
        let mut c = p.name.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    } else {
        p.full_name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn fake_etc(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("nexa-sessiond-{tag}-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("passwd"), "root:x:0:0:root:/root:/bin/sh\n\
            messagebus:x:81:81:messagebus:/var/run/dbus:/sbin/nologin\n\
            user:x:1000:1000:Nexa User:/home/user:/bin/sh\n\
            ama:x:1001:1001:Ama Mensah,,,:/home/ama:/bin/sh\n\
            kofi:x:1002:1002::/home/kofi:/bin/sh\n\
            locked:x:1003:1003::/home/locked:/bin/sh\n\
            daemon2:x:1004:1004::/home/d:/sbin/nologin\n").unwrap();
        fs::write(d.join("group"), "root:x:0:\nseat:x:86:user,ama,locked,daemon2\nwheel:x:10:user\n").unwrap();
        fs::write(d.join("shadow"), "user::19000:0:::::\nama:$6$abc:19000:0:::::\nkofi:$6$x:1::::::\nlocked:!:19000::::::\n").unwrap();
        d
    }

    #[test]
    fn who_may_have_a_desktop() {
        let etc = fake_etc("people");
        let names: Vec<String> = people(&etc).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["user", "ama"]);
        assert!(find(&etc, "kofi").unwrap_err().contains("seat group"));
        assert!(find(&etc, "locked").unwrap_err().contains("turned off"));
        assert!(find(&etc, "root").unwrap_err().contains("system account"));
        assert!(find(&etc, "nobody").unwrap_err().contains("no account"));
        assert!(find(&etc, "daemon2").is_err());
        assert_eq!(primary(&etc).unwrap().name, "user");
        assert_eq!(display_name(&find(&etc, "ama").unwrap()), "Ama Mensah");
        assert_eq!(display_name(&passwd(&etc).into_iter().find(|p| p.uid == 1002).unwrap()), "Kofi");
    }
}
