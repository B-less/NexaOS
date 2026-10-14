//! Read-only network facts for the assistant: interfaces and their state,
//! addresses, the default route, DNS servers, Wi-Fi link quality and
//! traffic counters. Straight from the kernel (sysfs, procfs, getifaddrs);
//! nothing is changed. Wi-Fi network names come from iwd, on the Python side.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

/// What the sandbox must allow for `info`.
pub const READ_PATHS: &[&str] = &["/sys", "/proc/net", "/etc/resolv.conf"];

fn read(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn human_bytes(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", units[i]) }
}

/// IPv4 and IPv6 addresses per interface.
fn addresses() -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a linked list we only read, then free.
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return out;
    }
    let mut cur = ifap;
    while !cur.is_null() {
        // SAFETY: cur is a valid node of the list returned above.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() || ifa.ifa_name.is_null() {
            continue;
        }
        // SAFETY: ifa_name is a NUL-terminated string owned by the list.
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
        // SAFETY: the family says which sockaddr type ifa_addr points to.
        let addr = match unsafe { (*ifa.ifa_addr).sa_family } as i32 {
            libc::AF_INET => {
                let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
                Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)).to_string()
            }
            libc::AF_INET6 => {
                let sin6 = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
                let ip = Ipv6Addr::from(sin6.sin6_addr.s6_addr);
                if ip.segments()[0] & 0xffc0 == 0xfe80 { continue } // link-local: noise
                ip.to_string()
            }
            _ => continue,
        };
        out.entry(name).or_default().push(addr);
    }
    // SAFETY: allocated by getifaddrs above.
    unsafe { libc::freeifaddrs(ifap) };
    out
}

/// (interface, gateway) of the IPv4 default route, from /proc/net/route.
fn default_route(route_table: &str) -> Option<(String, String)> {
    route_table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() > 2 && f[1] == "00000000").then(|| {
            let gw = u32::from_str_radix(f[2], 16).unwrap_or(0);
            (f[0].to_string(), Ipv4Addr::from(u32::from_be(gw)).to_string())
        })
    })
}

/// Link quality in percent per wireless interface, from /proc/net/wireless.
fn wireless_quality(table: &str) -> BTreeMap<String, u32> {
    table.lines().skip(2).filter_map(|l| {
        let (name, rest) = l.split_once(':')?;
        let link: f64 = rest.split_whitespace().nth(1)?.trim_end_matches('.').parse().ok()?;
        Some((name.trim().to_string(), ((link / 70.0) * 100.0).round().min(100.0) as u32))
    }).collect()
}

fn dns(resolv: &str) -> Vec<String> {
    resolv.lines().filter_map(|l| l.strip_prefix("nameserver").map(|s| s.trim().to_string())).collect()
}

pub fn info() -> Value {
    let addrs = addresses();
    let route = default_route(&read("/proc/net/route").unwrap_or_default());
    let quality = wireless_quality(&read("/proc/net/wireless").unwrap_or_default());
    let mut ifaces = Vec::new();
    let mut names: Vec<String> = fs::read_dir("/sys/class/net").into_iter().flatten().flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    for name in names {
        if name == "lo" {
            continue;
        }
        let dir = Path::new("/sys/class/net").join(&name);
        let wireless = dir.join("wireless").exists() || dir.join("phy80211").exists();
        let virtual_if = fs::read_link(&dir).map(|t| t.to_string_lossy().contains("/virtual/")).unwrap_or(false);
        let kind = if wireless { "wifi" } else if name.starts_with("wg") || name.starts_with("tun") { "vpn" }
                   else if virtual_if { "virtual" } else { "wired" };
        let state = read(dir.join("operstate")).unwrap_or_else(|| "unknown".into());
        let rx: u64 = read(dir.join("statistics/rx_bytes")).and_then(|s| s.parse().ok()).unwrap_or(0);
        let tx: u64 = read(dir.join("statistics/tx_bytes")).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut i = json!({
            "name": name, "kind": kind, "state": state,
            "addresses": addrs.get(&name).cloned().unwrap_or_default(),
            "mac": read(dir.join("address")),
            "received": human_bytes(rx), "sent": human_bytes(tx),
        });
        if let Some(speed) = read(dir.join("speed")).and_then(|s| s.parse::<i64>().ok()).filter(|s| *s > 0) {
            i["speed"] = json!(format!("{speed} Mb/s"));
        }
        if let Some(q) = quality.get(&name) {
            i["signal_percent"] = json!(q);
        }
        ifaces.push(i);
    }
    let online = route.is_some();
    let summary = match &route {
        Some((dev, gw)) => {
            let kind = ifaces.iter().find(|i| i["name"] == dev.as_str()).and_then(|i| i["kind"].as_str()).unwrap_or("network");
            format!("Online through {dev} ({kind}), gateway {gw}")
        }
        None => "Offline: no default route".into(),
    };
    json!({
        "summary": summary, "online": online,
        "default_route": route.map(|(dev, gw)| json!({"interface": dev, "gateway": gw})),
        "dns": dns(&read("/etc/resolv.conf").unwrap_or_default()),
        "interfaces": ifaces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_default_route() {
        let t = "Iface\tDestination\tGateway \tFlags\n\
                 wlan0\t0000A8C0\t00000000\t0001\n\
                 wlan0\t00000000\t0100A8C0\t0003\n";
        assert_eq!(default_route(t), Some(("wlan0".into(), "192.168.0.1".into())));
        assert_eq!(default_route("Iface\tDestination\n"), None);
    }

    #[test]
    fn parses_wireless_quality() {
        let t = "Inter-| sta-|   Quality        |\n face | tus | link level noise |\n wlan0: 0000   56.  -54.  -256        0      0      0\n";
        assert_eq!(wireless_quality(t).get("wlan0"), Some(&80));
    }

    #[test]
    fn dns_and_sizes() {
        assert_eq!(dns("# x\nnameserver 1.1.1.1\nnameserver 9.9.9.9\n"), vec!["1.1.1.1", "9.9.9.9"]);
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(12), "12 B");
    }

    #[test]
    fn live_info_has_a_shape() {
        let v = info();
        assert!(v["summary"].is_string() && v["interfaces"].is_array());
    }
}
