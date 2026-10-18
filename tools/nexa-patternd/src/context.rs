//! The moment an app is opened in: time of day (30-minute slots), weekday or
//! weekend, the place (which network router you're connected to, as an
//! opaque number - never the network's name), and mains power.

use std::fs;
use std::path::Path;

pub const SLOTS: u8 = 48;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Ctx {
    pub slot: u8,
    pub weekend: bool,
    pub place: u32,
    pub plugged: bool,
}

/// Local time of day and day kind for a Unix time (the system time zone).
pub fn local(ts: u64) -> (u8, bool, u32) {
    let t = ts as i64;
    // SAFETY: tm is plain data that localtime_r fills in.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t as *const i64 as *const _, &mut tm) };
    let minutes = (tm.tm_hour * 60 + tm.tm_min) as u32;
    ((minutes / 30) as u8, tm.tm_wday == 0 || tm.tm_wday == 6, minutes)
}

/// Seconds east of UTC at that moment (the local calendar's offset).
pub fn utc_offset(ts: u64) -> i64 {
    let t = ts as i64;
    // SAFETY: tm is plain data that localtime_r fills in.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t as *const i64 as *const _, &mut tm) };
    tm.tm_gmtoff
}

pub fn hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
}

/// Distance between two slots around the clock (23:30 is next to 00:00).
pub fn slot_distance(a: u8, b: u8) -> u8 {
    let d = a.abs_diff(b);
    d.min(SLOTS - d)
}

/// FNV-1a: a stable, non-reversible-enough number for a router's address.
fn fnv(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in bytes {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The default gateway's hardware address (from /proc/net/route and
/// /proc/net/arp), hashed; 0 when offline or unknown.
pub fn place(proc_root: &Path) -> u32 {
    let route = fs::read_to_string(proc_root.join("net/route")).unwrap_or_default();
    let gw = route.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.get(1) == Some(&"00000000")).then(|| f.get(2).and_then(|g| u32::from_str_radix(g, 16).ok())).flatten()
    });
    let Some(gw) = gw.filter(|g| *g != 0) else { return 0 };
    // /proc/net/route stores the address in network order read as a little-endian number.
    let ip = std::net::Ipv4Addr::from(gw.to_le_bytes());
    let arp = fs::read_to_string(proc_root.join("net/arp")).unwrap_or_default();
    arp.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.first() == Some(&ip.to_string().as_str()) && f.get(3).map_or(false, |m| *m != "00:00:00:00:00:00"))
            .then(|| fnv(f[3].as_bytes()))
    }).unwrap_or(0)
}

/// On mains power?
pub fn plugged(sys: &Path) -> bool {
    fs::read_dir(sys.join("class/power_supply")).into_iter().flatten().flatten().any(|e| {
        fs::read_to_string(e.path().join("type")).map(|t| t.trim() == "Mains").unwrap_or(false)
            && fs::read_to_string(e.path().join("online")).map(|o| o.trim() == "1").unwrap_or(false)
    })
}

pub fn now(proc_root: &Path, sys: &Path, ts: u64) -> Ctx {
    let (slot, weekend, _) = local(ts);
    Ctx { slot, weekend, place: place(proc_root), plugged: plugged(sys) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_and_slots() {
        assert_eq!(slot_distance(0, 47), 1);
        assert_eq!(slot_distance(10, 14), 4);
        assert_eq!(hhmm(8 * 60 + 30), "08:30");
        let (slot, _, minutes) = local(1_700_000_000);
        assert!(slot < SLOTS && minutes < 24 * 60);
    }

    #[test]
    fn router_becomes_a_number() {
        let r = std::env::temp_dir().join(format!("nexa-patternd-ctx-{}", std::process::id()));
        fs::create_dir_all(r.join("net")).unwrap();
        fs::write(r.join("net/route"), "Iface\tDestination\tGateway \tFlags\nwlan0\t0001A8C0\t00000000\t0001\nwlan0\t00000000\t0101A8C0\t0003\n").unwrap();
        fs::write(r.join("net/arp"), "IP address       HW type     Flags       HW address            Mask     Device\n192.168.1.1      0x1         0x2         a4:2b:b0:11:22:33     *        wlan0\n").unwrap();
        let p = place(&r);
        assert!(p != 0);
        assert_eq!(p, fnv(b"a4:2b:b0:11:22:33"), "192.168.1.1's router, hashed");
        fs::write(r.join("net/route"), "Iface\tDestination\tGateway\n").unwrap();
        assert_eq!(place(&r), 0, "offline");
    }
}
