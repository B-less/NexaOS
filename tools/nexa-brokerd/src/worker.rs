//! Performs one checked action. Runs as a separate process (`nexa-brokerd
//! exec`) that first confines itself with Landlock to what that action
//! needs, so a bug in one action can't reach the rest of the system.
//! Setting changes report the previous value, so they can be undone.

use crate::policy::Action;
use crate::{apps, drives, users, vpn};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn sh(prog: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(prog).args(args).env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").output()
        .map_err(|e| format!("couldn't run {prog}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    if out.status.success() { Ok(text) } else { Err(text.trim().lines().last().unwrap_or("failed").to_string()) }
}

/// Is this kernel log line about why hardware doesn't work?
pub fn hardware_line(line: &str) -> bool {
    let l = line.to_lowercase();
    let topic = ["firmware", "iwlwifi", "rtw88", "rtw89", "rtl8", "mt76", "mt79", "ath9k", "ath10k", "ath11k", "ath12k",
                 "brcmf", "mwifiex", "rfkill", "wlan", "bluetooth", "i2c_hid", "i2c-hid", "elan", "synaptics", "pinctrl"]
        .iter().any(|t| l.contains(t));
    let bad = ["fail", "error", "-2", "not found", "timeout", "unable", "can't", "cannot", "invalid", "blocked"]
        .iter().any(|t| l.contains(t));
    topic && bad
}

/// The kernel log's hardware lines (see hardware_line), oldest first, at
/// most 60 of 300 characters. Reads what is there and stops.
fn hardware_log(kmsg: &Path) -> Vec<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let Ok(mut f) = fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(kmsg) else { return vec![] };
    let mut out: Vec<String> = Vec::new();
    let mut buf = vec![0u8; 8192];
    for _ in 0..50000 {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let rec = String::from_utf8_lossy(&buf[..n]);
                let msg = rec.split_once(';').map(|(_, m)| m).unwrap_or(&rec).lines().next().unwrap_or("").to_string();
                if hardware_line(&msg) && out.last() != Some(&msg) {
                    out.push(msg.chars().take(300).collect());
                }
            }
            Err(e) if e.raw_os_error() == Some(libc::EPIPE) => continue,
            Err(_) => break,
        }
    }
    let skip = out.len().saturating_sub(60);
    out.split_off(skip)
}

