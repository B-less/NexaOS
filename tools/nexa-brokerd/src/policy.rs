//! Every system change the broker will make, with its arguments checked
//! field by field. Anything not listed here can't be asked for at all.
//!
//! Each action also says:
//! - who must confirm it: `Allow` (the person at the computer, already
//!   asked by the app's own approval card) or `Password` (their password,
//!   checked against /etc/shadow; the live USB has none, so it allows);
//! - how its worker is confined (Landlock), and how long it may take.
//!   Starting services and mounting drives run unconfined on purpose: a
//!   Landlock domain is inherited for life (a service started inside one
//!   couldn't open its ports), and Landlock forbids mount(2) altogether.
//!   For those, the checks here are the guard.

use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Allow,
    Password,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    // power
    Reboot,
    Poweroff,
    Suspend,
    // hardware and settings
    SetGovernor(String),
    SetBrightness(u32),
    SetTimezone(String),
    SetHostname(String),
    BluetoothUnblock,
    /// Wi-Fi radio on and the adapter's driver re-attached (Nexa Doctor's fix).
    RepairWifi,
    WriteAiConfig(serde_json::Map<String, Value>),
    // services
    Service { verb: String, name: String },
    ServiceAtBoot { name: String, enable: bool },
    ListServices,
    // software
    UpdateIndex,
    SystemUpdate,
    InstallPackages(Vec<String>),
    RemovePackages(Vec<String>),
    InstallApp { path: String, allow_unverified: bool },
    UninstallApp(String),
    // VPN (WireGuard)
    VpnList,
    VpnImport { name: String, config: String },
    VpnUp(String),
    VpnDown(String),
    VpnRemove(String),
    // removable drives
    ListDrives,
    MountDrive(String),
    UnmountDrive(String),
    EjectDrive(String),
    // people
    ChangePassword { new_password: String },
    AddUser { name: String, full_name: String, password: String, admin: bool },
    RemoveUser(String),
}

/// How the worker is confined.
pub struct Access {
    /// false: no Landlock (see the module comment for which and why).
    pub sandbox: bool,
    pub write: Vec<&'static str>,
    pub exec: bool,
    pub net: Vec<u16>,
    pub timeout_s: u64,
}

pub const GOVERNORS: &[&str] = &["performance", "powersave", "ondemand", "conservative", "schedutil", "userspace"];

/// Never started, stopped, restarted or switched off at boot from here.
/// (sshd may be started, but only with a password.)
pub const PROTECTED_SERVICES: &[&str] = &[
    "dbus", "seatd", "udev", "udev-trigger", "udev-settle", "udev-postmount", "mdev", "devfs", "dmesg", "sysfs",
    "procfs", "localmount", "root", "fsck", "hwdrivers", "modules", "bootmisc", "hostname", "sysctl", "hwclock",
    "killprocs", "mount-ro", "savecache", "nexa-broker", "nexa-session", "sway", "networking", "firewall",
    "nftables", "syslog", "netdhcp", "zram", "bootsplash",
];

/// Removing these would break the system.
pub const PROTECTED_PACKAGES: &[&str] = &[
    "alpine-base", "busybox", "musl", "apk-tools", "openrc", "alpine-baselayout", "alpine-keys", "libc-utils",
    "doas", "linux-firmware", "sway", "swaybg", "waybar", "mako", "gtk+3.0", "python3", "py3-gobject3", "dbus",
    "seatd", "eudev", "iwd", "bluez", "nftables", "pipewire", "wireplumber", "ca-certificates", "tzdata", "curl",
    "mesa", "font-inter", "llama-server", "llama.cpp-cpu", "wireguard-tools", "shadow",
];

pub const RESERVED_USERS: &[&str] = &[
    "root", "bin", "daemon", "adm", "lp", "sync", "shutdown", "halt", "mail", "news", "uucp", "operator", "man",
    "postmaster", "cron", "ftp", "sshd", "at", "squid", "xfs", "games", "cyrus", "vpopmail", "ntp", "smmsp",
    "guest", "nobody", "messagebus", "seatd", "seat", "wheel", "video", "audio", "input", "admin", "nexa-mind",
    "shadow",
];
pub const MIN_PASSWORD: usize = 4;

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| format!("missing '{key}'"))
}

fn valid_service_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && !n.starts_with('.')
}

