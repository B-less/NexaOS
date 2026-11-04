//! nexa-doctor: Nexa Doctor keeps the desktop running and fixes common
//! problems by itself.
//!
//!   nexa-doctor watch          in the session: the watchdog and health checks
//!   nexa-doctor check          what's wrong right now (JSON)
//!   nexa-doctor fix ID         fix one finding (volume, brightness, wifi, restart:PART)
//!   nexa-doctor log            what it did lately
//!   nexa-doctor security       the security chip (TPM 1.2 / 2.0) and Secure Boot (JSON)
//!   nexa-doctor firmware-keys [--text]  the PC's Secure Boot keys, read-only: cleared
//!                              keys, or a missing Windows key, and how to fix them
//!   nexa-doctor hardware [--text]  the computer's parts and which have no driver
//!                              (the text is also saved to ~/nexa-hardware-report.txt)
//!
//! Watchdog: the desktop's own parts (the bar, notifications, Orbit, the
//! "For you" card, Nexa Patterns, the focus watcher, idle/screen-off,
//! battery warnings, Hyprland's wallpaper) are restarted if they stop - at
//! most 3 times in 5 minutes each, then it says so instead of looping. Only
//! parts that were running are watched, so nothing is started that the
//! session didn't start.
//!
//! Health checks (every minute): volume above 100% is brought back to 100%;
//! a backlight at 0 (a black screen) is set to 30%; a Wi-Fi card that has
//! no network interface is repaired once (system broker: radio on, driver
//! re-attached); a disk almost full and memory running out are reported.
//! Once per boot it checks the PC's Secure Boot keys (secureboot.rs) and
//! warns, with the BIOS steps, if they were cleared or Windows' key is
//! gone: then Windows won't start. It never writes firmware keys.
//!
//! It runs as you with no extra rights and is deliberately not confined: the
//! parts it restarts must not inherit a sandbox. Anything needing root goes
//! through nexa-brokerd, which checks and logs it.

mod hardware;
mod secureboot;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    /// All of these must appear in the process's command line.
    pub matches: Vec<String>,
    pub start: Vec<String>,
    /// "sway", "hyprland" or "" for both.
    #[serde(default)]
    pub session: String,
}

fn part(name: &str, matches: &[&str], start: &[&str], session: &str) -> Part {
    Part { name: name.into(), matches: matches.iter().map(|s| s.to_string()).collect(),
           start: start.iter().map(|s| s.to_string()).collect(), session: session.into() }
}

/// The desktop's own parts (NEXA_DOCTOR_PARTS, a JSON list, replaces them in tests).
fn parts() -> Vec<Part> {
    if let Some(json) = std::env::var_os("NEXA_DOCTOR_PARTS") {
        return serde_json::from_str(&json.to_string_lossy()).unwrap_or_default();
    }
    vec![
        // nexa-bar picks the bar's config (always shown, or hidden until
        // the pointer touches the top edge) for either session.
        part("the top bar", &["waybar"], &["nexa-bar"], ""),
        part("notifications", &["mako"], &["mako"], ""),
        // The For you card, drives and Nexa Link's pop-ups: one process.
        part("the desktop parts", &["nexa_shell", "parts"], &["nexa-parts"], ""),
        part("Orbit", &["nexa-orbit"], &["nexa-orbit"], ""),
        part("Nexa Link", &["nexa-linkd", "serve"], &["nexa-linkd", "serve"], ""),
        part("Nexa MicroBrain", &["nexa-microbrain", "serve"], &["nexa-microbrain", "serve"], ""),
        part("Nexa Patterns", &["nexa-patternd", "serve"], &["nexa-patternd", "serve"], ""),
        part("the app-in-front boost", &["nexa-kerneld", "focus-watch"], &["nexa-kerneld", "focus-watch"], ""),
        part("idle and screen-off", &["nexa-idle"], &["nexa-idle"], ""),
        part("battery warnings", &["nexa-battery-watch"], &["nexa-battery-watch"], ""),
        part("the wallpaper", &["swaybg"], &["swaybg", "-i", "/usr/share/backgrounds/nexaos/wallpaper.png", "-m", "fill"], "hyprland"),
    ]
}