fn read(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// cpufreq policy files for every CPU.
fn governor_files(root: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(root.join("sys/devices/system/cpu")).into_iter().flatten().flatten()
        .map(|e| e.path().join("cpufreq/scaling_governor")).filter(|p| p.exists()).collect();
    v.sort();
    v
}

fn backlight(root: &Path) -> Option<PathBuf> {
    // Prefer the firmware/platform device over raw panel ones.
    let mut devs: Vec<PathBuf> = fs::read_dir(root.join("sys/class/backlight")).ok()?.flatten().map(|e| e.path()).collect();
    devs.sort_by_key(|d| match read(&d.join("type")).as_deref() { Some("firmware") => 0, Some("platform") => 1, _ => 2 });
    devs.into_iter().next()
}

fn undo(action: &str, args: Value) -> Value {
    json!({"action": action, "args": args})
}

/// Who asked (set by the service from SO_PEERCRED, never by the client).
pub struct Caller {
    pub uid: u32,
    pub user: String,
}

fn rc_quiet(args: &[&str], prog: &str) -> Result<(), String> {
    // Output to /dev/null: a daemon a service starts may keep inherited
    // pipes open forever, which would hang the broker waiting for EOF.
    let st = Command::new(prog).args(args).env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .status().map_err(|e| format!("couldn't run {prog}: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("{prog} {} failed", args.join(" "))) }
}

pub fn perform(a: &Action, root: &Path, caller: &Caller) -> Result<Value, String> {
    let vpn_paths = (root.join("etc/wireguard"), root.join("etc/resolv.conf"), root.join("run/nexa-vpn/resolv.conf.before-vpn"));
    let vp = vpn::Paths { wg: &vpn_paths.0, resolv: &vpn_paths.1, backup: &vpn_paths.2 };
    match a {
        Action::Reboot | Action::Poweroff => {
            // SAFETY: sync(2) has no failure mode.
            unsafe { libc::sync() };
            let prog = if *a == Action::Reboot { "reboot" } else { "poweroff" };
            sh(prog, &[])?;
            Ok(json!({"done": a.describe()}))
        }
        Action::Suspend => {
            unsafe { libc::sync() };
            // Blocks until the computer wakes up.
            fs::write(root.join("sys/power/state"), "mem").map_err(|e| format!("couldn't sleep: {e}"))?;
            Ok(json!({"done": "Woke up from sleep"}))
        }
        Action::SetGovernor(g) => {
            let files = governor_files(root);
            if files.is_empty() {
                return Err("this computer doesn't let the CPU mode be changed".into());
            }
            let available = read(&files[0].with_file_name("scaling_available_governors")).unwrap_or_default();
            if !available.is_empty() && !available.split_whitespace().any(|x| x == g) {
                return Err(format!("this CPU offers: {available}"));
            }
            let before = read(&files[0]).unwrap_or_default();
            for f in &files {
                fs::write(f, g).map_err(|e| format!("{}: {e}", f.display()))?;
            }
            Ok(json!({"done": a.describe(), "previous": before, "undo": undo("set_governor", json!({"governor": before}))}))
        }
        Action::SetBrightness(pct) => {
            let dev = backlight(root).ok_or("this screen's brightness can't be set from software")?;
            let max: u64 = read(&dev.join("max_brightness")).and_then(|s| s.parse().ok()).filter(|m| *m > 0).ok_or("no max_brightness")?;
            let now: u64 = read(&dev.join("brightness")).and_then(|s| s.parse().ok()).unwrap_or(max);
            // Never fully dark: 1% minimum, or the screen looks broken.
            let value = ((max * (*pct as u64).max(1)) / 100).max(1);
            fs::write(dev.join("brightness"), value.to_string()).map_err(|e| e.to_string())?;
            let before = ((now * 100 + max / 2) / max) as u32;
            Ok(json!({"done": a.describe(), "previous": before, "undo": undo("set_brightness", json!({"percent": before}))}))
        }
        Action::SetTimezone(z) => {
            let etc = root.join("etc");
            let before = read(&etc.join("timezone")).unwrap_or_else(|| "UTC".into());
            let target = root.join("usr/share/zoneinfo").join(z);
            let tmp = etc.join(".localtime.nexa");
            let _ = fs::remove_file(&tmp);
            // A copy, not a symlink: /usr may live on another (read-only) layer.
            fs::copy(&target, &tmp).map_err(|e| e.to_string())?;
            fs::rename(&tmp, etc.join("localtime")).map_err(|e| e.to_string())?;
            fs::write(etc.join("timezone"), format!("{z}\n")).map_err(|e| e.to_string())?;
            Ok(json!({"done": a.describe(), "previous": before, "undo": undo("set_timezone", json!({"zone": before}))}))
        }
        Action::SetHostname(n) => {
            let etc = root.join("etc");
            let before = read(&etc.join("hostname")).unwrap_or_default();
            fs::write(etc.join("hostname"), format!("{n}\n")).map_err(|e| e.to_string())?;
            // Keep the name resolvable: 127.0.1.1 <name> in /etc/hosts.
            let hosts = fs::read_to_string(etc.join("hosts")).unwrap_or_default();
            let mut lines: Vec<String> = hosts.lines().filter(|l| !l.starts_with("127.0.1.1")).map(str::to_string).collect();
            lines.push(format!("127.0.1.1\t{n}"));
            fs::write(etc.join("hosts"), lines.join("\n") + "\n").map_err(|e| e.to_string())?;
            if root == Path::new("/") {
                fs::write("/proc/sys/kernel/hostname", n).map_err(|e| e.to_string())?;
            }
            Ok(json!({"done": a.describe(), "previous": before, "undo": undo("set_hostname", json!({"name": before}))}))
        }
        Action::Service { verb, name } => {
            rc_quiet(&[name, verb], "rc-service")?;
            let undo_verb = match verb.as_str() { "start" => Some("stop_service"), "stop" => Some("start_service"), _ => None };
            let mut out = json!({"done": format!("{} {name}", match verb.as_str() { "start" => "Started", "stop" => "Stopped", _ => "Restarted" })});
            if let Some(u) = undo_verb {
                out["undo"] = undo(u, json!({"name": name}));
            }
            Ok(out)
        }
        Action::ServiceAtBoot { name, enable } => {
            if *enable { sh("rc-update", &["add", name, "default"])?; } else { sh("rc-update", &["del", name, "default"])?; }
            Ok(json!({"done": a.describe(), "undo": undo(if *enable { "disable_service" } else { "enable_service" }, json!({"name": name}))}))
        }
        Action::BluetoothUnblock => {
            let mut hard = false;
            let mut found = false;
            for e in fs::read_dir(root.join("sys/class/rfkill")).into_iter().flatten().flatten() {
                let d = e.path();
                if read(&d.join("type")).as_deref() != Some("bluetooth") {
                    continue;
                }
                found = true;
                let _ = fs::write(d.join("soft"), "0");
                hard |= read(&d.join("hard")).as_deref() == Some("1");
            }
            if hard {
                return Err("Bluetooth is turned off by a hardware switch or key.".into());
            }
            Ok(json!({"done": if found { "Bluetooth radio is on" } else { "No Bluetooth radio was blocked" }}))
        }
        Action::HardwareLog => {
            // Only lines about firmware, Wi-Fi/Bluetooth drivers, radios and
            // input devices (why a card or touchpad doesn't work), never the
            // rest of the kernel log: people have no root, but need this.
            Ok(json!({"lines": hardware_log(&root.join("dev/kmsg"))}))
        }
        Action::RepairWifi => {
            // The radio: on, unless a key or switch holds it off.
            let mut hard = false;
            for e in fs::read_dir(root.join("sys/class/rfkill")).into_iter().flatten().flatten() {
                let d = e.path();
                if read(&d.join("type")).as_deref() == Some("wlan") {
                    let _ = fs::write(d.join("soft"), "0");
                    hard |= read(&d.join("hard")).as_deref() == Some("1");
                }
            }
            // Network adapters without an interface: attach the driver again
            // (its firmware is on the system disk, which wasn't there yet
            // when the kernel first looked).
            let pci = root.join("sys/bus/pci");
            let mut reattached = Vec::new();
            for e in fs::read_dir(pci.join("devices")).into_iter().flatten().flatten() {
                let d = e.path();
                let class = read(&d.join("class")).unwrap_or_default();
                if !class.starts_with("0x02") || d.join("net").exists() || d.join("ieee80211").exists() {
                    continue;
                }
                let id = e.file_name().to_string_lossy().into_owned();
                if d.join("driver").exists() {
                    let _ = fs::write(d.join("driver/unbind"), &id);
                }
                let _ = fs::write(pci.join("drivers_probe"), &id);
                reattached.push(id);
            }
            // Wi-Fi on the SD bus (Celeron/Atom laptops) and USB Wi-Fi: a
            // driver that found no firmware lets go, so offer unbound ones
            // again (USB: vendor-specific or wireless interfaces only).
            for bus in ["sdio", "usb"] {
                let b = root.join("sys/bus").join(bus);
                for e in fs::read_dir(b.join("devices")).into_iter().flatten().flatten() {
                    let d = e.path();
                    let id = e.file_name().to_string_lossy().into_owned();
                    if d.join("driver").exists() || (bus == "usb" && !id.contains(':')) {
                        continue;
                    }
                    if bus == "usb" && !matches!(read(&d.join("bInterfaceClass")).as_deref(), Some("ff" | "e0")) {
                        continue;
                    }
                    let _ = fs::write(b.join("drivers_probe"), &id);
                    reattached.push(id);
                }
            }
            if hard {
                return Err("Wi-Fi is switched off by the keyboard's Wi-Fi key or a switch - press it once.".into());
            }
            Ok(json!({"done": if reattached.is_empty() { "The Wi-Fi radio is on".to_string() }
                              else { format!("Restarted the Wi-Fi adapter ({})", reattached.join(", ")) },
                      "reattached": reattached}))
        }
        Action::WriteAiConfig(data) => {
            let path = root.join("etc/nexa-ai/config.json");
            let mut cur: serde_json::Map<String, Value> = fs::read(&path).ok()
                .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            for (k, v) in data {
                cur.insert(k.clone(), v.clone());
            }
            fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, serde_json::to_vec_pretty(&cur).unwrap()).map_err(|e| e.to_string())?;
            // root:1000 0640: the desktop user reads it (it holds the API key).
            let c = std::ffi::CString::new(tmp.to_string_lossy().as_bytes()).unwrap();
            // SAFETY: chown on a path we just created.
            unsafe { libc::chown(c.as_ptr(), 0, 1000) };
            fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o640)).map_err(|e| e.to_string())?;
            fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            Ok(json!({"done": "Saved the assistant settings"}))
        }
        Action::UpdateIndex => {
            sh("apk", &["update"])?;
            let list = parse_updates(&sh("apk", &["version", "-l", "<"]).unwrap_or_default());
            let n = list.len();
            Ok(json!({"done": if n == 0 { "Everything is up to date".to_string() } else { format!("{n} update{} available", if n == 1 { "" } else { "s" }) },
                      "updates": n,
                      "packages": list.iter().take(200).map(|(name, from, to)| json!({"name": name, "from": from, "to": to})).collect::<Vec<_>>()}))
        }
        Action::ListServices => Ok(list_services(root)),
        Action::SystemUpdate => {
            sh("apk", &["upgrade", "-U", "--no-interactive"])?;
            Ok(json!({"done": "The system is up to date"}))
        }
        Action::InstallApp { path, allow_unverified } => apps::install(&apps::Places::under(root), caller.uid, path, *allow_unverified, true),
        Action::UninstallApp(id) => apps::uninstall(&apps::Places::under(root), id),
        Action::VpnList => Ok(vpn::list(&vp)),
        Action::VpnImport { name, config } => vpn::import(&vp, name, config),
        Action::VpnUp(n) => vpn::up(&vp, n),
        Action::VpnDown(n) => vpn::down(&vp, n),
        Action::VpnRemove(n) => vpn::remove(&vp, n),
        Action::ListDrives => Ok(drives::list(&drives::Tree::real())),
        Action::MountDrive(k) | Action::UnmountDrive(k) | Action::EjectDrive(k) => {
            let t = drives::Tree::real();
            let r = match a {
                Action::MountDrive(_) => drives::mount(&t, k),
                Action::UnmountDrive(_) => drives::unmount(&t, k),
                _ => drives::eject(&t, k),
            };
            drives::publish(&t);
            r
        }
        Action::ChangePassword { new_password } => users::change_password(&caller.user, new_password),
        Action::AddUser { name, full_name, password, admin } => users::add(&root.join("etc"), name, full_name, password, *admin),
        Action::RemoveUser(name) => users::remove(&root.join("etc"), &caller.user, name),
        Action::InstallPackages(list) => {
            let mut args = vec!["add", "--no-interactive", "--"];
            args.extend(list.iter().map(String::as_str));
            sh("apk", &args)?;
            Ok(json!({"done": format!("Installed {}", list.join(", ")), "undo": undo("remove_packages", json!({"names": list}))}))
        }
        Action::RemovePackages(list) => {
            let mut args = vec!["del", "--no-interactive", "--"];
            args.extend(list.iter().map(String::as_str));
            sh("apk", &args)?;
            Ok(json!({"done": format!("Removed {}", list.join(", ")), "undo": undo("install_packages", json!({"names": list}))}))
        }
    }
}

