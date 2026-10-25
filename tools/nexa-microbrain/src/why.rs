//! Why (Magic Layer): plain answers from real evidence. Every finding says
//! whether it is **proven** (measured just now, or recorded when it
//! happened) or only **possible** (fits what is seen, but isn't shown to be
//! the cause). No guessing dressed up as fact.
//!
//!   {"cmd": "why", "topic": "slow" | "crash" | "memory" | "network" | "stopped"}
//!
//! Evidence: /proc (processes, pressure stall information, the kernel's
//! out-of-memory count), /sys (network links, Wi-Fi switches, processor
//! speed and temperature), /etc/resolv.conf and one DNS question to the
//! server the system already uses, MicroBrain's own decisions and jobs, and
//! the desktop's record of apps that ended abnormally
//! (~/.local/state/nexa/exits.jsonl, written by nexa_common.exits).

use crate::agents::{Coordinator, Decision};
use crate::resources::Level;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const TOPICS: &[(&str, &str)] = &[
    ("slow", "Why is it slow?"),
    ("crash", "Why did an app close?"),
    ("memory", "What is using the memory?"),
    ("network", "Why is there no internet?"),
    ("stopped", "Why did something stop?"),
];

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Finding {
    /// "proven" or "possible".
    pub certainty: &'static str,
    pub text: String,
    /// What it rests on, in a few words.
    pub evidence: String,
}

fn proven(text: impl Into<String>, evidence: impl Into<String>) -> Finding {
    Finding { certainty: "proven", text: text.into(), evidence: evidence.into() }
}

fn possible(text: impl Into<String>, evidence: impl Into<String>) -> Finding {
    Finding { certainty: "possible", text: text.into(), evidence: evidence.into() }
}

/// Where the evidence is (tests point these at made-up trees).
pub struct Sources {
    pub proc_root: PathBuf,
    pub sys_root: PathBuf,
    pub home: PathBuf,
    pub resolv: PathBuf,
    /// How long to watch the processor for "slow".
    pub cpu_window: Duration,
    /// Ask the DNS server (off in tests that have none).
    pub dns_probe: bool,
}

impl Sources {
    pub fn real(home: PathBuf) -> Sources {
        let p = crate::resources::Paths::real();
        Sources { proc_root: p.proc_root, sys_root: p.sys_root, home, resolv: "/etc/resolv.conf".into(),
                  cpu_window: Duration::from_millis(400), dns_probe: true }
    }
}

// ---- processes ---------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub state: char,
    pub jiffies: u64,
    pub rss_kb: u64,
    pub uid: u32,
}

/// The name people know: Nexa apps run as `python3 -m nexa_files`, which
/// the kernel calls "python3"; their module says which app it is.
fn display_name(dir: &Path, comm: &str) -> String {
    if comm.starts_with("python") {
        let cmd = std::fs::read(dir.join("cmdline")).unwrap_or_default();
        let args: Vec<String> = cmd.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        if let Some(i) = args.iter().position(|a| a == "-m") {
            if let Some(m) = args.get(i + 1) {
                return m.split('.').next().unwrap_or(m).replace('_', "-");
            }
        }
        if let Some(script) = args.get(1).filter(|a| !a.starts_with('-')) {
            return Path::new(script).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| comm.into());
        }
    }
    comm.to_string()
}

pub fn procs(proc_root: &Path) -> Vec<Proc> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: sysconf has no failure mode that matters here.
    let page_kb = (unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) / 1024) as u64;
    let mut out = Vec::new();
    for e in std::fs::read_dir(proc_root).into_iter().flatten().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Ok(stat) = std::fs::read_to_string(e.path().join("stat")) else { continue };
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
        let comm = &stat[open + 1..close];
        let f: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        if f.len() < 22 {
            continue;
        }
        let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
        out.push(Proc {
            pid,
            name: display_name(&e.path(), comm),
            state: f[0].chars().next().unwrap_or('?'),
            jiffies: n(11) + n(12),
            rss_kb: n(21) * page_kb,
            uid: e.metadata().map(|m| m.uid()).unwrap_or(u32::MAX),
        });
    }
    out
}

/// Processor use per program over `window`, % of one core, busiest first.
pub fn cpu_use(proc_root: &Path, window: Duration) -> Vec<(String, f64)> {
    let a: HashMap<u32, u64> = procs(proc_root).into_iter().map(|p| (p.pid, p.jiffies)).collect();
    let t = Instant::now();
    std::thread::sleep(window);
    let secs = t.elapsed().as_secs_f64().max(0.05);
    // SAFETY: as above.
    let tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let mut by: BTreeMap<String, f64> = BTreeMap::new();
    for p in procs(proc_root) {
        if let Some(j0) = a.get(&p.pid) {
            *by.entry(p.name).or_default() += p.jiffies.saturating_sub(*j0) as f64 / tck / secs * 100.0;
        }
    }
    let mut v: Vec<(String, f64)> = by.into_iter().collect();
    v.sort_by(|x, y| y.1.total_cmp(&x.1));
    v
}

