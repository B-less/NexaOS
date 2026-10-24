//! `nexa-doctor hardware`: what this computer is made of and which parts
//! have no driver - the report to send when the touchpad, Wi-Fi or sound
//! doesn't work on a laptop NexaOS hasn't met before.
//!
//! Everything comes from sysfs and procfs (read only): the maker and model
//! (DMI), the processor, PCI / USB / SD-bus devices with their IDs and
//! drivers, ACPI devices the firmware lists that nothing drives (touchpads
//! on the I2C bus show up here), input devices, and the kernel's firmware
//! errors when the kernel log is readable.

use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

fn read(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn driver(dev: &Path) -> Option<String> {
    fs::read_link(dev.join("driver")).ok()?.file_name().map(|n| n.to_string_lossy().into_owned())
}

fn hex(s: Option<String>) -> String {
    s.unwrap_or_default().trim_start_matches("0x").to_string()
}

/// What a PCI class is, in words (the parts people ask about).
fn pci_kind(class: &str) -> &'static str {
    let c = class.trim_start_matches("0x");
    match c.get(..4).unwrap_or("") {
        "0280" => "Wi-Fi",
        "0200" => "Ethernet",
        "0403" => "Sound",
        "0401" => "Sound",
        "0c03" => "USB controller",
        "0c05" => "SMBus",
        "0c80" | "0c81" => "Serial bus (I2C/SPI: touchpad, sensors)",
        "1180" => "Signal processing (thermal, I2C/SPI)",
        "0d11" => "Bluetooth",
        "0805" => "SD card reader",
        "0108" => "NVMe disk",
        "0106" => "SATA disk",
        _ => match c.get(..2).unwrap_or("") {
            "03" => "Display",
            "04" => "Multimedia",
            "02" => "Network",
            "06" => "Bridge",
            "07" => "Communication (modem, MEI)",
            "08" => "System peripheral",
            "09" => "Input",
            "0d" => "Wireless",
            "10" => "Encryption",
            "11" => "Signal processing",
            _ => "Other",
        },
    }
}

fn pci(sys: &Path) -> Vec<Value> {
    let mut v: Vec<Value> = fs::read_dir(sys.join("bus/pci/devices")).into_iter().flatten().flatten().map(|e| {
        let d = e.path();
        let class = hex(read(&d.join("class")));
        json!({
            "slot": e.file_name().to_string_lossy(),
            "id": format!("{}:{}", hex(read(&d.join("vendor"))), hex(read(&d.join("device")))),
            "class": class, "kind": pci_kind(&class), "driver": driver(&d),
        })
    }).collect();
    v.sort_by(|a, b| a["slot"].as_str().cmp(&b["slot"].as_str()));
    v
}

fn usb(sys: &Path) -> Vec<Value> {
    let base = sys.join("bus/usb/devices");
    let mut v: Vec<Value> = fs::read_dir(&base).into_iter().flatten().flatten().filter_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        // Devices only ("1-2"), not interfaces ("1-2:1.0") or root hubs ("usb1").
        if name.contains(':') || name.starts_with("usb") {
            return None;
        }
        let d = e.path();
        let interfaces: Vec<Value> = fs::read_dir(&d).into_iter().flatten().flatten()
            .filter(|i| i.file_name().to_string_lossy().starts_with(&format!("{name}:")))
            .map(|i| json!({"class": read(&i.path().join("bInterfaceClass")), "driver": driver(&i.path())}))
            .collect();
        let label = [read(&d.join("manufacturer")), read(&d.join("product"))].into_iter().flatten().collect::<Vec<_>>().join(" ");
        Some(json!({
            "port": name,
            "id": format!("{}:{}", read(&d.join("idVendor")).unwrap_or_default(), read(&d.join("idProduct")).unwrap_or_default()),
            "name": label,
            "interfaces": interfaces,
        }))
    }).collect();
    v.sort_by(|a, b| a["port"].as_str().cmp(&b["port"].as_str()));
    v
}

