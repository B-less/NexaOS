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
            let out = sh("apk", &["version", "-l", "<"]).unwrap_or_default();
            let updates = out.lines().filter(|l| l.contains('<')).count();
            Ok(json!({"done": format!("Checked for new software: {updates} update{} available", if updates == 1 { "" } else { "s" }), "updates": updates}))
        }
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
        Action::MountDrive(k) => drives::mount(&drives::Tree::real(), k),
        Action::UnmountDrive(k) => drives::unmount(&drives::Tree::real(), k),
        Action::EjectDrive(k) => drives::eject(&drives::Tree::real(), k),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Caller {
        Caller { uid: 1000, user: "user".into() }
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