fn memory_by_program(ps: &[Proc]) -> Vec<(String, u64)> {
    let mut by: BTreeMap<String, u64> = BTreeMap::new();
    for p in ps.iter().filter(|p| p.rss_kb > 0) {
        *by.entry(p.name.clone()).or_default() += p.rss_kb;
    }
    let mut v: Vec<(String, u64)> = by.into_iter().collect();
    v.sort_by(|x, y| y.1.cmp(&x.1));
    v
}

fn gb(mb: u64) -> String {
    if mb >= 1024 { format!("{:.1} GB", mb as f64 / 1024.0) } else { format!("{mb} MB") }
}

/// "14:05" in local time.
#[allow(deprecated)] // libc's musl time_t note: it is 64-bit on our musl either way
pub fn clock(at: u64) -> String {
    let t = at as libc::time_t;
    // SAFETY: localtime_r writes only into tm.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return format!("{at}");
    }
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

// ---- slow -----------------------------------------------------------------------

pub fn slow(c: &Coordinator, src: &Sources) -> Vec<Finding> {
    let mut out = Vec::new();
    let busy = cpu_use(&src.proc_root, src.cpu_window);
    for (name, pct) in busy.iter().take(3).filter(|(_, p)| *p >= 25.0) {
        out.push(proven(format!("{name} is using {pct:.0} % of a processor core."),
                        format!("measured over {:.1} s in /proc", src.cpu_window.as_secs_f64())));
    }
    if let Some(s) = &c.monitor.last {
        if s.psi_cpu >= 20.0 {
            out.push(proven(format!("Programs are waiting for the processor ({:.0} % of the last 10 s).", s.psi_cpu),
                            "the kernel's pressure figures (/proc/pressure/cpu)"));
        }
        if s.psi_mem >= 5.0 || c.monitor.mem_level >= Level::High {
            out.push(proven(format!("Programs are waiting for memory ({:.0} % of the last 10 s); {} is free.",
                                    s.psi_mem, gb(s.mem_avail_mb)), "/proc/pressure/memory and /proc/meminfo"));
        }
        let swap = s.swap_total_mb.saturating_sub(s.swap_free_mb);
        if swap >= 256 && s.psi_mem >= 1.0 {
            out.push(possible(format!("{} of memory is in swap: going back to an app that was moved there is slow.", gb(swap)),
                              "swap in use while programs wait for memory"));
        }
        if s.psi_io >= 10.0 {
            out.push(proven(format!("Programs are waiting for the disk ({:.0} % of the last 10 s).", s.psi_io),
                            "/proc/pressure/io"));
        }
        if c.monitor.disk_low {
            out.push(proven(format!("Storage is nearly full ({} free): some apps slow down or can't save.", gb(s.disk_free_mb)),
                            "free space in your home folder"));
        }
        // Slow on purpose: a battery or low-resource mode holds things back.
        if c.budget.jobs == 0 || !c.budget.indexing {
            out.push(proven(format!("NexaOS is holding background work back: {}.", c.budget.why), "MicroBrain's current mode"));
        }
        if s.cpu_busy >= 50.0 {
            let cpu = src.sys_root.join("devices/system/cpu/cpu0/cpufreq");
            let num = |f: &str| read(&cpu.join(f)).and_then(|v| v.parse::<f64>().ok());
            if let (Some(cur), Some(max)) = (num("scaling_cur_freq"), num("cpuinfo_max_freq").filter(|m| *m > 0.0)) {
                if cur / max < 0.6 {
                    out.push(possible(format!("The processor is busy but running at {:.0} % of its top speed (power saving or heat).",
                                              100.0 * cur / max), "cpufreq: current and top speed"));
                }
            }
        }
    }
    for p in procs(&src.proc_root).iter().filter(|p| p.state == 'D').take(3) {
        out.push(proven(format!("{} is waiting for a disk or device.", p.name), format!("state D in /proc/{}/stat", p.pid)));
    }
    if let Some((zone, temp)) = hottest(&src.sys_root).filter(|(_, t)| *t >= 90.0) {
        out.push(possible(format!("The processor is hot ({temp:.0} °C): it slows itself down to cool off."),
                          format!("{zone} in /sys/class/thermal")));
    }
    if out.is_empty() {
        out.push(possible("Nothing measurable is short right now: the processor, memory and disk all have room. \
                           The app itself may be busy, or waiting for the network.",
                          "processor, memory and disk measured just now"));
    }
    out
}