fn sdio(sys: &Path) -> Vec<Value> {
    fs::read_dir(sys.join("bus/sdio/devices")).into_iter().flatten().flatten().map(|e| {
        let d = e.path();
        json!({"slot": e.file_name().to_string_lossy(),
               "id": format!("{}:{}", hex(read(&d.join("vendor"))), hex(read(&d.join("device")))),
               "class": read(&d.join("class")), "driver": driver(&d)})
    }).collect()
}

/// ACPI IDs every PC has, which never get a driver of their own.
fn generic_acpi(hid: &str) -> bool {
    hid.starts_with("PNP0") || hid.starts_with("LNX") || hid.starts_with("ACPI0") || hid.starts_with("device")
        || hid.starts_with("INT0800") || hid.starts_with("FIX")
}

/// Devices the firmware lists as present that nothing drives. An I2C
/// touchpad's own ID is vendor-specific (ELAN0001, SYNA7DB5, ...) but it
/// says it is compatible with PNP0C50 / ACPI0C50 (HID over I2C).
fn acpi(sys: &Path) -> (Vec<Value>, Vec<Value>) {
    let mut unbound = Vec::new();
    let mut touchpads = Vec::new();
    for e in fs::read_dir(sys.join("bus/acpi/devices")).into_iter().flatten().flatten() {
        let d = e.path();
        let Some(hid) = read(&d.join("hid")) else { continue };
        let status: u32 = read(&d.join("status")).and_then(|s| s.parse().ok()).unwrap_or(15);
        if status & 1 == 0 {
            continue; // not present
        }
        let modalias = read(&d.join("modalias")).unwrap_or_default();
        let node = d.join("physical_node");
        let drv = driver(&d).or_else(|| driver(&node));
        if modalias.contains("PNP0C50") || modalias.contains("ACPI0C50") {
            touchpads.push(json!({"acpi": e.file_name().to_string_lossy(), "hid": hid, "driver": drv}));
        }
        if drv.is_none() && !generic_acpi(&hid) {
            unbound.push(json!({"acpi": e.file_name().to_string_lossy(), "hid": hid, "name": read(&d.join("description"))}));
        }
    }
    unbound.sort_by(|a, b| a["acpi"].as_str().cmp(&b["acpi"].as_str()));
    (unbound, touchpads)
}

/// Input devices from /proc/bus/input/devices: names, and which look like
/// a touchpad or a mouse.
fn inputs(proc_root: &Path) -> Vec<Value> {
    let text = fs::read_to_string(proc_root.join("bus/input/devices")).unwrap_or_default();
    text.split("\n\n").filter_map(|block| {
        let name = block.lines().find_map(|l| l.strip_prefix("N: Name="))?.trim_matches('"').to_string();
        let handlers = block.lines().find_map(|l| l.strip_prefix("H: Handlers=")).unwrap_or("").trim().to_string();
        let lower = name.to_lowercase();
        let kind = if lower.contains("touchpad") || lower.contains("trackpad") || lower.contains("glidepoint") {
            "touchpad"
        } else if handlers.contains("mouse") {
            "mouse"
        } else if handlers.contains("kbd") && lower.contains("keyboard") {
            "keyboard"
        } else {
            "other"
        };
        Some(json!({"name": name, "kind": kind}))
    }).collect()
}

/// The kernel's firmware complaints, if the log is readable (it may not
/// be for a normal user). Non-blocking: reads what is there and stops.
fn firmware_errors(kmsg: &Path) -> Option<Vec<String>> {
    let mut f = fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(kmsg).ok()?;
    let mut out = Vec::new();
    let mut buf = vec![0u8; 8192];
    // Each read returns one record; EAGAIN once the end is reached.
    for _ in 0..20000 {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let rec = String::from_utf8_lossy(&buf[..n]);
                let msg = rec.split_once(';').map(|(_, m)| m).unwrap_or(&rec).lines().next().unwrap_or("").to_string();
                let l = msg.to_lowercase();
                // Firmware problems, and Wi-Fi drivers' own errors (they often
                // don't say "firmware": "iwlwifi ...: Failed to start RT ucode").
                let wifi_driver = ["iwlwifi", "rtw88", "rtw89", "rtl8", "mt76", "mt79", "ath9k", "ath10k", "ath11k", "ath12k", "brcmf", "mwifiex"]
                    .iter().any(|d| l.contains(d));
                let bad = l.contains("fail") || l.contains("error") || l.contains("-2") || l.contains("not found") || l.contains("timeout");
                if (l.contains("firmware") || wifi_driver) && bad {
                    out.push(msg);
                }
            }
            Err(e) if e.raw_os_error() == Some(libc::EPIPE) => continue, // a record was overwritten
            Err(_) => break,
        }
    }
    out.dedup();
    Some(out)
}