/// Alpine package names: letters, digits and + - _ . (no options, no paths).
pub fn valid_package_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 100 && n.chars().next().map_or(false, |c| c.is_ascii_alphanumeric())
        && n.chars().all(|c| c.is_ascii_alphanumeric() || "+-_.".contains(c))
}

/// .nexa package ids: lowercase, digits, - _ (they become command names).
pub fn valid_app_id(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().next().map_or(false, |c| c.is_ascii_lowercase())
        && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_".contains(c))
}

/// RFC 1123 host name label.
pub fn valid_hostname(n: &str) -> bool {
    !n.is_empty() && n.len() <= 63 && !n.starts_with('-') && !n.ends_with('-')
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// "Africa/Accra": letters, digits, + - _ and / between parts, no "..".
pub fn valid_zone(z: &str, zoneinfo: &Path) -> bool {
    !z.is_empty() && z.len() <= 64 && !z.starts_with('/') && !z.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        && z.chars().all(|c| c.is_ascii_alphanumeric() || "+-_/".contains(c))
        && zoneinfo.join(z).is_file()
}

/// Linux interface-name rules (a tunnel is an interface).
pub fn valid_tunnel_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 15 && n.chars().all(|c| c.is_ascii_alphanumeric() || "_=+.-".contains(c))
}

pub fn valid_username(n: &str) -> bool {
    !n.is_empty() && n.len() <= 31 && n.chars().next().map_or(false, |c| c.is_ascii_lowercase())
        && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_".contains(c))
        && !RESERVED_USERS.contains(&n)
}

pub fn check_new_password(p: &str) -> Result<(), String> {
    if p.chars().count() < MIN_PASSWORD {
        return Err(format!("the password must be at least {MIN_PASSWORD} characters"));
    }
    if p.contains(['\n', '\r', '\0', ':']) {
        return Err("the password can't contain line breaks or ':'".into());
    }
    Ok(())
}

/// sda1, mmcblk0p1: the only device names the drive actions accept.
pub fn valid_kname(n: &str) -> bool {
    let sd = n.strip_prefix("sd").map_or(false, |r| {
        let letters = r.chars().take_while(|c| c.is_ascii_lowercase()).count();
        letters > 0 && r[letters..].chars().all(|c| c.is_ascii_digit())
    });
    let mmc = n.strip_prefix("mmcblk").map_or(false, |r| {
        let (disk, part) = r.split_once('p').unwrap_or((r, ""));
        !disk.is_empty() && disk.chars().all(|c| c.is_ascii_digit()) && part.chars().all(|c| c.is_ascii_digit())
            && (r.contains('p') == !part.is_empty())
    });
    n.len() <= 16 && (sd || mmc)
}

fn packages(args: &Value) -> Result<Vec<String>, String> {
    let list: Vec<String> = match args.get("names").or_else(|| args.get("name")) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(s)) => s.split_whitespace().map(str::to_string).collect(),
        _ => return Err("missing 'names'".into()),
    };
    if list.is_empty() || list.len() > 20 {
        return Err("between 1 and 20 packages".into());
    }
    if let Some(bad) = list.iter().find(|n| !valid_package_name(n)) {
        return Err(format!("'{bad}' isn't a valid package name"));
    }
    Ok(list)
}

fn ai_config(args: &Value) -> Result<serde_json::Map<String, Value>, String> {
    let data = args.get("data").and_then(Value::as_object).ok_or("missing 'data'")?;
    for (k, v) in data {
        let ok = match k.as_str() {
            "api_key" | "model" => v.as_str().map_or(false, |s| s.len() <= 512 && !s.contains(['\n', '\0'])),
            "backend" => matches!(v.as_str(), Some("auto" | "local" | "claude")),
            "allow_control" | "require_confirmation" => v.is_boolean(),
            _ => return Err(format!("unexpected field '{k}' in the assistant settings")),
        };
        if !ok {
            return Err(format!("'{k}' has the wrong kind of value"));
        }
    }
    Ok(data.clone())
}