fn hottest(sys: &Path) -> Option<(String, f64)> {
    std::fs::read_dir(sys.join("class/thermal")).ok()?.flatten()
        .filter_map(|e| {
            let kind = read(&e.path().join("type")).unwrap_or_default();
            let t = read(&e.path().join("temp"))?.parse::<f64>().ok()? / 1000.0;
            (kind.contains("pkg") || kind.contains("cpu") || kind.contains("x86")).then_some((kind, t))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

// ---- memory ---------------------------------------------------------------------

pub fn memory(c: &Coordinator, src: &Sources, now: u64) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(s) = &c.monitor.last {
        let used = s.mem_total_mb.saturating_sub(s.mem_avail_mb);
        out.push(proven(format!("{} of {} is in use; {} is available.", gb(used), gb(s.mem_total_mb), gb(s.mem_avail_mb)),
                        "/proc/meminfo"));
        let swap = s.swap_total_mb.saturating_sub(s.swap_free_mb);
        if swap >= 64 {
            out.push(proven(format!("{} has been moved to swap.", gb(swap)), "/proc/meminfo"));
        }
    }
    for (name, kb) in memory_by_program(&procs(&src.proc_root)).into_iter().take(5) {
        out.push(proven(format!("{name}: {}", gb(kb / 1024)), "memory in use by its processes (/proc)"));
    }
    if let Some(f) = c.monitor.memory_forecast() {
        if let Some(m) = f.minutes_left.filter(|_| f.confidence >= 0.6) {
            out.push(possible(format!("At this rate memory runs short in about {m:.0} minutes."),
                              format!("the trend of the last minutes ({:.0} % sure)", f.confidence * 100.0)));
        }
    }
    let ooms: u64 = c.monitor.ooms.iter().filter(|o| now.saturating_sub(o.1) < 86400).map(|o| o.2).sum();
    if ooms > 0 {
        out.push(proven(format!("Memory ran out today: the kernel had to end a program {ooms} time(s)."),
                        "the kernel's out-of-memory count (/proc/vmstat)"));
    }
    out
}

// ---- crash ----------------------------------------------------------------------

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Exit {
    pub at: u64,
    pub app: String,
    #[serde(default)]
    pub signal: Option<i32>,
    #[serde(default)]
    pub code: Option<i32>,
    #[serde(default)]
    pub ran_s: f64,
    #[serde(default)]
    pub pid: u32,
    /// Set when the person ended it themselves ("Pulse").
    #[serde(default)]
    pub by: Option<String>,
}

pub fn exits_path(home: &Path) -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/state"))
        .join("nexa/exits.jsonl")
}

pub fn read_exits(home: &Path) -> Vec<Exit> {
    let text = std::fs::read_to_string(exits_path(home)).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(200)..].iter().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn signal_name(n: i32) -> &'static str {
    match n {
        4 => "SIGILL", 6 => "SIGABRT", 7 => "SIGBUS", 8 => "SIGFPE", 9 => "SIGKILL", 11 => "SIGSEGV", 15 => "SIGTERM",
        _ => "a signal",
    }
}

fn decided_about<'a>(log: &'a std::collections::VecDeque<Decision>, app: &str, at: u64) -> Option<&'a Decision> {
    log.iter().rev().find(|d| d.at.abs_diff(at) <= 30 && d.what.contains(app))
}

