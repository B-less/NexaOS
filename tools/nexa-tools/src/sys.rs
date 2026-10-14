//! System facts and small exact helpers for the assistant, read straight
//! from the kernel: date and time, battery, the busiest processes; plus
//! stopping one of the user's own programs, an exact calculator and unit
//! conversion (small models are bad at arithmetic).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::path::Path;

pub const PROC: &str = "/proc";
pub const POWER: &str = "/sys/class/power_supply";

fn read(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

// ---- date and time

pub fn now() -> Value {
    // SAFETY: time/localtime_r/strftime with buffers we own.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let fmt = |f: &str| {
            let mut buf = [0u8; 128];
            let cf = std::ffi::CString::new(f).unwrap();
            let n = libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), cf.as_ptr(), &tm);
            String::from_utf8_lossy(&buf[..n]).into_owned()
        };
        let zone = if tm.tm_zone.is_null() { String::new() } else { CStr::from_ptr(tm.tm_zone).to_string_lossy().into_owned() };
        json!({
            "date": fmt("%A %-d %B %Y"), "time": fmt("%H:%M"), "timezone": zone,
            "iso": fmt("%Y-%m-%dT%H:%M:%S%z"), "week": fmt("%V"), "day_of_year": fmt("%-j"),
        })
    }
}

// ---- battery

pub fn battery() -> Value {
    let mut batteries = Vec::new();
    let mut on_ac = None;
    for e in fs::read_dir(POWER).into_iter().flatten().flatten() {
        let d = e.path();
        match read(d.join("type")).as_deref() {
            Some("Mains") => on_ac = Some(read(d.join("online")).as_deref() == Some("1")),
            Some("Battery") => {
                let n = |f: &str| read(d.join(f)).and_then(|s| s.parse::<f64>().ok());
                let pct = n("capacity");
                let status = read(d.join("status")).unwrap_or_default();
                // Energy (µWh / µW) or charge (µAh / µA) counters, whichever the driver has.
                let (now, full, rate) = match (n("energy_now"), n("energy_full"), n("power_now")) {
                    (Some(a), Some(b), r) => (a, b, r),
                    _ => (n("charge_now").unwrap_or(0.0), n("charge_full").unwrap_or(0.0), n("current_now")),
                };
                let hours = rate.filter(|r| *r > 0.0).map(|r| if status == "Charging" { (full - now) / r } else { now / r });
                let health = match (n("energy_full"), n("energy_full_design")) {
                    (Some(f), Some(d)) if d > 0.0 => Some((f / d * 100.0).round()),
                    _ => match (n("charge_full"), n("charge_full_design")) {
                        (Some(f), Some(d)) if d > 0.0 => Some((f / d * 100.0).round()),
                        _ => None,
                    },
                };
                let mut b = json!({"name": e.file_name().to_string_lossy(), "percent": pct, "status": status});
                if let Some(h) = hours.filter(|h| *h < 48.0) {
                    let key = if status == "Charging" { "time_to_full" } else { "time_left" };
                    b[key] = json!(format!("{}h {:02}m", h as u64, ((h.fract()) * 60.0) as u64));
                }
                if let Some(h) = health {
                    b["health_percent"] = json!(h);
                }
                batteries.push(b);
            }
            _ => {}
        }
    }
    let summary = match batteries.first() {
        None => "No battery (a desktop, or the battery isn't detected)".to_string(),
        Some(b) => format!("Battery {}%, {}{}", b["percent"], b["status"].as_str().unwrap_or("").to_lowercase(),
            b.get("time_left").or(b.get("time_to_full")).and_then(Value::as_str)
                .map(|t| format!(", {t} {}", if b.get("time_left").is_some() { "left" } else { "to full" })).unwrap_or_default()),
    };
    json!({"summary": summary, "on_ac_power": on_ac, "batteries": batteries})
}

// ---- processes

struct Proc {
    pid: i32,
    name: String,
    uid: u32,
    ticks: u64,
    rss_kb: u64,
}