/// Parse and check a request. `root` prefixes system paths (tests).
pub fn parse(action: &str, args: &Value, root: &Path) -> Result<Action, String> {
    let tunnel = |args: &Value| -> Result<String, String> {
        let n = str_arg(args, "name")?;
        if valid_tunnel_name(n) { Ok(n.to_string()) } else { Err("VPN names use letters, digits and - _ . (up to 15)".into()) }
    };
    let drive = |args: &Value| -> Result<String, String> {
        let n = str_arg(args, "name")?;
        if valid_kname(n) { Ok(n.to_string()) } else { Err("that isn't a removable drive".into()) }
    };
    Ok(match action {
        "reboot" => Action::Reboot,
        "poweroff" => Action::Poweroff,
        "suspend" => Action::Suspend,
        "set_governor" => {
            let g = str_arg(args, "governor")?;
            if !GOVERNORS.contains(&g) {
                return Err(format!("'{g}' isn't a CPU governor ({})", GOVERNORS.join(", ")));
            }
            Action::SetGovernor(g.into())
        }
        "set_brightness" => {
            let p = args.get("percent").and_then(Value::as_f64).ok_or("missing 'percent'")?;
            if !(0.0..=100.0).contains(&p) {
                return Err("brightness is 0 to 100 percent".into());
            }
            Action::SetBrightness(p.round() as u32)
        }
        "set_timezone" => {
            let z = str_arg(args, "zone")?;
            if !valid_zone(z, &root.join("usr/share/zoneinfo")) {
                return Err(format!("'{z}' isn't a known time zone (like Africa/Accra or Europe/London)"));
            }
            Action::SetTimezone(z.into())
        }
        "set_hostname" => {
            let n = str_arg(args, "name")?.to_ascii_lowercase();
            if !valid_hostname(&n) {
                return Err(format!("'{n}' isn't a valid computer name (letters, digits and -, up to 63)"));
            }
            Action::SetHostname(n)
        }
        "bluetooth_unblock" => Action::BluetoothUnblock,
        "repair_wifi" => Action::RepairWifi,
        "write_ai_config" => Action::WriteAiConfig(ai_config(args)?),
        "start_service" | "stop_service" | "restart_service" | "enable_service" | "disable_service" => {
            let name = str_arg(args, "name")?;
            if !valid_service_name(name) || !root.join("etc/init.d").join(name).is_file() {
                return Err(format!("there is no service called '{name}'"));
            }
            if PROTECTED_SERVICES.contains(&name) {
                return Err(format!("'{name}' is part of the system itself and can't be changed from here"));
            }
            match action {
                "enable_service" => Action::ServiceAtBoot { name: name.into(), enable: true },
                "disable_service" => Action::ServiceAtBoot { name: name.into(), enable: false },
                _ => Action::Service { verb: action.trim_end_matches("_service").into(), name: name.into() },
            }
        }
        "list_services" => Action::ListServices,
        "update_index" => Action::UpdateIndex,
        "system_update" => Action::SystemUpdate,
        "install_packages" => Action::InstallPackages(packages(args)?),
        "remove_packages" => {
            let list = packages(args)?;
            if let Some(p) = list.iter().find(|n| PROTECTED_PACKAGES.contains(&n.as_str()) || n.starts_with("linux-firmware")) {
                return Err(format!("'{p}' is part of NexaOS itself and can't be removed"));
            }
            Action::RemovePackages(list)
        }
        "install_app" => {
            let path = str_arg(args, "path")?;
            if !path.starts_with('/') || !path.ends_with(".nexa") || path.contains('\0') || path.len() > 4096 {
                return Err("give the full path of a .nexa file".into());
            }
            Action::InstallApp { path: path.into(), allow_unverified: args.get("allow_unverified") == Some(&Value::Bool(true)) }
        }
        "uninstall_app" => {
            let id = str_arg(args, "id")?;
            if !valid_app_id(id) {
                return Err("invalid app id".into());
            }
            Action::UninstallApp(id.into())
        }
        "vpn_list" => Action::VpnList,
        "vpn_import" => {
            let config = str_arg(args, "config")?;
            if config.len() > 64 * 1024 {
                return Err("that VPN file is too big".into());
            }
            Action::VpnImport { name: tunnel(args)?, config: config.into() }
        }
        "vpn_up" => Action::VpnUp(tunnel(args)?),
        "vpn_down" => Action::VpnDown(tunnel(args)?),
        "vpn_remove" => Action::VpnRemove(tunnel(args)?),
        "list_drives" => Action::ListDrives,
        "mount_drive" => Action::MountDrive(drive(args)?),
        "unmount_drive" => Action::UnmountDrive(drive(args)?),
        "eject_drive" => Action::EjectDrive(drive(args)?),
        "change_password" => {
            let p = str_arg(args, "new_password")?;
            check_new_password(p)?;
            Action::ChangePassword { new_password: p.into() }
        }
        "add_user" => {
            let name = str_arg(args, "name")?;
            if !valid_username(name) {
                return Err("user names start with a lowercase letter and use a-z, 0-9, - and _ (up to 31)".into());
            }
            let full = args.get("full_name").and_then(Value::as_str).unwrap_or("");
            if full.len() > 64 || full.contains([':', '\n', '\0', ',']) {
                return Err("that full name has characters that aren't allowed".into());
            }
            let password = str_arg(args, "password")?;
            check_new_password(password)?;
            Action::AddUser { name: name.into(), full_name: full.into(), password: password.into(),
                              admin: args.get("admin") == Some(&Value::Bool(true)) }
        }
        "remove_user" => {
            let name = str_arg(args, "name")?;
            if !valid_username(name) {
                return Err("invalid user name".into());
            }
            Action::RemoveUser(name.into())
        }
        other => return Err(format!("'{other}' isn't something the system broker does")),
    })
}