pub fn crash(c: &Coordinator, src: &Sources, now: u64) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut matched = vec![false; c.monitor.ooms.len()];
    let exits: Vec<Exit> = read_exits(&src.home).into_iter().filter(|e| now.saturating_sub(e.at) < 86400).collect();
    // Ended by the person (Pulse): that explains the matching kill.
    let by_you: Vec<&Exit> = exits.iter().filter(|e| e.by.is_some()).collect();
    let explained = |e: &Exit| e.by.is_none() && by_you.iter().any(|y| y.pid == e.pid && e.pid != 0 && e.at.abs_diff(y.at) <= 60);
    for e in exits.iter().rev().filter(|e| !explained(e)).take(8) {
        let when = clock(e.at);
        if let Some(by) = &e.by {
            let how = if e.signal == Some(9) { "Force quit" } else { "End task" };
            out.push(proven(format!("{} was ended at {when} by you, with {how} in {by}.", e.app), format!("{by}'s record")));
            continue;
        }
        match e.signal {
            Some(9) => {
                let oom = c.monitor.ooms.iter().position(|(from, to, _)| e.at + 5 >= *from && e.at <= to + 5);
                if let Some(i) = oom {
                    matched[i] = true;
                    out.push(proven(format!("{} was ended by the kernel at {when}: memory ran out.", e.app),
                                    "it was killed while the kernel's out-of-memory count rose (/proc/vmstat)"));
                } else if let Some(d) = decided_about(&c.log, &e.app, e.at) {
                    out.push(proven(format!("{} was ended at {when} by MicroBrain: {}.", e.app, d.why), "MicroBrain's decision log"));
                } else {
                    out.push(possible(format!("{} was ended forcefully at {when} (from Pulse, another program, or the system).", e.app),
                                      "it ended with SIGKILL; no record says who sent it"));
                }
            }
            Some(s @ (4 | 7 | 8 | 11)) => out.push(proven(format!("{} crashed at {when}: a fault in the program.", e.app),
                                                          format!("it ended with {}", signal_name(s)))),
            Some(6) => out.push(proven(format!("{} stopped itself at {when} after an error it couldn't handle.", e.app),
                                       "it ended with SIGABRT")),
            Some(15) => out.push(possible(format!("{} was asked to close at {when} (by logging out, Pulse or another program).", e.app),
                                          "it ended with SIGTERM")),
            Some(s) => out.push(proven(format!("{} ended at {when} with signal {s}.", e.app), "the desktop's record")),
            None => {
                let code = e.code.unwrap_or(-1);
                if e.ran_s < 10.0 {
                    out.push(proven(format!("{} closed right after starting at {when} (error code {code}).", e.app),
                                    format!("it ran {:.1} s", e.ran_s)));
                    out.push(possible("A missing file, setting or permission is the usual reason; starting it from the Terminal shows its message.",
                                      "what an error at start usually means"));
                } else {
                    out.push(proven(format!("{} ended at {when} with error code {code}.", e.app), format!("it ran {:.0} s", e.ran_s)));
                }
            }
        }
    }
    for ((_, to, n), _) in c.monitor.ooms.iter().zip(&matched).filter(|(o, m)| !**m && now.saturating_sub(o.1) < 86400) {
        out.push(proven(format!("Memory ran out at {}: the kernel ended {n} program(s).", clock(*to)),
                        "the kernel's out-of-memory count (/proc/vmstat)"));
    }
    if out.is_empty() {
        out.push(proven("No app started from the desktop ended abnormally in the last 24 hours.",
                        "the desktop's record of how apps ended"));
    }
    out
}

// ---- network --------------------------------------------------------------------

struct Link {
    name: String,
    up: bool,
    carrier: Option<bool>,
    wireless: bool,
    state: String,
}

fn links(sys: &Path) -> Vec<Link> {
    let mut all: Vec<(Link, bool)> = std::fs::read_dir(sys.join("class/net")).into_iter().flatten().flatten()
        .filter(|e| e.file_name() != "lo")
        .map(|e| {
            let p = e.path();
            let state = read(&p.join("operstate")).unwrap_or_default();
            let carrier = read(&p.join("carrier")).map(|c| c == "1");
            let wireless = p.join("wireless").exists() || p.join("phy80211").exists();
            let physical = wireless || p.join("device").exists();
            let up = state == "up" || (state == "unknown" && carrier == Some(true));
            (Link { name: e.file_name().to_string_lossy().into_owned(), up, carrier, wireless, state }, physical)
        })
        .collect();
    // Bridges and tunnels only count when there is nothing physical.
    if all.iter().any(|(_, phys)| *phys) {
        all.retain(|(_, phys)| *phys);
    }
    let mut v: Vec<Link> = all.into_iter().map(|(l, _)| l).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn wifi_blocked(sys: &Path) -> Option<&'static str> {
    for e in std::fs::read_dir(sys.join("class/rfkill")).into_iter().flatten().flatten() {
        if read(&e.path().join("type")).as_deref() != Some("wlan") {
            continue;
        }
        if read(&e.path().join("hard")).as_deref() == Some("1") {
            return Some("a hardware switch");
        }
        if read(&e.path().join("soft")).as_deref() == Some("1") {
            return Some("airplane mode or the Wi-Fi switch");
        }
    }
    None
}

/// IPv4 addresses per interface, and whether one has a global IPv6 address.
fn addresses(proc_root: &Path) -> (HashMap<String, Vec<String>>, HashMap<String, bool>) {
    let mut v4: HashMap<String, Vec<String>> = HashMap::new();
    // SAFETY: getifaddrs allocates a list we free below; we only read it.
    unsafe {
        let mut ifa: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifa) == 0 {
            let mut p = ifa;
            while !p.is_null() {
                let a = &*p;
                if !a.ifa_addr.is_null() && (*a.ifa_addr).sa_family as i32 == libc::AF_INET {
                    let sin = &*(a.ifa_addr as *const libc::sockaddr_in);
                    let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                    let name = std::ffi::CStr::from_ptr(a.ifa_name).to_string_lossy().into_owned();
                    v4.entry(name).or_default().push(ip.to_string());
                }
                p = a.ifa_next;
            }
            libc::freeifaddrs(ifa);
        }
    }
    // /proc/net/if_inet6: address index prefix scope flags name; scope 00 = global.
    let mut v6 = HashMap::new();
    for l in std::fs::read_to_string(proc_root.join("net/if_inet6")).unwrap_or_default().lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() == 6 && f[3] == "00" {
            v6.insert(f[5].to_string(), true);
        }
    }
    (v4, v6)
}