fn processes() -> Vec<Proc> {
    let mut out = Vec::new();
    for e in fs::read_dir(PROC).into_iter().flatten().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
        let Some(stat) = read(e.path().join("stat")) else { continue };
        // comm is in parentheses and may contain spaces.
        let (Some(l), Some(r)) = (stat.find('('), stat.rfind(')')) else { continue };
        let name = stat[l + 1..r].to_string();
        let f: Vec<&str> = stat[r + 2..].split_whitespace().collect();
        let ticks = f.get(11).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
        let status = read(e.path().join("status")).unwrap_or_default();
        let field = |k: &str| status.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|x| x.parse::<u64>().ok());
        out.push(Proc { pid, name, uid: field("Uid:").unwrap_or(0) as u32, ticks, rss_kb: field("VmRSS:").unwrap_or(0) });
    }
    out
}

/// A friendlier name: the program from the command line (python apps show
/// as their module, e.g. nexa_settings).
fn display_name(pid: i32, comm: &str) -> String {
    let cmd = fs::read(format!("{PROC}/{pid}/cmdline")).unwrap_or_default();
    let args: Vec<String> = cmd.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
    if comm.starts_with("python") {
        if let Some(i) = args.iter().position(|a| a == "-m") {
            if let Some(m) = args.get(i + 1) {
                return m.clone();
            }
        }
        if let Some(script) = args.get(1) {
            return Path::new(script).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| comm.into());
        }
    }
    comm.to_string()
}

pub fn top_processes(args: &Value) -> Value {
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(8).clamp(1, 30) as usize;
    let before: HashMap<i32, u64> = processes().into_iter().map(|p| (p.pid, p.ticks)).collect();
    let t0 = read("/proc/stat").and_then(|s| s.lines().next().map(|l| l.split_whitespace().skip(1).filter_map(|x| x.parse::<u64>().ok()).sum::<u64>())).unwrap_or(0);
    std::thread::sleep(std::time::Duration::from_millis(700));
    let t1 = read("/proc/stat").and_then(|s| s.lines().next().map(|l| l.split_whitespace().skip(1).filter_map(|x| x.parse::<u64>().ok()).sum::<u64>())).unwrap_or(0);
    let cpus = read("/proc/stat").map(|s| s.lines().filter(|l| l.starts_with("cpu") && !l.starts_with("cpu ")).count()).unwrap_or(1).max(1) as f64;
    let total = (t1.saturating_sub(t0)).max(1) as f64;
    let mem_total_kb = read("/proc/meminfo").and_then(|m| m.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|x| x.parse::<f64>().ok())).unwrap_or(1.0);
    let me = unsafe { libc::getuid() };
    let mut rows: Vec<(f64, Proc)> = processes().into_iter().filter(|p| p.rss_kb > 0 || before.contains_key(&p.pid))
        .map(|p| {
            let d = p.ticks.saturating_sub(*before.get(&p.pid).unwrap_or(&p.ticks)) as f64;
            // Percent of one core, like top.
            (d / total * cpus * 100.0, p)
        }).collect();
    let row = |(cpu, p): &(f64, Proc)| json!({
        "pid": p.pid, "name": display_name(p.pid, &p.name), "cpu_percent": (cpu * 10.0).round() / 10.0,
        "memory": if p.rss_kb > 1024 * 1024 { format!("{:.1} GB", p.rss_kb as f64 / 1048576.0) } else { format!("{} MB", p.rss_kb / 1024) },
        "memory_percent": ((p.rss_kb as f64 / mem_total_kb) * 1000.0).round() / 10.0,
        "yours": p.uid == me,
    });
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let by_cpu: Vec<Value> = rows.iter().take(limit).map(row).collect();
    rows.sort_by(|a, b| b.1.rss_kb.cmp(&a.1.rss_kb));
    let by_memory: Vec<Value> = rows.iter().take(limit).map(row).collect();
    let summary = format!("Busiest now: {}; most memory: {}",
        by_cpu.first().map(|p| format!("{} ({}% CPU)", p["name"].as_str().unwrap_or(""), p["cpu_percent"])).unwrap_or_default(),
        by_memory.first().map(|p| format!("{} ({})", p["name"].as_str().unwrap_or(""), p["memory"].as_str().unwrap_or(""))).unwrap_or_default());
    json!({"summary": summary, "by_cpu": by_cpu, "by_memory": by_memory, "processes": rows.len()})
}

/// Never stopped by the assistant: the desktop itself.
const PROTECTED: &[&str] = &["sway", "Hyprland", "waybar", "mako", "swaybg", "swayidle", "nexa-session", "nexa-desktop",
    "dbus-daemon", "pipewire", "wireplumber", "seatd", "nexa_shell", "nexa-tools", "nexa_ai", "nexa-ai-gui", "sh", "bash", "login"];

