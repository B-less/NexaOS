//! Who is asking (from the kernel, SO_PEERCRED), whether they may ask at
//! all (root, or a member of the wheel group, as the desktop user is), and
//! checking a password against /etc/shadow with the C library's crypt()
//! (SHA-512 "$6$" hashes, as the NexaOS installer writes them).

use std::ffi::{CStr, CString};
use std::fs;
use std::path::Path;

pub const ADMIN_GROUP: &str = "wheel";

/// The user name and the groups (primary and supplementary) of `uid`.
pub fn user_and_groups(uid: u32, etc: &Path) -> Option<(String, Vec<String>)> {
    let passwd = fs::read_to_string(etc.join("passwd")).ok()?;
    let (name, gid) = passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 4 && f[2].parse::<u32>().ok() == Some(uid)).then(|| (f[0].to_string(), f[3].to_string()))
    })?;
    let group = fs::read_to_string(etc.join("group")).unwrap_or_default();
    let groups = group.lines().filter_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 4 && (f[2] == gid || f[3].split(',').any(|m| m == name))).then(|| f[0].to_string())
    }).collect();
    Some((name, groups))
}

pub fn may_ask(uid: u32, etc: &Path) -> bool {
    uid == 0 || user_and_groups(uid, etc).map_or(false, |(_, g)| g.iter().any(|g| g == ADMIN_GROUP))
}

extern "C" {
    fn crypt(key: *const libc::c_char, salt: *const libc::c_char) -> *mut libc::c_char;
}

#[derive(Debug, PartialEq)]
pub enum Check {
    Ok,
    /// The account has no password (the live USB): nothing to check.
    NoPassword,
    Wrong,
    Locked,
}

/// Compare in constant time (no early exit on the first difference).
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn check_password(user: &str, password: Option<&str>, etc: &Path) -> Check {
    let Ok(shadow) = fs::read_to_string(etc.join("shadow")) else { return Check::Locked };
    let Some(hash) = shadow.lines().find_map(|l| {
        let mut f = l.splitn(3, ':');
        (f.next() == Some(user)).then(|| f.next().unwrap_or("").to_string())
    }) else { return Check::Locked };
    if hash.is_empty() {
        return Check::NoPassword;
    }
    if hash.starts_with('!') || hash.starts_with('*') {
        return Check::Locked;
    }
    let Some(pw) = password else { return Check::Wrong };
    let (Ok(k), Ok(s)) = (CString::new(pw), CString::new(hash.as_str())) else { return Check::Wrong };
    // SAFETY: crypt returns a pointer to a static buffer (or NULL); we copy
    // it out immediately. The broker handles one request at a time.
    let out = unsafe { crypt(k.as_ptr(), s.as_ptr()) };
    if out.is_null() {
        return Check::Wrong;
    }
    let got = unsafe { CStr::from_ptr(out) }.to_bytes().to_vec();
    if same(&got, hash.as_bytes()) { Check::Ok } else { Check::Wrong }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn etc(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("nexa-broker-auth-{}-{name}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("passwd"), "root:x:0:0::/root:/bin/sh\nuser:x:1000:1000::/home/user:/bin/sh\nguest:x:1001:1001::/home/guest:/bin/sh\nmessagebus:x:81:81::/:/sbin/nologin\n").unwrap();
        fs::write(d.join("group"), "root:x:0:\nwheel:x:10:user\nuser:x:1000:\nguest:x:1001:\n").unwrap();
        // "secret" hashed with SHA-512 by musl's crypt() with salt "$6$nexasalt".
        fs::write(d.join("shadow"), "root:!::0:::::\nuser:$6$nexasalt$MzZ40LeatUOJUqhCI2oVd1bgvOTb6sBY6vwLqVCZBgpJcOF1s2DBQ8Jftnj.UdYccj83QrC89CZuYVEbDt5Zg1:0:::::\nguest::0:::::\n").unwrap();
        d
    }

    #[test]
    fn only_admins_may_ask() {
        let e = etc("admins");
        assert!(may_ask(0, &e) && may_ask(1000, &e));
        assert!(!may_ask(1001, &e), "not in wheel");
        assert!(!may_ask(81, &e), "a daemon");
        assert!(!may_ask(4242, &e), "unknown uid");
        assert_eq!(user_and_groups(1000, &e).unwrap(), ("user".to_string(), vec!["wheel".to_string(), "user".to_string()]));
    }

    #[test]
    fn passwords() {
        let e = etc("passwords");
        assert_eq!(check_password("guest", None, &e), Check::NoPassword, "live USB: no password set");
        assert_eq!(check_password("root", Some("x"), &e), Check::Locked);
        assert_eq!(check_password("nobody", Some("x"), &e), Check::Locked);
        assert_eq!(check_password("user", None, &e), Check::Wrong);
        assert_eq!(check_password("user", Some("wrong"), &e), Check::Wrong);
        assert_eq!(check_password("user", Some("secret"), &e), Check::Ok);
    }
}