fn default_route(proc_root: &Path) -> Option<String> {
    for l in std::fs::read_to_string(proc_root.join("net/route")).unwrap_or_default().lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() > 2 && f[1] == "00000000" {
            let gw = u32::from_str_radix(f[2], 16).ok()?;
            return Some(format!("{} via {}", std::net::Ipv4Addr::from(u32::from_be(gw)), f[0]));
        }
    }
    let v6 = std::fs::read_to_string(proc_root.join("net/ipv6_route")).unwrap_or_default();
    v6.lines().find(|l| l.starts_with("00000000000000000000000000000000 00 ") && !l.trim_end().ends_with(" lo"))
        .map(|l| format!("IPv6 via {}", l.split_whitespace().last().unwrap_or("?")))
}

pub fn nameservers(resolv: &Path) -> Vec<String> {
    std::fs::read_to_string(resolv).unwrap_or_default().lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// One DNS question, the way apps ask: the address of example.org (a name
/// reserved for this, which always exists), retried like a resolver does
/// (a first packet can be dropped). Ok((ms, rcode)) once anything answers.
pub fn dns_probe(server: &str, per_try: Duration, tries: u32) -> Result<(u128, u8), String> {
    let ip: std::net::IpAddr = server.split('%').next().unwrap_or(server).parse().map_err(|_| format!("{server} isn't an address"))?;
    dns_probe_at(SocketAddr::new(ip, 53), per_try, tries)
}

pub fn dns_probe_at(to: SocketAddr, per_try: Duration, tries: u32) -> Result<(u128, u8), String> {
    let local = if to.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let sock = UdpSocket::bind(local).map_err(|e| e.to_string())?;
    sock.set_read_timeout(Some(per_try)).map_err(|e| e.to_string())?;
    let mut seed = [0u8; 2];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut seed));
    let id = u16::from_be_bytes(seed) ^ (std::process::id() as u16);
    let mut q = Vec::with_capacity(29);
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]); // recursion desired, one question
    for label in ["example", "org"] {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]); // end of name, A, IN
    let t = Instant::now();
    let mut buf = [0u8; 512];
    for _ in 0..tries.max(1) {
        sock.send_to(&q, to).map_err(|e| e.to_string())?;
        let sent = Instant::now();
        while sent.elapsed() < per_try {
            match sock.recv_from(&mut buf) {
                Ok((n, from)) if n >= 12 && from.ip() == to.ip() && buf[0..2] == id.to_be_bytes() && buf[2] & 0x80 != 0 => {
                    return Ok((t.elapsed().as_millis(), buf[3] & 0x0f));
                }
                Ok(_) => continue, // someone else's packet
                Err(_) => break,   // this try timed out
            }
        }
    }
    Err("no answer".into())
}

pub fn network(src: &Sources) -> Vec<Finding> {
    let mut out = Vec::new();
    let ls = links(&src.sys_root);
    if ls.is_empty() {
        return vec![proven("This computer has no network adapter that Linux can use.", "/sys/class/net")];
    }
    let up: Vec<&Link> = ls.iter().filter(|l| l.up).collect();
    if up.is_empty() {
        if let Some(how) = ls.iter().any(|l| l.wireless).then(|| wifi_blocked(&src.sys_root)).flatten() {
            out.push(proven(format!("Wi-Fi is switched off by {how}."), "rfkill in /sys/class/rfkill"));
        }
        for l in &ls {
            if l.wireless {
                out.push(proven(format!("Wi-Fi ({}) is not connected to a network.", l.name),
                                format!("operstate \"{}\" in /sys/class/net/{}", l.state, l.name)));
            } else if l.carrier == Some(false) {
                out.push(proven(format!("The cable on {} is unplugged, or the other end is switched off.", l.name),
                                format!("no carrier on /sys/class/net/{}", l.name)));
            } else {
                out.push(proven(format!("{} is down.", l.name), format!("operstate \"{}\"", l.state)));
            }
        }
        return out;
    }
    let (v4, v6) = addresses(&src.proc_root);
    let mut addressed = Vec::new();
    for l in &up {
        let good: Vec<&String> = v4.get(&l.name).map(|v| v.iter().filter(|a| !a.starts_with("169.254.")).collect()).unwrap_or_default();
        if let Some(a) = good.first() {
            addressed.push(format!("{} ({a})", l.name));
        } else if v6.get(&l.name).copied().unwrap_or(false) {
            addressed.push(format!("{} (IPv6)", l.name));
        } else {
            out.push(proven(format!("{} is connected but got no address from the network: its router didn't answer (DHCP).", l.name),
                            format!("no usable address on {}", l.name)));
        }
    }
    if addressed.is_empty() {
        return out;
    }
    let route = default_route(&src.proc_root);
    if route.is_none() {
        out.push(proven("There's no way out of the local network (no default route).", "/proc/net/route"));
        return out;
    }
    let ns = nameservers(&src.resolv);
    if ns.is_empty() {
        out.push(proven("No DNS server is set, so names like example.org can't be looked up.",
                        format!("no nameserver in {}", src.resolv.display())));
        return out;
    }
    let mut names_ok = None;
    if src.dns_probe {
        match dns_probe(&ns[0], Duration::from_secs(1), 3) {
            Ok((ms, 0)) => names_ok = Some(format!("the DNS server {} answered in {ms} ms", ns[0])),
            Ok((_, rc)) => out.push(proven(format!("The DNS server {} answers, but with an error (code {rc}): it can't look names up.", ns[0]),
                                           "a DNS question for example.org, just now")),
            Err(_) => {
                out.push(proven(format!("The DNS server {} didn't answer (3 tries, 1 s each): names can't be looked up.", ns[0]),
                                "a DNS question for example.org, just now"));
                out.push(possible("If this network needs a sign-in page (a hotel or a café), open the browser.",
                                  "networks that hold traffic until you sign in behave like this"));
            }
        }
    }
    if out.is_empty() {
        let names = names_ok.unwrap_or_else(|| "DNS not asked".into());
        out.push(possible(format!("The connection looks fine from here: {} is up, the way out is {}, {names}. \
                                   A site that fails may be down itself.", addressed.join(", "), route.unwrap_or_default()),
                          "link, address, route and DNS checked just now"));
    }
    out
}