pub fn stop_process(args: &Value) -> Result<Value, String> {
    let me = unsafe { libc::getuid() };
    let target: Vec<Proc> = if let Some(pid) = args.get("pid").and_then(Value::as_i64) {
        processes().into_iter().filter(|p| p.pid as i64 == pid).collect()
    } else {
        let name = args.get("name").and_then(Value::as_str).ok_or("give a pid or a name")?.to_lowercase();
        processes().into_iter().filter(|p| display_name(p.pid, &p.name).to_lowercase() == name || p.name.to_lowercase() == name).collect()
    };
    if target.is_empty() {
        return Err("no such program is running".into());
    }
    let mut stopped = Vec::new();
    for p in &target {
        let shown = display_name(p.pid, &p.name);
        if p.uid != me {
            return Err(format!("{shown} (pid {}) belongs to the system, not to you; it can't be stopped from here", p.pid));
        }
        if PROTECTED.contains(&p.name.as_str()) || PROTECTED.contains(&shown.as_str()) || p.pid == std::process::id() as i32 {
            return Err(format!("{shown} is part of the desktop; stopping it would break your session"));
        }
        // SAFETY: plain kill(2) on a pid we just read, owned by us.
        if unsafe { libc::kill(p.pid, libc::SIGTERM) } != 0 {
            return Err(format!("couldn't stop {shown}: {}", std::io::Error::last_os_error()));
        }
        stopped.push(format!("{shown} (pid {})", p.pid));
    }
    Ok(json!({"done": format!("Asked {} to close", stopped.join(", ")), "stopped": stopped}))
}

// ---- calculator

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.s.get(self.i) == Some(&c) { self.i += 1; true } else { false }
    }
    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') { v += self.term()?; } else if self.eat(b'-') { v -= self.term()?; } else { return Ok(v); }
        }
    }
    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat(b'*') || self.eat(b'x') {
                v *= self.unary()?;
            } else if self.eat(b'/') {
                let d = self.unary()?;
                if d == 0.0 { return Err("division by zero".into()); }
                v /= d;
            } else if self.eat(b'%') {
                // "15 % of 80" and "20%" are percentages; "a % b" between
                // numbers is the remainder.
                self.ws();
                if self.s[self.i..].starts_with(b"of") {
                    self.i += 2;
                    v = v / 100.0 * self.unary()?;
                } else if self.s.get(self.i).map_or(true, |c| !(c.is_ascii_digit() || *c == b'(' || *c == b'.')) {
                    v /= 100.0;
                } else {
                    let d = self.unary()?;
                    if d == 0.0 { return Err("division by zero".into()); }
                    v %= d;
                }
            } else {
                return Ok(v);
            }
        }
    }
    // -3^2 is -(3^2), and 2^-1 works: powers bind tighter than signs.
    fn unary(&mut self) -> Result<f64, String> {
        if self.eat(b'-') { return Ok(-self.unary()?); }
        if self.eat(b'+') { return self.unary(); }
        self.power()
    }
    fn power(&mut self) -> Result<f64, String> {
        let b = self.atom()?;
        if self.eat(b'^') { Ok(b.powf(self.unary()?)) } else { Ok(b) }
    }
    fn atom(&mut self) -> Result<f64, String> {
        self.ws();
        if self.eat(b'(') {
            let v = self.expr()?;
            if !self.eat(b')') { return Err("missing )".into()); }
            return Ok(v);
        }
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.' || self.s[self.i] == b',') {
            self.i += 1;
        }
        if self.i > start {
            let t: String = String::from_utf8_lossy(&self.s[start..self.i]).replace(',', "");
            let mut v: f64 = t.parse().map_err(|_| format!("bad number {t}"))?;
            if self.s.get(self.i).map_or(false, |c| *c == b'e' || *c == b'E') && self.s.get(self.i + 1).map_or(false, |c| c.is_ascii_digit() || *c == b'-') {
                self.i += 1;
                let neg = self.eat(b'-');
                let es = self.i;
                while self.i < self.s.len() && self.s[self.i].is_ascii_digit() { self.i += 1; }
                let e: i32 = String::from_utf8_lossy(&self.s[es..self.i]).parse().unwrap_or(0);
                v *= 10f64.powi(if neg { -e } else { e });
            }
            return Ok(v);
        }
        while self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
            self.i += 1;
        }
        let word = String::from_utf8_lossy(&self.s[start..self.i]).to_lowercase();
        match word.as_str() {
            "pi" => return Ok(std::f64::consts::PI),
            "e" => return Ok(std::f64::consts::E),
            "" => return Err(format!("unexpected '{}'", self.s.get(self.i).map(|c| *c as char).unwrap_or(' '))),
            _ => {}
        }
        if !self.eat(b'(') { return Err(format!("unknown word '{word}'")); }
        let a = self.expr()?;
        if !self.eat(b')') { return Err("missing )".into()); }
        Ok(match word.as_str() {
            "sqrt" => { if a < 0.0 { return Err("square root of a negative number".into()); } a.sqrt() }
            "abs" => a.abs(), "round" => a.round(), "floor" => a.floor(), "ceil" => a.ceil(),
            "ln" => a.ln(), "log" => a.log10(), "exp" => a.exp(),
            "sin" => a.to_radians().sin(), "cos" => a.to_radians().cos(), "tan" => a.to_radians().tan(),
            other => return Err(format!("unknown function '{other}'")),
        })
    }
}