fn session() -> &'static str {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() { "hyprland" } else { "sway" }
}

fn proc_root() -> PathBuf {
    std::env::var_os("NEXA_DOCTOR_PROC").map(PathBuf::from).unwrap_or_else(|| "/proc".into())
}

/// Command lines of this user's processes.
fn my_processes(proc_root: &Path) -> Vec<String> {
    // SAFETY: getuid can't fail.
    let me = unsafe { libc::getuid() };
    fs::read_dir(proc_root).into_iter().flatten().flatten()
        .filter(|e| e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()))
        .filter(|e| fs::metadata(e.path()).map(|m| m.uid() == me).unwrap_or(false))
        .filter_map(|e| fs::read(e.path().join("cmdline")).ok())
        .map(|b| b.split(|c| *c == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect::<Vec<_>>().join(" "))
        .filter(|c| !c.is_empty())
        .collect()
}

pub fn running(cmdlines: &[String], p: &Part) -> bool {
    cmdlines.iter().any(|c| !c.contains("nexa-doctor") && p.matches.iter().all(|m| c.contains(m.as_str())))
}

/// Restarts allowed now, given the times of earlier ones.
pub fn may_restart(history: &mut Vec<Instant>, now: Instant) -> bool {
    history.retain(|t| now.duration_since(*t) < RESTART_WINDOW);
    history.len() < RESTARTS
}

fn state_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    base.join("nexa-doctor")
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn log(what: &str) {
    let dir = state_dir();
    let _ = fs::create_dir_all(&dir);
    let line = json!({"time": now_secs(), "what": what}).to_string();
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(dir.join("log.jsonl")) {
        let _ = writeln!(f, "{line}");
    }
    // Keep it short: the last 500 lines.
    if let Ok(text) = fs::read_to_string(dir.join("log.jsonl")) {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > 600 {
            let _ = fs::write(dir.join("log.jsonl"), lines[lines.len() - 500..].join("\n") + "\n");
        }
    }
}