/// Wi-Fi beyond the card itself: the radio's on/off switches (rfkill: the
/// keyboard's Wi-Fi key or a switch is "hard", settings are "soft"), the
/// network interfaces and their drivers, and whether iwd (which runs Wi-Fi)
/// is running.
fn wifi(sys: &Path, proc_root: &Path) -> Value {
    let radios: Vec<Value> = fs::read_dir(sys.join("class/rfkill")).into_iter().flatten().flatten().filter_map(|e| {
        let d = e.path();
        let kind = read(&d.join("type"))?;
        Some(json!({"name": read(&d.join("name")), "type": kind,
                    "soft": read(&d.join("soft")).as_deref() == Some("1"), "hard": read(&d.join("hard")).as_deref() == Some("1")}))
    }).collect();
    let interfaces: Vec<Value> = fs::read_dir(sys.join("class/net")).into_iter().flatten().flatten().filter(|e| {
        e.path().join("wireless").exists() || e.path().join("phy80211").exists()
    }).map(|e| {
        let d = e.path();
        json!({"name": e.file_name().to_string_lossy(), "state": read(&d.join("operstate")), "driver": driver(&d.join("device"))})
    }).collect();
    let iwd = fs::read_dir(proc_root).into_iter().flatten().flatten()
        .any(|e| read(&e.path().join("comm")).as_deref() == Some("iwd"));
    json!({"radios": radios, "interfaces": interfaces, "iwd": iwd})
}

fn cpu(proc_root: &Path) -> Option<String> {
    fs::read_to_string(proc_root.join("cpuinfo")).ok()?.lines()
        .find_map(|l| l.strip_prefix("model name").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim().to_string()))
}