pub fn evaluate(expr: &str) -> Result<f64, String> {
    let e = expr.replace('×', "*").replace('÷', "/").replace("**", "^");
    let mut p = Parser { s: e.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("couldn't understand '{}'", &e[p.i..]));
    }
    if !v.is_finite() { return Err("the result is too large".into()); }
    Ok(v)
}

fn tidy(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) }
    else { let s = format!("{:.10}", v); s.trim_end_matches('0').trim_end_matches('.').to_string() }
}

pub fn calculate(args: &Value) -> Result<Value, String> {
    let expr = args.get("expression").and_then(Value::as_str).ok_or("missing 'expression'")?;
    if expr.len() > 300 { return Err("that expression is too long".into()); }
    let v = evaluate(expr)?;
    Ok(json!({"expression": expr, "result": tidy(v)}))
}

// ---- unit conversion: (name, aliases, quantity, factor to base unit)

const UNITS: &[(&str, &[&str], &str, f64)] = &[
    ("m", &["m", "meter", "meters", "metre", "metres"], "length", 1.0),
    ("km", &["km", "kilometer", "kilometers", "kilometre", "kilometres"], "length", 1000.0),
    ("cm", &["cm", "centimeter", "centimeters"], "length", 0.01),
    ("mm", &["mm", "millimeter", "millimeters"], "length", 0.001),
    ("mi", &["mi", "mile", "miles"], "length", 1609.344),
    ("ft", &["ft", "foot", "feet"], "length", 0.3048),
    ("in", &["in", "inch", "inches"], "length", 0.0254),
    ("yd", &["yd", "yard", "yards"], "length", 0.9144),
    ("kg", &["kg", "kilogram", "kilograms", "kilo", "kilos"], "mass", 1.0),
    ("g", &["g", "gram", "grams"], "mass", 0.001),
    ("lb", &["lb", "lbs", "pound", "pounds"], "mass", 0.45359237),
    ("oz", &["oz", "ounce", "ounces"], "mass", 0.028349523125),
    ("l", &["l", "liter", "liters", "litre", "litres"], "volume", 1.0),
    ("ml", &["ml", "milliliter", "milliliters"], "volume", 0.001),
    ("gal", &["gal", "gallon", "gallons"], "volume", 3.785411784),
    ("cup", &["cup", "cups"], "volume", 0.2365882365),
    ("km/h", &["km/h", "kph", "kmh"], "speed", 1.0),
    ("mph", &["mph"], "speed", 1.609344),
    ("m/s", &["m/s"], "speed", 3.6),
    ("B", &["b", "byte", "bytes"], "data", 1.0),
    ("KB", &["kb", "kilobyte", "kilobytes"], "data", 1024.0),
    ("MB", &["mb", "megabyte", "megabytes"], "data", 1048576.0),
    ("GB", &["gb", "gigabyte", "gigabytes"], "data", 1073741824.0),
    ("TB", &["tb", "terabyte", "terabytes"], "data", 1099511627776.0),
    ("s", &["s", "sec", "second", "seconds"], "time", 1.0),
    ("min", &["min", "minute", "minutes"], "time", 60.0),
    ("h", &["h", "hr", "hour", "hours"], "time", 3600.0),
    ("day", &["day", "days"], "time", 86400.0),
];