fn notify(title: &str, body: &str) {
    let _ = Command::new("notify-send").args(["-a", "Nexa Doctor", "-i", "nexa-settings", title, body])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn spawn(argv: &[String]) -> bool {
    let Some((prog, args)) = argv.split_first() else { return false };
    let mut c = Command::new(prog);
    c.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // SAFETY: setsid in the child before exec: the part outlives a doctor restart.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    c.spawn().is_ok()
}

fn run(argv: &[&str]) -> Option<String> {
    let out = Command::new(argv[0]).args(&argv[1..]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Ask the system broker (one JSON line each way).
fn broker(action: &str, args: Value) -> Result<Value, String> {
    let path = std::env::var_os("NEXA_BROKER_SOCKET").map(PathBuf::from).unwrap_or_else(|| "/run/nexa-broker.sock".into());
    let mut s = UnixStream::connect(&path).map_err(|e| format!("the system broker isn't running: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(40)));
    writeln!(s, "{}", json!({"action": action, "args": args})).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if v["ok"] == true { Ok(v["result"].clone()) } else { Err(v["error"].as_str().unwrap_or("refused").to_string()) }
}

// ---------- health checks ----------

#[derive(Debug, Serialize, PartialEq)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub detail: String,
    /// Can `nexa-doctor fix ID` fix it?
    pub fixable: bool,
}

pub fn volume_of(wpctl: &str) -> Option<f64> {
    wpctl.split_whitespace().nth(1)?.parse().ok()
}

fn sys_root() -> PathBuf {
    std::env::var_os("NEXA_DOCTOR_SYS").map(PathBuf::from).unwrap_or_else(|| "/sys".into())
}

fn read_u64(p: &Path) -> Option<u64> {
    fs::read_to_string(p).ok()?.trim().parse().ok()
}

/// Backlights at 0 (a black screen): (name, max).
fn dark_backlights(sys: &Path) -> Vec<(String, u64)> {
    fs::read_dir(sys.join("class/backlight")).into_iter().flatten().flatten().filter_map(|e| {
        let max = read_u64(&e.path().join("max_brightness"))?;
        (read_u64(&e.path().join("brightness"))? == 0 && max > 0).then(|| (e.file_name().to_string_lossy().into_owned(), max))
    }).collect()
}

/// Network controllers (PCI class 0x0280) that have no network interface.
fn broken_wifi(sys: &Path) -> Vec<String> {
    fs::read_dir(sys.join("bus/pci/devices")).into_iter().flatten().flatten().filter(|e| {
        let class = fs::read_to_string(e.path().join("class")).unwrap_or_default();
        class.trim().starts_with("0x0280") && !e.path().join("net").exists() && !e.path().join("ieee80211").exists()
    }).map(|e| e.file_name().to_string_lossy().into_owned()).collect()
}

/// Is there a Wi-Fi card at all (a PCI network controller or any radio)?
fn has_wifi(sys: &Path) -> bool {
    fs::read_dir(sys.join("class/ieee80211")).map_or(false, |mut r| r.next().is_some())
        || fs::read_dir(sys.join("bus/pci/devices")).into_iter().flatten().flatten()
            .any(|e| fs::read_to_string(e.path().join("class")).map_or(false, |c| c.trim().starts_with("0x0280")))
}

/// Is iwd (which runs Wi-Fi) running? Any user can see process names.
fn iwd_running(proc_root: &Path) -> bool {
    fs::read_dir(proc_root).into_iter().flatten().flatten()
        .any(|e| fs::read_to_string(e.path().join("comm")).map_or(false, |c| c.trim() == "iwd"))
}

fn wifi_hard_blocked(sys: &Path) -> bool {
    fs::read_dir(sys.join("class/rfkill")).into_iter().flatten().flatten().any(|e| {
        fs::read_to_string(e.path().join("type")).map(|t| t.trim() == "wlan").unwrap_or(false)
            && fs::read_to_string(e.path().join("hard")).map(|h| h.trim() == "1").unwrap_or(false)
    })
}

/// A TPM maker from the 4 ASCII letters TPM 1.2 reports ("Manufacturer: 0x49465800").
fn tpm_maker(code: &str) -> Option<String> {
    let n = u32::from_str_radix(code.trim().trim_start_matches("0x"), 16).ok()?;
    let id: String = n.to_be_bytes().iter().filter(|b| b.is_ascii_alphanumeric()).map(|&b| b as char).collect();
    let name = match id.as_str() {
        "IFX" => "Infineon", "STM" => "STMicroelectronics", "NTC" => "Nuvoton", "ATML" => "Atmel",
        "INTC" => "Intel", "BRCM" => "Broadcom", "AMD" => "AMD", "NSM" => "National Semiconductor",
        "" => return None,
        other => other,
    };
    Some(name.to_string())
}

/// The security chip (TPM) and Secure Boot, from sysfs. Both TPM 1.2 and
/// 2.0 use the same kernel drivers: tpm_tis for a separate chip, tpm_crb for
/// one built into the processor's firmware (Intel PTT, AMD fTPM).
pub fn security(sys: &Path) -> Value {
    let dev = sys.join("class/tpm/tpm0");
    let tpm = if dev.exists() {
        let caps = fs::read_to_string(dev.join("caps")).or_else(|_| fs::read_to_string(dev.join("device/caps"))).unwrap_or_default();
        let field = |name: &str| caps.lines().find_map(|l| l.strip_prefix(name)).map(|v| v.trim().to_string());
        let version = match fs::read_to_string(dev.join("tpm_version_major")).map(|v| v.trim().to_string()).as_deref() {
            Ok("2") => "2.0".to_string(),
            Ok("1") => field("TCG version:").unwrap_or_else(|| "1.2".into()),
            _ => field("TCG version:").unwrap_or_else(|| "2.0".into()),
        };
        let driver = fs::read_link(dev.join("device/driver")).ok()
            .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        let kind = if driver == "tpm_crb" { "built into the processor" } else { "separate chip" };
        let maker = field("Manufacturer:").and_then(|m| tpm_maker(&m));
        let firmware = field("Firmware version:");
        let mut summary = format!("TPM {version} ({kind}");
        if let Some(m) = &maker {
            summary += &format!(", {m}");
        }
        summary.push(')');
        json!({"present": true, "version": version, "kind": kind, "driver": driver, "maker": maker,
               "firmware": firmware, "summary": summary})
    } else {
        // Firmware lists a TPM (ACPI MSFT0101 = 2.0, PNP0C31 = 1.2) but no driver took it.
        let listed = fs::read_dir(sys.join("bus/acpi/devices")).into_iter().flatten().flatten()
            .any(|e| { let n = e.file_name().to_string_lossy().into_owned(); n.starts_with("MSFT0101") || n.starts_with("PNP0C31") });
        let summary = if listed {
            "Found but not working (try switching it off and on in the BIOS)"
        } else {
            "None found (it may be switched off in the BIOS: Security > TPM)"
        };
        json!({"present": false, "listed_by_firmware": listed, "summary": summary})
    };
    let keys = secureboot::read(sys);
    let secure_boot = if !keys.uefi {
        "Not available (started in legacy BIOS mode)"
    } else if keys.setup_mode == Some(true) {
        "Off (the PC's keys were cleared)"
    } else if keys.secure_boot == Some(true) {
        "On"
    } else {
        "Off"
    };
    json!({"tpm": tpm, "secure_boot": secure_boot, "keys": keys})
}

/// Free space on the home folder's filesystem, as a fraction.
fn home_free() -> Option<f64> {
    let home = std::ffi::CString::new(std::env::var("HOME").ok()?).ok()?;
    // SAFETY: statvfs fills a plain struct.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(home.as_ptr(), &mut st) } != 0 || st.f_blocks == 0 {
        return None;
    }
    Some(st.f_bavail as f64 / st.f_blocks as f64)
}

fn memory_pressure(proc_root: &Path) -> Option<f64> {
    let text = fs::read_to_string(proc_root.join("pressure/memory")).ok()?;
    text.lines().next()?.split_whitespace().find_map(|w| w.strip_prefix("avg60="))?.parse().ok()
}

pub fn check() -> Vec<Finding> {
    let mut out = Vec::new();
    let sys = sys_root();
    if let Some(v) = run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"]).as_deref().and_then(volume_of) {
        if v > 1.0001 {
            out.push(Finding { id: "volume".into(), title: format!("Volume is above 100% ({:.0}%)", v * 100.0),
                               detail: "Above 100% sound distorts and can damage speakers.".into(), fixable: true });
        }
    }
    for (name, _) in dark_backlights(&sys) {
        out.push(Finding { id: "brightness".into(), title: "The screen brightness is at 0".into(),
                           detail: format!("The display ({name}) is turned all the way down."), fixable: true });
    }
    let wifi = broken_wifi(&sys);
    if wifi_hard_blocked(&sys) {
        out.push(Finding { id: "wifi-key".into(), title: "Wi-Fi is switched off".into(),
                           detail: "The keyboard's Wi-Fi key (or a switch) has turned the radio off - press it once.".into(), fixable: false });
    } else if !wifi.is_empty() {
        out.push(Finding { id: "wifi".into(), title: "The Wi-Fi adapter isn't working".into(),
                           detail: format!("The card ({}) has no network interface.", wifi.join(", ")), fixable: true });
    }
    if has_wifi(&sys) && !iwd_running(&proc_root()) {
        out.push(Finding { id: "wifi-service".into(), title: "Wi-Fi's service isn't running".into(),
                           detail: "iwd, which runs Wi-Fi, has stopped, so no network can be found or joined.".into(), fixable: true });
    }
    if let Some(free) = home_free() {
        if free < 0.05 {
            out.push(Finding { id: "disk".into(), title: format!("The disk is almost full ({:.0}% free)", free * 100.0),
                               detail: "Empty the Trash or remove large files (the assistant can find them).".into(), fixable: false });
        }
    }
    if let Some(p) = memory_pressure(&proc_root()) {
        if p > 20.0 {
            out.push(Finding { id: "memory".into(), title: "The computer is short of memory".into(),
                               detail: format!("Programs waited for memory {p:.0}% of the last minute. Close apps you aren't using."), fixable: false });
        }
    }
    for p in secureboot::read(&sys).problems {
        out.push(Finding { id: p.id, title: p.title, detail: p.detail, fixable: false });
    }
    let cmdlines = my_processes(&proc_root());
    let missing = missing_parts(&cmdlines);
    for p in missing {
        out.push(Finding { id: format!("restart:{}", p.matches.join(" ")), title: format!("{} isn't running", capital(&p.name)),
                           detail: "Part of the desktop has stopped.".into(), fixable: true });
    }
    out
}

/// Parts of this session that should be running and aren't (only when the
/// session itself is up, i.e. the bar or Dock was ever started).
fn missing_parts(cmdlines: &[String]) -> Vec<Part> {
    let s = session();
    let ps: Vec<Part> = parts().into_iter().filter(|p| p.session.is_empty() || p.session == s).collect();
    if !ps.iter().any(|p| running(cmdlines, p)) {
        return vec![]; // no desktop session here at all
    }
    ps.into_iter().filter(|p| !running(cmdlines, p)).collect()
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

pub fn fix(id: &str) -> Result<String, String> {
    match id {
        "volume" => run(&["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "1.0"]).map(|_| "Volume set back to 100%".to_string())
            .ok_or_else(|| "couldn't change the volume".into()),
        "brightness" => broker("set_brightness", json!({"percent": 30})).map(|_| "Screen brightness set to 30%".to_string()),
        "wifi" => broker("repair_wifi", json!({})).map(|v| v["done"].as_str().unwrap_or("Wi-Fi repaired").to_string()),
        "wifi-service" => broker("restart_service", json!({"name": "iwd"})).map(|_| "Wi-Fi's service started again".to_string()),
        other => {
            let key = other.strip_prefix("restart:").ok_or_else(|| format!("unknown problem '{other}'"))?;
            let p = parts().into_iter().find(|p| p.matches.join(" ") == key && (p.session.is_empty() || p.session == session()))
                .ok_or_else(|| format!("unknown part '{key}'"))?;
            if spawn(&p.start) { Ok(format!("Restarted {}", p.name)) } else { Err(format!("couldn't start {}", p.name)) }
        }
    }
}

// ---------- the watchdog ----------

fn watch() -> ! {
    let interval = Duration::from_millis(std::env::var("NEXA_DOCTOR_INTERVAL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(3000));
    let health_every = Duration::from_secs(std::env::var("NEXA_DOCTOR_HEALTH_S").ok().and_then(|v| v.parse().ok()).unwrap_or(60));
    let started = Instant::now();
    let mut seen: HashMap<String, bool> = HashMap::new();
    let mut restarts: HashMap<String, Vec<Instant>> = HashMap::new();
    let mut gave_up: HashMap<String, bool> = HashMap::new();
    let mut last_health = Instant::now() - health_every;
    let mut iwd_restarted: Option<Instant> = None;
    let mut told: HashMap<&'static str, Instant> = HashMap::new();
    let mut wifi_repaired = false;
    log("watching");
    firmware_keys_once();
    loop {
        let cmdlines = my_processes(&proc_root());
        for p in parts().into_iter().filter(|p| p.session.is_empty() || p.session == session()) {
            let key = format!("{}|{}", p.name, p.matches.join(" "));
            if running(&cmdlines, &p) {
                seen.insert(key.clone(), true);
                continue;
            }
            if !seen.get(&key).copied().unwrap_or(false) || gave_up.get(&key).copied().unwrap_or(false) {
                continue; // never ran in this session, or it keeps crashing
            }
            let history = restarts.entry(key.clone()).or_default();
            if may_restart(history, Instant::now()) {
                history.push(Instant::now());
                let ok = spawn(&p.start);
                log(&format!("{} stopped; restarted it ({})", p.name, if ok { "ok" } else { "failed" }));
            } else {
                gave_up.insert(key.clone(), true);
                log(&format!("{} keeps stopping; gave up after {RESTARTS} restarts", p.name));
                notify(&format!("{} keeps stopping", capital(&p.name)), "Nexa Doctor restarted it several times. Restarting the computer may help.");
            }
        }
        if last_health.elapsed() >= health_every {
            last_health = Instant::now();
            let sys = sys_root();
            // Fix what can safely be fixed without asking.
            if let Some(v) = run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"]).as_deref().and_then(volume_of) {
                if v > 1.0001 && fix("volume").is_ok() {
                    log(&format!("volume was {:.0}%; set back to 100%", v * 100.0));
                }
            }
            if !dark_backlights(&sys).is_empty() && fix("brightness").is_ok() {
                log("the screen was at brightness 0; set to 30%");
            }
            // Wi-Fi's service (iwd) stopped: start it again, at most once
            // every 2 minutes (it once quit at every boot; see conf.d/dbus).
            if has_wifi(&sys) && !iwd_running(&proc_root())
                && iwd_restarted.map_or(true, |t: Instant| t.elapsed() > Duration::from_secs(120)) {
                iwd_restarted = Some(Instant::now());
                match fix("wifi-service") {
                    Ok(done) => log(&format!("iwd wasn't running: {done}")),
                    Err(e) => log(&format!("iwd wasn't running and couldn't be started: {e}")),
                }
            }
            // Wi-Fi: one automatic repair per session, once the boot has settled.
            if !wifi_repaired && started.elapsed() > Duration::from_secs(45) && !broken_wifi(&sys).is_empty() && !wifi_hard_blocked(&sys) {
                wifi_repaired = true;
                match fix("wifi") {
                    Ok(done) => {
                        log(&format!("Wi-Fi adapter had no interface: {done}"));
                        std::thread::sleep(Duration::from_secs(8));
                        if broken_wifi(&sys).is_empty() {
                            notify("Wi-Fi repaired", "The Wi-Fi adapter wasn't working; Nexa Doctor restarted it.");
                        } else {
                            notify("The Wi-Fi adapter isn't working", "Nexa Doctor tried to restart it. Restarting the computer may help.");
                        }
                    }
                    Err(e) => log(&format!("couldn't repair Wi-Fi: {e}")),
                }
            }
            let mut tell = |key: &'static str, every: Duration, title: &str, body: &str| {
                if told.get(key).map_or(true, |t| t.elapsed() > every) {
                    told.insert(key, Instant::now());
                    log(title);
                    notify(title, body);
                }
            };
            if home_free().map_or(false, |f| f < 0.05) {
                tell("disk", Duration::from_secs(6 * 3600), "The disk is almost full", "Empty the Trash or remove large files you don't need.");
            }
            if memory_pressure(&proc_root()).map_or(false, |p| p > 20.0) {
                tell("memory", Duration::from_secs(1800), "The computer is short of memory", "Close apps you aren't using to speed things up.");
            }
        }
        std::thread::sleep(interval);
    }
}

/// Once per boot (the boot id is kept in the state folder): warn about
/// Secure Boot keys that stop Windows starting. A critical notification
/// stays until it is closed.
fn firmware_keys_once() {
    let problems = secureboot::read(&sys_root()).problems;
    let Some(p) = problems.first() else { return };
    let boot_id = fs::read_to_string(proc_root().join("sys/kernel/random/boot_id")).unwrap_or_default();
    let mark = state_dir().join("firmware-keys-told");
    if !boot_id.is_empty() && fs::read_to_string(&mark).map_or(false, |m| m == boot_id) {
        return;
    }
    let _ = fs::create_dir_all(state_dir());
    let _ = fs::write(&mark, &boot_id);
    log(&p.title);
    let _ = Command::new("notify-send").args(["-a", "Nexa Doctor", "-u", "critical", "-i", "dialog-warning", &p.title,
            &format!("{}\n\nDetails: nexa-doctor firmware-keys --text", p.detail)])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reply = match args.first().map(String::as_str) {
        Some("watch") => watch(),
        Some("check") => Ok(json!({"findings": check()})),
        Some("fix") => match args.get(1) {
            Some(id) => fix(id).map(|done| json!({"done": done})),
            None => Err("usage: nexa-doctor fix ID".into()),
        },
        Some("log") => Ok(json!({"log": fs::read_to_string(state_dir().join("log.jsonl")).unwrap_or_default().lines()
            .rev().take(50).filter_map(|l| serde_json::from_str::<Value>(l).ok()).collect::<Vec<_>>()})),
        Some("security") => Ok(security(&sys_root())),
        Some("firmware-keys") => {
            let k = secureboot::read(&sys_root());
            if args.iter().any(|a| a == "--text") {
                print!("{}", secureboot::text(&k));
                return;
            }
            serde_json::to_value(k).map_err(|e| e.to_string())
        }
        Some("hardware") => {
            let kmsg = std::env::var_os("NEXA_DOCTOR_KMSG").map(PathBuf::from).unwrap_or_else(|| "/dev/kmsg".into());
            let mut r = hardware::report(&sys_root(), &proc_root(), &kmsg);
            // The kernel log needs root, which people don't have: the system
            // broker hands out just its hardware lines (Wi-Fi, firmware, radios).
            if r["firmware_errors"].is_null() {
                if let Ok(v) = broker("hardware_log", json!({})) {
                    r["firmware_errors"] = v["lines"].clone();
                }
            }
            let text = hardware::text(&r);
            let saved = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("nexa-hardware-report.txt"))
                .filter(|p| fs::write(p, &text).is_ok());
            if args.iter().any(|a| a == "--text") {
                print!("{text}");
                if let Some(p) = saved {
                    println!("\n(saved to {})", p.display());
                }
                return;
            }
            Ok(r)
        }
        _ => Err("usage: nexa-doctor watch | check | fix ID | log | security | firmware-keys [--text] | hardware [--text]".into()),
    };
    match reply {
        Ok(v) => println!("{}", json!({"ok": true, "result": v})),
        Err(e) => {
            println!("{}", json!({"ok": false, "error": e}));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_and_backoff() {
        let p = part("the desktop parts", &["nexa_shell", "parts"], &["nexa-parts"], "");
        assert!(running(&["python3 -m nexa_shell parts".into()], &p));
        assert!(!running(&["python3 -m nexa_shell nexus".into()], &p));
        assert!(!running(&["nexa-doctor fix restart:nexa_shell parts".into()], &p), "never itself");
        let o = part("Orbit", &["nexa-orbit"], &["nexa-orbit"], "");
        assert!(running(&["/usr/local/bin/nexa-orbit".into()], &o));
        assert!(!running(&["nexa-living".into()], &o));
        let mut h = Vec::new();
        let t = Instant::now();
        for _ in 0..3 {
            assert!(may_restart(&mut h, t));
            h.push(t);
        }
        assert!(!may_restart(&mut h, t), "4th restart in 5 minutes: no");
        assert!(may_restart(&mut h, t + RESTART_WINDOW + Duration::from_secs(1)), "later: yes again");
        assert_eq!(volume_of("Volume: 1.35"), Some(1.35));
        assert_eq!(volume_of("Volume: 0.40 [MUTED]"), Some(0.40));
    }

    #[test]
    fn hardware_checks_on_a_fake_sys() {
        let r = std::env::temp_dir().join(format!("nexa-doctor-sys-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let bl = r.join("class/backlight/intel_backlight");
        fs::create_dir_all(&bl).unwrap();
        fs::write(bl.join("brightness"), "0\n").unwrap();
        fs::write(bl.join("max_brightness"), "19393\n").unwrap();
        assert_eq!(dark_backlights(&r), vec![("intel_backlight".to_string(), 19393)]);
        fs::write(bl.join("brightness"), "5000\n").unwrap();
        assert!(dark_backlights(&r).is_empty());
        let card = r.join("bus/pci/devices/0000:00:14.3");
        fs::create_dir_all(&card).unwrap();
        fs::write(card.join("class"), "0x028000\n").unwrap();
        assert_eq!(broken_wifi(&r), vec!["0000:00:14.3"]);
        fs::create_dir_all(card.join("net")).unwrap();
        assert!(broken_wifi(&r).is_empty());
        let rf = r.join("class/rfkill/rfkill0");
        fs::create_dir_all(&rf).unwrap();
        fs::write(rf.join("type"), "wlan\n").unwrap();
        fs::write(rf.join("hard"), "1\n").unwrap();
        assert!(wifi_hard_blocked(&r));
        assert!(has_wifi(&r), "a 0x0280 card");
        let p = r.join("proc");
        fs::create_dir_all(p.join("7")).unwrap();
        fs::write(p.join("7/comm"), "sway\n").unwrap();
        assert!(!iwd_running(&p));
        fs::create_dir_all(p.join("9")).unwrap();
        fs::write(p.join("9/comm"), "iwd\n").unwrap();
        assert!(iwd_running(&p));
    }

    #[test]
    fn security_chip_on_a_fake_sys() {
        let r = std::env::temp_dir().join(format!("nexa-doctor-tpm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        fs::create_dir_all(&r).unwrap();
        let v = security(&r);
        assert_eq!(v["tpm"]["present"], false);
        assert!(v["tpm"]["summary"].as_str().unwrap().contains("BIOS"));
        assert!(v["secure_boot"].as_str().unwrap().contains("legacy"));
        // TPM 1.2, a separate Infineon chip; Secure Boot on.
        let tpm = r.join("class/tpm/tpm0");
        fs::create_dir_all(tpm.join("device")).unwrap();
        fs::write(tpm.join("tpm_version_major"), "1\n").unwrap();
        fs::write(tpm.join("device/caps"), "Manufacturer: 0x49465800\nTCG version: 1.2\nFirmware version: 6.43\n").unwrap();
        fs::create_dir_all(r.join("drivers/tpm_tis")).unwrap();
        std::os::unix::fs::symlink(r.join("drivers/tpm_tis"), tpm.join("device/driver")).unwrap();
        fs::create_dir_all(r.join("firmware/efi/efivars")).unwrap();
        fs::write(r.join("firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"), [6, 0, 0, 0, 1]).unwrap();
        let v = security(&r);
        assert_eq!(v["tpm"]["summary"], "TPM 1.2 (separate chip, Infineon)", "{v}");
        assert_eq!(v["tpm"]["firmware"], "6.43");
        assert_eq!(v["secure_boot"], "Off (the PC's keys were cleared)", "no PK and no SetupMode: cleared");
        fs::write(r.join("firmware/efi/efivars/SetupMode-8be4df61-93ca-11d2-aa0d-00e098032b8c"), [6, 0, 0, 0, 0]).unwrap();
        assert_eq!(security(&r)["secure_boot"], "On");
        // TPM 2.0 in the processor's firmware (Intel PTT): tpm_crb, no caps.
        fs::write(tpm.join("tpm_version_major"), "2\n").unwrap();
        fs::remove_file(tpm.join("device/caps")).unwrap();
        fs::remove_file(tpm.join("device/driver")).unwrap();
        fs::create_dir_all(r.join("drivers/tpm_crb")).unwrap();
        std::os::unix::fs::symlink(r.join("drivers/tpm_crb"), tpm.join("device/driver")).unwrap();
        assert_eq!(security(&r)["tpm"]["summary"], "TPM 2.0 (built into the processor)");
        // Listed by the firmware, but no driver attached.
        fs::remove_dir_all(r.join("class/tpm")).unwrap();
        fs::create_dir_all(r.join("bus/acpi/devices/MSFT0101:00")).unwrap();
        assert!(security(&r)["tpm"]["summary"].as_str().unwrap().starts_with("Found but not working"));
    }
}