impl Action {
    pub fn level(&self) -> Level {
        use Action::*;
        match self {
            SetHostname(_) | InstallPackages(_) | RemovePackages(_) | SystemUpdate | InstallApp { .. } | UninstallApp(_)
            | ServiceAtBoot { .. } | ChangePassword { .. } | AddUser { .. } | RemoveUser(_) => Level::Password,
            Service { name, verb } if name == "sshd" || verb == "stop" => Level::Password,
            _ => Level::Allow,
        }
    }

    pub fn access(&self) -> Access {
        use Action::*;
        let a = |sandbox: bool, write: Vec<&'static str>, exec: bool, net: Vec<u16>, timeout_s: u64| Access { sandbox, write, exec, net, timeout_s };
        match self {
            Reboot | Poweroff => a(true, vec![], true, vec![], 20),
            Suspend => a(true, vec!["/sys/power"], false, vec![], 24 * 3600),
            SetGovernor(_) => a(true, vec!["/sys/devices/system/cpu"], false, vec![], 10),
            SetBrightness(_) => a(true, vec!["/sys/devices", "/sys/class/backlight"], false, vec![], 10),
            BluetoothUnblock => a(true, vec!["/sys/devices", "/sys/class/rfkill"], false, vec![], 10),
            RepairWifi => a(true, vec!["/sys/devices", "/sys/class/rfkill", "/sys/bus/pci", "/sys/bus/sdio", "/sys/bus/usb"], false, vec![], 30),
            SetTimezone(_) => a(true, vec!["/etc"], false, vec![], 10),
            SetHostname(_) => a(true, vec!["/etc", "/proc/sys/kernel"], true, vec![], 10),
            WriteAiConfig(_) => a(true, vec!["/etc/nexa-ai"], false, vec![], 10),
            // Unconfined: started services would inherit the domain for life.
            Service { .. } => a(false, vec![], true, vec![], 120),
            ServiceAtBoot { .. } => a(true, vec!["/etc/runlevels"], true, vec![], 30),
            // apk writes all over the system and fetches over HTTPS.
            UpdateIndex => a(true, vec!["/var/cache/apk", "/lib/apk", "/etc/apk"], true, vec![80, 443], 300),
            SystemUpdate | InstallPackages(_) | RemovePackages(_) => a(true, vec!["/"], true, vec![80, 443], 3600),
            // Installing an app may install its Alpine dependencies too.
            InstallApp { .. } | UninstallApp(_) => a(true, vec!["/"], true, vec![80, 443], 1800),
            VpnList => a(true, vec![], true, vec![], 15),
            ListServices => a(true, vec![], false, vec![], 10),
            VpnImport { .. } | VpnUp(_) | VpnDown(_) | VpnRemove(_) => a(true, vec!["/etc", "/run"], true, vec![], 45),
            // Unconfined: Landlock forbids mount(2).
            ListDrives | MountDrive(_) | UnmountDrive(_) | EjectDrive(_) => a(false, vec![], true, vec![], 60),
            ChangePassword { .. } | AddUser { .. } | RemoveUser(_) => a(true, vec!["/etc", "/home"], true, vec![], 60),
        }
    }