pub fn report(sys: &Path, proc_root: &Path, kmsg: &Path) -> Value {
    let dmi = sys.join("class/dmi/id");
    let pci = pci(sys);
    let usb = usb(sys);
    let sdio = sdio(sys);
    let (acpi_unbound, i2c_touchpads) = acpi(sys);
    let inputs = inputs(proc_root);

    let mut problems: Vec<String> = Vec::new();
    for d in &pci {
        let class = d["class"].as_str().unwrap_or("");
        if d["driver"].is_null() && !class.starts_with("06") && !class.starts_with("08") && !class.starts_with("11") {
            problems.push(format!("No driver: {} [PCI {}]", d["kind"].as_str().unwrap_or(""), d["id"].as_str().unwrap_or("")));
        }
    }
    for d in &usb {
        let unbound = d["interfaces"].as_array().map_or(false, |i| i.iter().any(|i| i["driver"].is_null()));
        if unbound {
            problems.push(format!("No driver: {} [USB {}]", d["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("USB device"), d["id"].as_str().unwrap_or("")));
        }
    }
    for d in &sdio {
        if d["driver"].is_null() {
            problems.push(format!("No driver: SD-bus device (often Wi-Fi) [SDIO {}]", d["id"].as_str().unwrap_or("")));
        }
    }
    for t in &i2c_touchpads {
        if t["driver"].is_null() {
            problems.push(format!("The touchpad {} (I2C) has no driver: its I2C bus or GPIO controller isn't supported", t["hid"].as_str().unwrap_or("")));
        }
    }
    if !inputs.iter().any(|i| i["kind"] == "touchpad" || i["kind"] == "mouse") {
        problems.push("No touchpad or mouse found by the kernel".into());
    }
    if !pci.iter().any(|d| d["kind"] == "Wi-Fi" && !d["driver"].is_null())
        && !sdio.iter().any(|d| !d["driver"].is_null())
        && !fs::read_dir(sys.join("class/ieee80211")).map_or(false, |mut r| r.next().is_some())
    {
        problems.push("No working Wi-Fi adapter".into());
    }
    let wifi = wifi(sys, proc_root);
    let wlan = |key: &str| wifi["radios"].as_array().into_iter().flatten().any(|r| r["type"] == "wlan" && r[key] == true);
    if wlan("hard") {
        problems.push("Wi-Fi is switched off by the keyboard's Wi-Fi key or a switch (press it; on some laptops it's Fn + a key with an aeroplane or antenna)".into());
    } else if wlan("soft") {
        problems.push("Wi-Fi is switched off in software (Settings, or aeroplane mode)".into());
    }
    for d in &pci {
        let slot = d["slot"].as_str().unwrap_or("");
        let dev = sys.join("bus/pci/devices").join(slot);
        if d["kind"] == "Wi-Fi" && !d["driver"].is_null() && !dev.join("net").exists() && !dev.join("ieee80211").exists() {
            problems.push(format!("The Wi-Fi card has its driver ({}) but no network interface: its firmware didn't load (see the errors below)",
                                  d["driver"].as_str().unwrap_or("")));
        }
    }
    if wifi["interfaces"].as_array().map_or(false, |i| !i.is_empty()) && wifi["iwd"] == false {
        problems.push("The Wi-Fi service (iwd) isn't running".into());
    }

    json!({
        "computer": {
            "maker": read(&dmi.join("sys_vendor")), "model": read(&dmi.join("product_name")),
            "board": read(&dmi.join("board_name")), "bios": read(&dmi.join("bios_version")),
            "processor": cpu(proc_root),
        },
        "problems": problems,
        "pci": pci, "usb": usb, "sdio": sdio,
        "acpi_without_driver": acpi_unbound, "i2c_touchpads": i2c_touchpads,
        "input": inputs, "wifi": wifi,
        "firmware_errors": firmware_errors(kmsg),
    })
}

/// The report as text, short enough to photograph.
pub fn text(r: &Value) -> String {
    let s = |v: &Value| v.as_str().unwrap_or("?").to_string();
    let c = &r["computer"];
    let mut t = format!("NexaOS hardware report\n{} {} (board {}, BIOS {})\n{}\n\n", s(&c["maker"]), s(&c["model"]), s(&c["board"]), s(&c["bios"]), s(&c["processor"]));
    let problems = r["problems"].as_array().cloned().unwrap_or_default();
    t += if problems.is_empty() { "No problems found.\n" } else { "Problems:\n" };
    for p in &problems {
        t += &format!("  ! {}\n", s(p));
    }
    t += "\nPCI:\n";
    for d in r["pci"].as_array().into_iter().flatten() {
        if d["class"].as_str().map_or(false, |c| c.starts_with("06")) {
            continue;
        }
        t += &format!("  {} {}  {}  -> {}\n", s(&d["slot"]), s(&d["id"]), s(&d["kind"]), d["driver"].as_str().unwrap_or("NO DRIVER"));
    }
    t += "USB:\n";
    for d in r["usb"].as_array().into_iter().flatten() {
        let drivers: Vec<String> = d["interfaces"].as_array().into_iter().flatten().map(|i| i["driver"].as_str().unwrap_or("NO DRIVER").to_string()).collect();
        t += &format!("  {} {}  -> {}\n", s(&d["id"]), s(&d["name"]), drivers.join(", "));
    }
    for d in r["sdio"].as_array().into_iter().flatten() {
        t += &format!("SDIO: {} class {} -> {}\n", s(&d["id"]), s(&d["class"]), d["driver"].as_str().unwrap_or("NO DRIVER"));
    }
    for d in r["i2c_touchpads"].as_array().into_iter().flatten() {
        t += &format!("I2C touchpad: {} -> {}\n", s(&d["hid"]), d["driver"].as_str().unwrap_or("NO DRIVER"));
    }
    let acpi: Vec<String> = r["acpi_without_driver"].as_array().into_iter().flatten().map(|d| s(&d["hid"])).collect();
    if !acpi.is_empty() {
        t += &format!("ACPI without a driver: {}\n", acpi.join(" "));
    }
    let input: Vec<String> = r["input"].as_array().into_iter().flatten().filter(|i| i["kind"] != "other").map(|i| format!("{} ({})", s(&i["name"]), s(&i["kind"]))).collect();
    t += &format!("Input: {}\n", if input.is_empty() { "none".to_string() } else { input.join("; ") });
    let w = &r["wifi"];
    let radios: Vec<String> = w["radios"].as_array().into_iter().flatten().filter(|x| x["type"] == "wlan")
        .map(|x| format!("{} {}", s(&x["name"]), if x["hard"] == true { "OFF (key/switch)" } else if x["soft"] == true { "off (software)" } else { "on" })).collect();
    let ifaces: Vec<String> = w["interfaces"].as_array().into_iter().flatten()
        .map(|x| format!("{} {} ({})", s(&x["name"]), s(&x["state"]), x["driver"].as_str().unwrap_or("?"))).collect();
    t += &format!("Wi-Fi: radio {}; interface {}; iwd {}\n",
                  if radios.is_empty() { "none".into() } else { radios.join(", ") },
                  if ifaces.is_empty() { "none".into() } else { ifaces.join(", ") },
                  if w["iwd"] == true { "running" } else { "NOT running" });
    match r["firmware_errors"].as_array() {
        Some(e) if !e.is_empty() => {
            t += "Firmware / Wi-Fi errors:\n";
            for l in e.iter().take(15) {
                t += &format!("  {}\n", s(l));
            }
        }
        Some(_) => t += "Firmware / Wi-Fi errors: none\n",
        None => t += "Firmware / Wi-Fi errors: kernel log not readable (the system service isn't running)\n",
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn put(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn bind(root: &Path, dev: &str, drv: &str) {
        fs::create_dir_all(root.join("drivers").join(drv)).unwrap();
        symlink(root.join("drivers").join(drv), root.join(dev).join("driver")).unwrap();
    }

    #[test]
    fn a_budget_laptop_with_a_dead_touchpad_and_wifi() {
        let r = std::env::temp_dir().join(format!("nexa-doctor-hw-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let sys = r.join("sys");
        put(&sys, "class/dmi/id/sys_vendor", "KA Tech\n");
        put(&sys, "class/dmi/id/product_name", "TM1\n");
        // Intel graphics with its driver; a Realtek Wi-Fi card without one.
        put(&sys, "bus/pci/devices/0000:00:02.0/class", "0x030000\n");
        put(&sys, "bus/pci/devices/0000:00:02.0/vendor", "0x8086\n");
        put(&sys, "bus/pci/devices/0000:00:02.0/device", "0x3185\n");
        bind(&sys, "bus/pci/devices/0000:00:02.0", "i915");
        put(&sys, "bus/pci/devices/0000:01:00.0/class", "0x028000\n");
        put(&sys, "bus/pci/devices/0000:01:00.0/vendor", "0x10ec\n");
        put(&sys, "bus/pci/devices/0000:01:00.0/device", "0xc821\n");
        // A host bridge without a driver is normal.
        put(&sys, "bus/pci/devices/0000:00:00.0/class", "0x060000\n");
        // A USB keyboard (bound) and an unbound vendor interface.
        put(&sys, "bus/usb/devices/1-2/idVendor", "0bda\n");
        put(&sys, "bus/usb/devices/1-2/idProduct", "b711\n");
        put(&sys, "bus/usb/devices/1-2/product", "802.11n WLAN Adapter\n");
        put(&sys, "bus/usb/devices/1-2/1-2:1.0/bInterfaceClass", "ff\n");
        // An I2C-HID touchpad nothing drives (no GPIO controller for it).
        put(&sys, "bus/acpi/devices/ELAN0001:00/hid", "ELAN0001\n");
        put(&sys, "bus/acpi/devices/ELAN0001:00/status", "15\n");
        put(&sys, "bus/acpi/devices/ELAN0001:00/modalias", "acpi:ELAN0001:PNP0C50:\n");
        put(&sys, "bus/acpi/devices/INT3453:00/hid", "INT3453\n");
        put(&sys, "bus/acpi/devices/PNP0A08:00/hid", "PNP0A08\n");
        put(&sys, "bus/acpi/devices/INT9999:00/hid", "INT9999\n");
        put(&sys, "bus/acpi/devices/INT9999:00/status", "0\n");
        let proc_root = r.join("proc");
        put(&proc_root, "cpuinfo", "processor\t: 0\nmodel name\t: Intel(R) Celeron(R) N4020 CPU @ 1.10GHz\n");
        put(&proc_root, "bus/input/devices", "I: Bus=0011\nN: Name=\"AT Translated Set 2 keyboard\"\nH: Handlers=sysrq kbd event0\n\nI: Bus=0019\nN: Name=\"Power Button\"\nH: Handlers=kbd event1\n");

        put(&sys, "class/rfkill/rfkill0/type", "wlan\n");
        put(&sys, "class/rfkill/rfkill0/name", "phy0\n");
        put(&sys, "class/rfkill/rfkill0/soft", "0\n");
        put(&sys, "class/rfkill/rfkill0/hard", "1\n");
        let v = report(&sys, &proc_root, &r.join("no-kmsg"));
        assert!(v["problems"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().contains("keyboard's Wi-Fi key")), "{}", v["problems"]);
        assert!(text(&v).contains("Wi-Fi: radio phy0 OFF (key/switch); interface none; iwd NOT running"), "{}", text(&v));
        put(&sys, "class/rfkill/rfkill0/hard", "0\n");
        let v = report(&sys, &proc_root, &r.join("no-kmsg"));
        assert_eq!(v["computer"]["model"], "TM1");
        assert!(v["computer"]["processor"].as_str().unwrap().contains("N4020"));
        let problems: Vec<&str> = v["problems"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
        assert!(problems.iter().any(|p| p.contains("Wi-Fi [PCI 10ec:c821]")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("[USB 0bda:b711]")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("ELAN0001")), "{problems:?}");
        assert!(problems.contains(&"No touchpad or mouse found by the kernel"));
        assert!(problems.contains(&"No working Wi-Fi adapter"));
        assert!(!problems.iter().any(|p| p.contains("Bridge") || p.contains("Display")), "{problems:?}");
        let acpi: Vec<&str> = v["acpi_without_driver"].as_array().unwrap().iter().map(|a| a["hid"].as_str().unwrap()).collect();
        assert_eq!(acpi, vec!["ELAN0001", "INT3453"], "generic and absent devices are left out");
        assert!(v["firmware_errors"].is_null());
        let t = text(&v);
        assert!(t.contains("KA Tech TM1") && t.contains("10ec:c821  Wi-Fi  -> NO DRIVER") && t.contains("kernel log not readable"), "{t}");
        assert!(!t.contains("0000:00:00.0"), "bridges left out of the text");

        // The driver attached but no interface: the firmware didn't load.
        bind(&sys, "bus/pci/devices/0000:01:00.0", "rtw88_8821ce");
        let v = report(&sys, &proc_root, &r.join("no-kmsg"));
        assert!(v["problems"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().contains("rtw88_8821ce) but no network interface")), "{}", v["problems"]);
        // With an interface, a touchpad and iwd: those problems go.
        fs::create_dir_all(sys.join("bus/pci/devices/0000:01:00.0/net/wlan0")).unwrap();
        put(&sys, "class/net/wlan0/operstate", "up\n");
        fs::create_dir_all(sys.join("class/net/wlan0/wireless")).unwrap();
        put(&proc_root, "42/comm", "iwd\n");
        fs::create_dir_all(sys.join("class/ieee80211/phy0")).unwrap();
        bind(&sys, "bus/acpi/devices/ELAN0001:00", "i2c_hid_acpi");
        put(&proc_root, "bus/input/devices", "N: Name=\"ELAN0001:00 04F3:3140 Touchpad\"\nH: Handlers=mouse0 event5\n");
        let v = report(&sys, &proc_root, &r.join("no-kmsg"));
        let problems: Vec<&str> = v["problems"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
        assert!(!problems.iter().any(|p| p.contains("Wi-Fi") || p.contains("touchpad") || p.contains("ELAN") || p.contains("iwd")), "{problems:?}");
        assert!(text(&v).contains("interface wlan0 up"), "{}", text(&v));
        assert_eq!(v["input"][0]["kind"], "touchpad");
    }
}