fn unit(name: &str) -> Option<&'static (&'static str, &'static [&'static str], &'static str, f64)> {
    let n = name.trim().to_lowercase();
    UNITS.iter().find(|u| u.1.contains(&n.as_str()))
}

fn temperature(name: &str) -> Option<char> {
    match name.trim().to_lowercase().trim_start_matches('°') {
        "c" | "celsius" => Some('C'), "f" | "fahrenheit" => Some('F'), "k" | "kelvin" => Some('K'), _ => None,
    }
}

pub fn convert(args: &Value) -> Result<Value, String> {
    let value = args.get("value").and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| evaluate(s).ok()))).ok_or("missing 'value'")?;
    let from = args.get("from").and_then(Value::as_str).ok_or("missing 'from'")?;
    let to = args.get("to").and_then(Value::as_str).ok_or("missing 'to'")?;
    let result = if let (Some(a), Some(b)) = (temperature(from), temperature(to)) {
        let c = match a { 'C' => value, 'F' => (value - 32.0) * 5.0 / 9.0, _ => value - 273.15 };
        let r = match b { 'C' => c, 'F' => c * 9.0 / 5.0 + 32.0, _ => c + 273.15 };
        return Ok(json!({"result": format!("{} °{a} = {} °{b}", tidy(value), tidy((r * 100.0).round() / 100.0))}));
    } else {
        let (a, b) = (unit(from).ok_or_else(|| format!("unknown unit '{from}'"))?, unit(to).ok_or_else(|| format!("unknown unit '{to}'"))?);
        if a.2 != b.2 {
            return Err(format!("can't convert {} ({}) to {} ({})", a.0, a.2, b.0, b.2));
        }
        value * a.3 / b.3
    };
    let shown = if result.abs() >= 100.0 { (result * 100.0).round() / 100.0 } else { (result * 10000.0).round() / 10000.0 };
    Ok(json!({"result": format!("{} {} = {} {}", tidy(value), unit(from).unwrap().0, tidy(shown), unit(to).unwrap().0)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_is_exact() {
        for (e, want) in [("2+3*4", 14.0), ("(2+3)*4", 20.0), ("2^10", 1024.0), ("-3^2", -9.0), ("15% of 80", 12.0),
                          ("200 * 15%", 30.0), ("17 % 5", 2.0), ("sqrt(144)", 12.0), ("1,250 / 4", 312.5), ("2 x 3", 6.0),
                          ("1.5e3", 1500.0), ("round(2.5)", 3.0), ("10 ÷ 4", 2.5), ("2^-1", 0.5), ("2^3^2", 512.0)] {
            assert!((evaluate(e).unwrap() - want).abs() < 1e-9, "{e} = {:?}", evaluate(e));
        }
        assert!((evaluate("sin(30)").unwrap() - 0.5).abs() < 1e-9, "degrees");
        assert!(evaluate("1/0").is_err() && evaluate("2+").is_err() && evaluate("rm -rf").is_err() && evaluate("sqrt(-1)").is_err());
        assert_eq!(tidy(0.1 + 0.2), "0.3");
    }

    #[test]
    fn conversions() {
        let c = |v: f64, f: &str, t: &str| convert(&json!({"value": v, "from": f, "to": t})).map(|r| r["result"].as_str().unwrap().to_string());
        assert_eq!(c(10.0, "km", "miles").unwrap(), "10 km = 6.2137 mi");
        assert_eq!(c(100.0, "F", "celsius").unwrap(), "100 °F = 37.78 °C");
        assert_eq!(c(2.0, "GB", "MB").unwrap(), "2 GB = 2048 MB");
        assert!(c(1.0, "kg", "km").unwrap_err().contains("can't convert"));
    }

    #[test]
    fn live_facts_have_a_shape() {
        assert!(now()["date"].as_str().unwrap().len() > 8);
        assert!(battery()["summary"].is_string());
        let t = top_processes(&json!({"limit": 3}));
        assert!(t["by_cpu"].as_array().unwrap().len() <= 3 && t["processes"].as_u64().unwrap() > 0);
    }

    #[test]
    fn the_desktop_and_other_users_are_safe() {
        assert!(stop_process(&json!({"pid": 999_999_999})).is_err());
        assert!(stop_process(&json!({"name": "no-such-program-xyz"})).is_err());
        assert!(PROTECTED.contains(&"sway") && PROTECTED.contains(&"waybar"));
    }
}