// ---- stopped --------------------------------------------------------------------

pub fn stopped(c: &Coordinator, src: &Sources, now: u64) -> Vec<Finding> {
    let mut out = Vec::new();
    let mine: Vec<Proc> = procs(&src.proc_root).into_iter().filter(|p| p.uid == c.uid).collect();
    for p in mine.iter().filter(|p| p.state == 'T' || p.state == 't').take(5) {
        match decided_about(&c.log, &p.name, now).or_else(|| c.log.iter().rev().find(|d| d.what.contains(&p.name))) {
            Some(d) => out.push(proven(format!("{} is paused by MicroBrain: {}.", p.name, d.why), "MicroBrain's decision log")),
            None => out.push(proven(format!("{} is paused (stopped), not closed.", p.name), format!("state T in /proc/{}/stat", p.pid))),
        }
    }
    for p in mine.iter().filter(|p| p.state == 'D').take(3) {
        out.push(proven(format!("{} is waiting for a disk or device.", p.name), format!("state D in /proc/{}/stat", p.pid)));
    }
    for j in c.scheduler.waiting() {
        let why = j.waiting.clone().unwrap_or_else(|| "its turn hasn't come".into());
        out.push(proven(format!("MicroBrain's job \"{}\" is waiting: {why}.", j.name), "MicroBrain's job queue"));
    }
    for d in c.log.iter().rev().filter(|d| now.saturating_sub(d.at) < 1800).take(5) {
        out.push(proven(format!("{} at {}: {}.", d.what, clock(d.at), d.why), format!("MicroBrain's decision log ({})", d.agent)));
    }
    for e in read_exits(&src.home).iter().rev().filter(|e| now.saturating_sub(e.at) < 1800).take(3) {
        out.push(proven(format!("{} ended at {} (see \"Why did an app close?\").", e.app, clock(e.at)), "the desktop's record"));
    }
    if out.is_empty() {
        out.push(proven("Nothing of yours is paused or waiting right now, and nothing was stopped in the last 30 minutes.",
                        "every process of yours, MicroBrain's jobs and decisions"));
    }
    out
}

pub fn answer(topic: &str, c: &Coordinator, src: &Sources, now: u64) -> Option<Vec<Finding>> {
    Some(match topic {
        "slow" => slow(c, src),
        "memory" => memory(c, src, now),
        "crash" => crash(c, src, now),
        "network" => network(src),
        "stopped" => stopped(c, src, now),
        _ => return None,
    })
}