/// `apk version -l "<"` lines ("busybox-1.36.1-r0   < 1.36.1-r1") as
/// (name, installed version, new version). Names may contain dashes.
pub fn parse_updates(text: &str) -> Vec<(String, String, String)> {
    text.lines().filter_map(|l| {
        let (left, right) = l.split_once('<')?;
        let full = left.trim();
        let to = right.trim().to_string();
        // name-version-rN: the version is the last dash-part starting with a digit before -rN.
        let parts: Vec<&str> = full.rsplitn(3, '-').collect();
        if parts.len() < 3 || !parts[0].starts_with('r') || !parts[1].chars().next()?.is_ascii_digit() {
            return None;
        }
        Some((parts[2].to_string(), format!("{}-{}", parts[1], parts[0]), to))
    }).collect()
}

/// Services this computer has: which start with it, which run now, and
/// which are part of the system itself (no switch for those).
pub fn list_services(root: &Path) -> Value {
    let init = root.join("etc/init.d");
    let mut names: Vec<String> = fs::read_dir(&init).into_iter().flatten().flatten()
        .filter(|e| e.path().is_file()).filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.') && !n.ends_with(".sh") && n != "functions.sh").collect();
    names.sort();
    let services: Vec<Value> = names.iter().map(|n| {
        let script = fs::read_to_string(init.join(n)).unwrap_or_default();
        let description = script.lines().find_map(|l| l.trim().strip_prefix("description=")).map(|d| d.trim_matches(['"', '\'']).to_string()).unwrap_or_default();
        let level = ["sysinit", "boot", "default"].into_iter().find(|lv| root.join("etc/runlevels").join(lv).join(n).exists());
        json!({"name": n, "description": description, "at_startup": level.is_some(), "runlevel": level,
               "running": root.join("run/openrc/started").join(n).exists(),
               "protected": crate::policy::PROTECTED_SERVICES.contains(&n.as_str())})
    }).collect();
    json!({"services": services})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Caller {
        Caller { uid: 1000, user: "user".into() }
    }

    #[test]
    fn update_lists_and_services() {
        let out = "Installed:                                Available:\nbusybox-1.36.1-r0              < 1.36.1-r1\nllama.cpp-cpu-0.0.9564-r0      < 0.0.9600-r0\ngst-plugins-good-1.26.5-r0     < 1.26.6-r0\n";
        assert_eq!(parse_updates(out), vec![
            ("busybox".into(), "1.36.1-r0".into(), "1.36.1-r1".into()),
            ("llama.cpp-cpu".into(), "0.0.9564-r0".into(), "0.0.9600-r0".into()),
            ("gst-plugins-good".into(), "1.26.5-r0".into(), "1.26.6-r0".into())]);
        let r = fake_root("svc");
        fs::create_dir_all(r.join("etc/init.d")).unwrap();
        fs::create_dir_all(r.join("etc/runlevels/default")).unwrap();
        fs::create_dir_all(r.join("run/openrc/started")).unwrap();
        fs::write(r.join("etc/init.d/tor"), "#!/sbin/openrc-run\ndescription=\"Tor anonymity network\"\n").unwrap();
        fs::write(r.join("etc/init.d/dbus"), "#!/sbin/openrc-run\n").unwrap();
        fs::write(r.join("etc/runlevels/default/dbus"), "").unwrap();
        fs::write(r.join("run/openrc/started/dbus"), "").unwrap();
        let v = list_services(&r);
        assert_eq!(v["services"][0], json!({"name": "dbus", "description": "", "at_startup": true, "runlevel": "default", "running": true, "protected": true}));
        assert_eq!(v["services"][1]["description"], "Tor anonymity network");
        assert_eq!((v["services"][1]["at_startup"].as_bool(), v["services"][1]["protected"].as_bool()), (Some(false), Some(false)));
    }

    #[test]
    fn assistant_settings_merge_and_bluetooth() {
        let r = fake_root("ai");
        fs::create_dir_all(r.join("etc/nexa-ai")).unwrap();
        fs::write(r.join("etc/nexa-ai/config.json"), r#"{"model": "m", "api_key": "k"}"#).unwrap();
        let mut d = serde_json::Map::new();
        d.insert("backend".into(), json!("local"));
        perform(&Action::WriteAiConfig(d), &r, &me()).unwrap();
        let v: Value = serde_json::from_slice(&fs::read(r.join("etc/nexa-ai/config.json")).unwrap()).unwrap();
        assert_eq!((v["backend"].as_str(), v["api_key"].as_str()), (Some("local"), Some("k")), "merged, nothing lost");
        let rf = r.join("sys/class/rfkill/rfkill0");
        fs::create_dir_all(&rf).unwrap();
        fs::write(rf.join("type"), "bluetooth\n").unwrap();
        fs::write(rf.join("soft"), "1\n").unwrap();
        fs::write(rf.join("hard"), "0\n").unwrap();
        perform(&Action::BluetoothUnblock, &r, &me()).unwrap();
        assert_eq!(read(&rf.join("soft")).unwrap(), "0");
        fs::write(rf.join("hard"), "1\n").unwrap();
        assert!(perform(&Action::BluetoothUnblock, &r, &me()).unwrap_err().contains("hardware switch"));
    }

    #[test]
    fn hardware_log_lines() {
        assert!(hardware_line("iwlwifi 0000:00:14.3: Direct firmware load for iwlwifi-9000-pu-b0-jf-b0-46.ucode failed with error -2"));
        assert!(hardware_line("ath10k_pci 0000:02:00.0: failed to fetch board file: -2"));
        assert!(hardware_line("i2c_hid_acpi i2c-ELAN0001:00: can't add hid device: -110"));
        assert!(!hardware_line("usb 1-2: new high-speed USB device number 3 using xhci_hcd"), "only problems");
        assert!(!hardware_line("EXT4-fs error (device sda1): something private"), "nothing outside hardware");
        // A kmsg-like file: records "prio,seq,time,-;message".
        let r = fake_root("hwlog");
        fs::create_dir_all(r.join("dev")).unwrap();
        fs::write(r.join("dev/kmsg"), "6,1,100,-;iwlwifi 0000:00:14.3: Direct firmware load for x failed with error -2\n").unwrap();
        let out = perform(&Action::HardwareLog, &r, &me()).unwrap();
        assert_eq!(out["lines"][0], "iwlwifi 0000:00:14.3: Direct firmware load for x failed with error -2", "{out}");
    }

    #[test]
    fn wifi_repair() {
        let r = fake_root("wifi");
        let rf = r.join("sys/class/rfkill/rfkill1");
        fs::create_dir_all(&rf).unwrap();
        fs::write(rf.join("type"), "wlan\n").unwrap();
        fs::write(rf.join("soft"), "1\n").unwrap();
        fs::write(rf.join("hard"), "0\n").unwrap();
        let pci = r.join("sys/bus/pci");
        // A Wi-Fi card stuck with a firmware-less driver, a working Ethernet card, a GPU.
        for (id, class, net) in [("0000:00:14.3", "0x028000", false), ("0000:00:1f.6", "0x020000", true), ("0000:00:02.0", "0x030000", false)] {
            let d = pci.join("devices").join(id);
            fs::create_dir_all(d.join("driver")).unwrap();
            fs::write(d.join("class"), format!("{class}\n")).unwrap();
            if net {
                fs::create_dir_all(d.join("net")).unwrap();
            }
        }
        // SD-bus Wi-Fi left unbound; USB: a vendor interface (Wi-Fi dongle)
        // unbound, a keyboard interface unbound, a whole device (no ':').
        fs::create_dir_all(r.join("sys/bus/sdio/devices/mmc1:0001:1")).unwrap();
        fs::create_dir_all(r.join("sys/bus/sdio/devices/mmc1:0001:2/driver")).unwrap();
        for (id, class) in [("1-2:1.0", "ff"), ("1-3:1.0", "03"), ("1-2", "00")] {
            let d = r.join("sys/bus/usb/devices").join(id);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("bInterfaceClass"), format!("{class}\n")).unwrap();
        }
        let out = perform(&Action::RepairWifi, &r, &me()).unwrap();
        assert_eq!(out["reattached"], json!(["0000:00:14.3", "mmc1:0001:1", "1-2:1.0"]), "{out}");
        assert_eq!(read(&r.join("sys/bus/sdio/drivers_probe")).unwrap(), "mmc1:0001:1");
        assert_eq!(read(&r.join("sys/bus/usb/drivers_probe")).unwrap(), "1-2:1.0");
        assert_eq!(read(&rf.join("soft")).unwrap(), "0", "radio switched on");
        assert_eq!(read(&pci.join("drivers_probe")).unwrap(), "0000:00:14.3");
        assert_eq!(read(&pci.join("devices/0000:00:14.3/driver/unbind")).unwrap(), "0000:00:14.3");
        fs::write(rf.join("hard"), "1\n").unwrap();
        assert!(perform(&Action::RepairWifi, &r, &me()).unwrap_err().contains("Wi-Fi key"));
    }

    fn fake_root(name: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("nexa-broker-worker-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        for cpu in ["cpu0", "cpu1"] {
            let d = r.join("sys/devices/system/cpu").join(cpu).join("cpufreq");
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("scaling_governor"), "powersave\n").unwrap();
            fs::write(d.join("scaling_available_governors"), "performance powersave\n").unwrap();
        }
        let bl = r.join("sys/class/backlight/intel_backlight");
        fs::create_dir_all(&bl).unwrap();
        fs::write(bl.join("max_brightness"), "19200\n").unwrap();
        fs::write(bl.join("brightness"), "9600\n").unwrap();
        fs::write(bl.join("type"), "raw\n").unwrap();
        fs::create_dir_all(r.join("etc")).unwrap();
        fs::write(r.join("etc/hostname"), "nexa\n").unwrap();
        fs::write(r.join("etc/hosts"), "127.0.0.1\tlocalhost\n127.0.1.1\tnexa\n").unwrap();
        fs::create_dir_all(r.join("usr/share/zoneinfo/Africa")).unwrap();
        fs::write(r.join("usr/share/zoneinfo/Africa/Accra"), "TZif2-accra").unwrap();
        r
    }

    #[test]
    fn governor_with_undo() {
        let r = fake_root("gov");
        let out = perform(&Action::SetGovernor("performance".into()), &r, &me()).unwrap();
        assert_eq!(out["previous"], "powersave");
        assert_eq!(out["undo"], json!({"action": "set_governor", "args": {"governor": "powersave"}}));
        assert_eq!(read(&r.join("sys/devices/system/cpu/cpu1/cpufreq/scaling_governor")).unwrap(), "performance");
        assert!(perform(&Action::SetGovernor("ondemand".into()), &r, &me()).unwrap_err().contains("offers"));
    }

    #[test]
    fn brightness_never_goes_black() {
        let r = fake_root("bl");
        let out = perform(&Action::SetBrightness(25), &r, &me()).unwrap();
        assert_eq!(out["previous"], 50);
        assert_eq!(read(&r.join("sys/class/backlight/intel_backlight/brightness")).unwrap(), "4800");
        perform(&Action::SetBrightness(0), &r, &me()).unwrap();
        assert_eq!(read(&r.join("sys/class/backlight/intel_backlight/brightness")).unwrap(), "192", "1% minimum");
    }

    #[test]
    fn timezone_and_hostname() {
        let r = fake_root("tz");
        let out = perform(&Action::SetTimezone("Africa/Accra".into()), &r, &me()).unwrap();
        assert_eq!(out["previous"], "UTC");
        assert_eq!(fs::read_to_string(r.join("etc/localtime")).unwrap(), "TZif2-accra");
        assert_eq!(read(&r.join("etc/timezone")).unwrap(), "Africa/Accra");
        let out = perform(&Action::SetHostname("studio".into()), &r, &me()).unwrap();
        assert_eq!(out["previous"], "nexa");
        let hosts = fs::read_to_string(r.join("etc/hosts")).unwrap();
        assert!(hosts.contains("127.0.1.1\tstudio") && !hosts.contains("\tnexa") && hosts.contains("localhost"), "{hosts}");
    }
}