    /// One line for the audit log and the approval card. Never includes a password.
    pub fn describe(&self) -> String {
        use Action::*;
        match self {
            Reboot => "Restart the computer".into(),
            Poweroff => "Shut down the computer".into(),
            Suspend => "Put the computer to sleep".into(),
            SetGovernor(g) => format!("Set the CPU to {g} mode"),
            SetBrightness(p) => format!("Set the screen brightness to {p}%"),
            SetTimezone(z) => format!("Set the time zone to {z}"),
            SetHostname(n) => format!("Rename this computer to {n}"),
            BluetoothUnblock => "Turn the Bluetooth radio on".into(),
            RepairWifi => "Repair the Wi-Fi adapter".into(),
            WriteAiConfig(d) => format!("Change the assistant settings ({})", d.keys().cloned().collect::<Vec<_>>().join(", ")),
            Service { verb, name } => format!("{} the {name} service", capitalise(verb)),
            ServiceAtBoot { name, enable } => format!("{} {name} at startup", if *enable { "Start" } else { "Don't start" }),
            UpdateIndex => "Check for new software".into(),
            SystemUpdate => "Update the system".into(),
            InstallPackages(p) => format!("Install {}", p.join(", ")),
            RemovePackages(p) => format!("Remove {}", p.join(", ")),
            InstallApp { path, .. } => format!("Install the app {}", path.rsplit('/').next().unwrap_or(path)),
            UninstallApp(id) => format!("Uninstall the app {id}"),
            VpnList => "List VPNs".into(),
            ListServices => "List services".into(),
            VpnImport { name, .. } => format!("Add the VPN {name}"),
            VpnUp(n) => format!("Connect the VPN {n}"),
            VpnDown(n) => format!("Disconnect the VPN {n}"),
            VpnRemove(n) => format!("Remove the VPN {n}"),
            ListDrives => "List removable drives".into(),
            MountDrive(n) => format!("Open the drive {n}"),
            UnmountDrive(n) => format!("Close the drive {n}"),
            EjectDrive(n) => format!("Eject the drive {n}"),
            ChangePassword { .. } => "Change your password".into(),
            AddUser { name, admin, .. } => format!("Add the {} {name}", if *admin { "administrator" } else { "user" }),
            RemoveUser(n) => format!("Remove the user {n}"),
        }
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn root() -> std::path::PathBuf {
        // Per thread: tests run in parallel and must not share files.
        let r = std::env::temp_dir().join(format!("nexa-broker-policy-{}-{:?}", std::process::id(), std::thread::current().id())
            .replace(['(', ')'], ""));
        fs::create_dir_all(r.join("etc/init.d")).unwrap();
        fs::create_dir_all(r.join("usr/share/zoneinfo/Africa")).unwrap();
        fs::write(r.join("usr/share/zoneinfo/Africa/Accra"), "TZif").unwrap();
        for s in ["bluetooth", "sshd", "dbus", "tor"] {
            fs::write(r.join("etc/init.d").join(s), "#!/sbin/openrc-run").unwrap();
        }
        r
    }

    #[test]
    fn only_known_actions_with_valid_arguments() {
        let r = root();
        assert!(parse("rm_rf", &json!({}), &r).is_err());
        assert_eq!(parse("set_governor", &json!({"governor": "powersave"}), &r).unwrap(), Action::SetGovernor("powersave".into()));
        assert!(parse("set_governor", &json!({"governor": "turbo; reboot"}), &r).is_err());
        assert_eq!(parse("set_brightness", &json!({"percent": 42.4}), &r).unwrap(), Action::SetBrightness(42));
        assert!(parse("set_brightness", &json!({"percent": 140}), &r).is_err());
        assert!(parse("set_timezone", &json!({"zone": "Africa/Accra"}), &r).is_ok());
        for z in ["../../etc/shadow", "/etc/passwd", "Africa/../../x", "Mars/Olympus"] {
            assert!(parse("set_timezone", &json!({"zone": z}), &r).is_err(), "{z}");
        }
        assert_eq!(parse("set_hostname", &json!({"name": "Nexa-Laptop"}), &r).unwrap(), Action::SetHostname("nexa-laptop".into()));
        for n in ["-x", "a b", "a.b", "x;reboot", &"a".repeat(64)] {
            assert!(parse("set_hostname", &json!({"name": n}), &r).is_err(), "{n}");
        }
    }

    #[test]
    fn services_exist_and_the_system_is_off_limits() {
        let r = root();
        assert!(parse("restart_service", &json!({"name": "bluetooth"}), &r).is_ok());
        assert!(parse("stop_service", &json!({"name": "dbus"}), &r).unwrap_err().contains("part of the system"));
        assert!(parse("disable_service", &json!({"name": "dbus"}), &r).is_err());
        assert!(parse("start_service", &json!({"name": "nope"}), &r).unwrap_err().contains("no service"));
        assert!(parse("start_service", &json!({"name": "../../bin/sh"}), &r).is_err());
        assert_eq!(parse("restart_service", &json!({"name": "tor"}), &r).unwrap().level(), Level::Allow);
        assert_eq!(parse("stop_service", &json!({"name": "tor"}), &r).unwrap().level(), Level::Password);
        assert_eq!(parse("start_service", &json!({"name": "sshd"}), &r).unwrap().level(), Level::Password);
        assert_eq!(parse("enable_service", &json!({"name": "tor"}), &r).unwrap(), Action::ServiceAtBoot { name: "tor".into(), enable: true });
        assert!(!parse("restart_service", &json!({"name": "tor"}), &r).unwrap().access().sandbox, "services start unconfined");
    }

    #[test]
    fn packages_and_apps() {
        let r = root();
        assert_eq!(parse("install_packages", &json!({"names": ["vlc", "gimp"]}), &r).unwrap(),
                   Action::InstallPackages(vec!["vlc".into(), "gimp".into()]));
        for bad in ["--allow-untrusted", "-X", "http://evil/x.apk", "../x", "a b;c", ""] {
            assert!(parse("install_packages", &json!({"names": [bad]}), &r).is_err(), "{bad}");
        }
        assert!(parse("remove_packages", &json!({"names": ["musl"]}), &r).unwrap_err().contains("part of NexaOS"));
        assert!(parse("remove_packages", &json!({"names": ["linux-firmware-intel"]}), &r).is_err());
        assert!(parse("install_app", &json!({"path": "/home/user/Downloads/x.nexa"}), &r).is_ok());
        for p in ["x.nexa", "/etc/shadow", "/home/user/x.nexa\u{0}"] {
            assert!(parse("install_app", &json!({"path": p}), &r).is_err(), "{p}");
        }
        assert!(parse("uninstall_app", &json!({"id": "../../bin/sh"}), &r).is_err());
        assert_eq!(parse("system_update", &json!({}), &r).unwrap().level(), Level::Password);
    }

    #[test]
    fn settings_vpn_drives_people() {
        let r = root();
        assert!(parse("write_ai_config", &json!({"data": {"backend": "local", "allow_control": true}}), &r).is_ok());
        assert!(parse("write_ai_config", &json!({"data": {"backend": "evil"}}), &r).is_err());
        assert!(parse("write_ai_config", &json!({"data": {"shell": "/bin/sh"}}), &r).is_err());
        assert!(parse("vpn_up", &json!({"name": "home-vpn"}), &r).is_ok());
        assert!(parse("vpn_up", &json!({"name": "../wg0"}), &r).is_err());
        assert!(parse("vpn_up", &json!({"name": "a-name-much-too-long"}), &r).is_err());
        for k in ["sda", "sdb1", "mmcblk0", "mmcblk0p2"] {
            assert!(valid_kname(k), "{k}");
        }
        for k in ["nvme0n1", "sda/../x", "loop0", "mmcblk0p", "sd1", "vda"] {
            assert!(!valid_kname(k), "{k}");
        }
        assert!(parse("add_user", &json!({"name": "ama", "password": "longenough"}), &r).is_ok());
        assert!(parse("add_user", &json!({"name": "root", "password": "longenough"}), &r).is_err());
        assert!(parse("add_user", &json!({"name": "ama", "password": "x"}), &r).is_err());
        assert!(parse("add_user", &json!({"name": "ama", "full_name": "A:B", "password": "longenough"}), &r).is_err());
        assert!(parse("change_password", &json!({"new_password": "line\nbreak"}), &r).is_err());
        assert!(!parse("change_password", &json!({"new_password": "secret1"}), &r).unwrap().describe().contains("secret1"));
    }
}