/// The findings as plain text (Nexus shows answers as text).
pub fn as_text(findings: &[Finding]) -> String {
    findings.iter().map(|f| format!("{} {}", if f.certainty == "proven" { "•" } else { "◦ Possibly:" }, f.text))
        .collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Coordinator;
    use crate::config::Profile;
    use crate::pulse::Mode;
    use crate::resources::Sample;

    fn tree() -> PathBuf {
        let d = std::env::temp_dir().join(format!("why-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn stat(proc_root: &Path, pid: u32, comm: &str, state: char, jiffies: u64, rss_pages: u64) {
        // pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime stime ... rss(24th)
        let line = format!("{pid} ({comm}) {state} 1 1 1 0 -1 0 0 0 0 0 {jiffies} 0 0 0 20 0 1 0 100 1000 {rss_pages} 0");
        put(&proc_root.join(format!("{pid}/stat")), &line);
    }

    fn coord(proc_root: &Path) -> Coordinator {
        Coordinator::new(Profile::Medium, Mode::Balanced, vec![], proc_root.to_path_buf(), u32::MAX)
    }

    fn src(root: &Path) -> Sources {
        Sources { proc_root: root.join("proc"), sys_root: root.join("sys"), home: root.join("home"),
                  resolv: root.join("resolv.conf"), cpu_window: Duration::from_millis(10), dns_probe: false }
    }

    #[test]
    fn processes_are_read_and_python_apps_named() {
        let r = tree();
        stat(&r.join("proc"), 10, "python3", 'S', 50, 2560);
        std::fs::write(r.join("proc/10/cmdline"), b"python3\0-m\0nexa_files\0").unwrap();
        stat(&r.join("proc"), 11, "foot (term)", 'T', 5, 256);
        let ps = procs(&r.join("proc"));
        let files = ps.iter().find(|p| p.pid == 10).unwrap();
        assert_eq!(files.name, "nexa-files");
        assert_eq!(files.jiffies, 50);
        assert!(files.rss_kb >= 2560 * 4);
        assert_eq!(ps.iter().find(|p| p.pid == 11).unwrap().name, "foot (term)");
        assert_eq!(ps.iter().find(|p| p.pid == 11).unwrap().state, 'T');
    }

    #[test]
    fn memory_lists_the_biggest_programs() {
        let r = tree();
        stat(&r.join("proc"), 20, "firefox", 'S', 0, 100_000);
        stat(&r.join("proc"), 21, "firefox", 'S', 0, 50_000);
        stat(&r.join("proc"), 22, "foot", 'S', 0, 1_000);
        let mut c = coord(&r.join("proc"));
        c.monitor.push(Sample { at: 1, mem_total_mb: 4000, mem_avail_mb: 1000, ..Default::default() });
        let f = memory(&c, &src(&r), 2);
        assert!(f[0].text.starts_with("2.9 GB of 3.9 GB is in use"), "{:?}", f[0]);
        assert!(f[1].text.starts_with("firefox: "), "the biggest (both processes) first: {:?}", f[1]);
        assert!(f.iter().all(|x| x.certainty == "proven"));
    }

    #[test]
    fn crash_tells_oom_from_faults_and_unknown_kills() {
        let r = tree();
        let home = r.join("home");
        put(&exits_path(&home), &[
            r#"{"at": 1000, "app": "nexa-files", "signal": 11, "ran_s": 30}"#,
            r#"{"at": 2000, "app": "firefox", "signal": 9, "ran_s": 900}"#,
            r#"{"at": 3000, "app": "nexa-notes", "signal": 9, "ran_s": 60}"#,
            r#"{"at": 3500, "app": "nexa-calc", "code": 1, "ran_s": 0.4}"#,
            r#"{"at": 3600, "app": "foot", "pid": 77, "signal": 9, "ran_s": 300}"#,
            r#"{"at": 3601, "app": "Terminal", "pid": 77, "signal": 9, "by": "Pulse"}"#,
            "not json",
        ].join("\n"));
        let mut c = coord(&r.join("proc"));
        c.monitor.push(Sample { at: 1990, oom_kills: 4, ..Default::default() });
        c.monitor.push(Sample { at: 2003, oom_kills: 5, ..Default::default() });
        let f = crash(&c, &src(&r), 4000);
        let find = |app: &str| f.iter().find(|x| x.text.starts_with(app)).unwrap_or_else(|| panic!("{app}: {f:?}")).clone();
        assert_eq!(find("firefox").certainty, "proven");
        assert!(find("firefox").text.contains("memory ran out"));
        assert_eq!(find("nexa-notes").certainty, "possible", "a kill with no evidence of who: only possible");
        assert!(find("nexa-files").text.contains("crashed"));
        assert!(find("nexa-calc").text.contains("right after starting"));
        assert!(find("Terminal").text.contains("by you, with Force quit in Pulse"), "{f:?}");
        assert!(!f.iter().any(|x| x.text.starts_with("foot")), "the kill is explained by Pulse's record, not repeated");
        assert!(!f.iter().any(|x| x.text.starts_with("Memory ran out at")), "the out-of-memory event is explained by firefox");
        // Over a day later: nothing.
        let later = crash(&c, &src(&r), 200_000);
        assert_eq!(later.len(), 1);
        assert!(later[0].text.starts_with("No app"));
    }

    #[test]
    fn network_finds_the_first_missing_piece() {
        let r = tree();
        let s = src(&r);
        let net = r.join("sys/class/net");
        // A cable unplugged, Wi-Fi switched off.
        put(&net.join("eth0/operstate"), "down");
        put(&net.join("eth0/carrier"), "0");
        std::fs::create_dir_all(net.join("eth0/device")).unwrap();
        put(&net.join("wlan0/operstate"), "down");
        std::fs::create_dir_all(net.join("wlan0/wireless")).unwrap();
        put(&r.join("sys/class/rfkill/rfkill0/type"), "wlan");
        put(&r.join("sys/class/rfkill/rfkill0/soft"), "1");
        put(&r.join("sys/class/rfkill/rfkill0/hard"), "0");
        let f = network(&s);
        assert!(f[0].text.contains("Wi-Fi is switched off by airplane mode"), "{f:?}");
        assert!(f.iter().any(|x| x.text.contains("cable on eth0 is unplugged")));
        // No physical adapter at all.
        let r2 = tree();
        std::fs::create_dir_all(r2.join("sys/class/net")).unwrap();
        assert!(network(&src(&r2))[0].text.contains("no network adapter"));
    }

    #[test]
    fn nameservers_and_routes_are_read() {
        let r = tree();
        put(&r.join("resolv.conf"), "# hi\nnameserver 10.0.0.1\nnameserver  fe80::1%wlan0\nsearch lan\n");
        assert_eq!(nameservers(&r.join("resolv.conf")), vec!["10.0.0.1", "fe80::1%wlan0"]);
        put(&r.join("proc/net/route"), "Iface\tDestination\tGateway\tFlags\nwlan0\t0000A8C0\t00000000\t0001\nwlan0\t00000000\t0100A8C0\t0003\n");
        assert_eq!(default_route(&r.join("proc")).as_deref(), Some("192.168.0.1 via wlan0"));
    }

    #[test]
    fn dns_probe_retries_and_reads_the_answer() {
        // A tiny DNS server on loopback that drops the first question (as a
        // waking proxy does) and answers the second with the same id.
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let mut b = [0u8; 512];
            let _ = server.recv_from(&mut b).unwrap(); // dropped
            let (n, from) = server.recv_from(&mut b).unwrap();
            assert_eq!(&b[12..25], b"\x07example\x03org\x00", "asks for example.org");
            b[2] |= 0x80;
            b[3] = 0x03; // NXDOMAIN: the server works, says no
            server.send_to(&b[..n], from).unwrap();
        });
        let (_, rcode) = dns_probe_at(addr, Duration::from_millis(300), 3).expect("answered on the retry");
        assert_eq!(rcode, 3);
        t.join().unwrap();
        // Nobody listening: an honest "no answer" after the tries.
        let quiet = UdpSocket::bind("127.0.0.1:0").unwrap();
        let start = Instant::now();
        assert!(dns_probe_at(quiet.local_addr().unwrap(), Duration::from_millis(100), 2).is_err());
        assert!(start.elapsed() >= Duration::from_millis(200));
        assert!(dns_probe("not-an-ip", Duration::from_millis(10), 1).is_err());
    }

    #[test]
    fn stopped_names_paused_programs_and_waiting_jobs() {
        let r = tree();
        let p = r.join("proc");
        stat(&p, 30, "foot", 'T', 0, 10);
        let mut c = coord(&p);
        // The test's processes belong to whoever runs it.
        c.uid = unsafe { libc::getuid() };
        let f = stopped(&c, &src(&r), 100);
        assert!(f.iter().any(|x| x.text == "foot is paused (stopped), not closed." && x.certainty == "proven"), "{f:?}");
        let r2 = tree();
        std::fs::create_dir_all(r2.join("proc")).unwrap();
        let c2 = coord(&r2.join("proc"));
        assert!(stopped(&c2, &src(&r2), 100)[0].text.starts_with("Nothing of yours"));
    }

    #[test]
    fn slow_is_honest_when_nothing_is_short() {
        let r = tree();
        std::fs::create_dir_all(r.join("proc")).unwrap();
        let mut c = coord(&r.join("proc"));
        c.monitor.push(Sample { at: 1, mem_total_mb: 4000, mem_avail_mb: 3000, disk_total_mb: 100_000, disk_free_mb: 50_000, ..Default::default() });
        let f = slow(&c, &src(&r));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].certainty, "possible");
        // Memory pressure shows up as proven.
        c.monitor.push(Sample { at: 2, psi_mem: 30.0, mem_total_mb: 4000, mem_avail_mb: 150, disk_total_mb: 100_000, disk_free_mb: 50_000, ..Default::default() });
        let f = slow(&c, &src(&r));
        assert!(f.iter().any(|x| x.text.starts_with("Programs are waiting for memory") && x.certainty == "proven"), "{f:?}");
    }
}
