//! Installing and removing .nexa apps (format: docs/PACKAGES.md).
//!
//! - The package is read with the *caller's* file permissions (setfsuid),
//!   so asking root to "install" /etc/shadow gets nowhere, then copied to a
//!   private folder: the original is user-writable and could change.
//! - `nexa-pack extract` verifies every file's SHA-256 and the publisher's
//!   signature and writes the files from the very bytes it checked.
//! - An app only ever places files under its own names (bin/<id>,
//!   lib/nexa-<id>, nexa-<id>.desktop, icons/nexa-<id>.*), never over a
//!   system command or anything another app or the system installed, and
//!   records exactly what it placed so removing it is clean.
//! - Its files only ever run later as the unprivileged user; root never
//!   runs anything from the package.

use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const UNVERIFIED: &str = "unverified: ";
const MAX_PACKAGE: u64 = 1024 * 1024 * 1024;
const COMMAND_DIRS: &[&str] = &["/usr/local/sbin", "/usr/local/bin", "/usr/sbin", "/usr/bin", "/sbin", "/bin"];

pub struct Places {
    pub bin: PathBuf,
    pub lib: PathBuf,
    pub apps: PathBuf,
    pub icons: PathBuf,
    pub registry: PathBuf,
    pub keys: PathBuf,
    pub work: PathBuf,
    pub nexa_pack: PathBuf,
    pub command_dirs: Vec<PathBuf>,
    /// The real system: run apk for dependencies and refresh the caches.
    pub system: bool,
}

impl Places {
    /// Everything below `root` (tests use a fake system tree).
    pub fn under(root: &Path) -> Places {
        let r = |p: &str| root.join(p);
        Places {
            bin: r("usr/local/bin"), lib: r("usr/local/lib"), apps: r("usr/share/applications"),
            icons: r("usr/share/icons/hicolor/scalable/apps"), registry: r("var/lib/nexa-packages"),
            keys: r("etc/nexa/keys"), work: r("var/lib/nexa-broker/work"), nexa_pack: r("usr/local/bin/nexa-pack"),
            command_dirs: COMMAND_DIRS.iter().map(|d| root.join(d.trim_start_matches('/'))).collect(),
            system: root == Path::new("/"),
        }
    }

    fn owned(&self, id: &str) -> Vec<PathBuf> {
        vec![self.bin.join(id), self.lib.join(format!("nexa-{id}")), self.apps.join(format!("nexa-{id}.desktop")),
             self.icons.join(format!("nexa-{id}.svg")), self.icons.join(format!("nexa-{id}.png"))]
    }

    fn registry_file(&self, id: &str) -> PathBuf {
        self.registry.join(format!("{id}.json"))
    }

    fn registered(&self, id: &str) -> Option<Vec<PathBuf>> {
        let v: Value = serde_json::from_slice(&fs::read(self.registry_file(id)).ok()?).ok()?;
        Some(v["paths"].as_array()?.iter().filter_map(Value::as_str).map(PathBuf::from).collect())
    }
}

/// Read `path` as user `uid` would be allowed to.
fn read_as(uid: u32, path: &Path) -> Result<Vec<u8>, String> {
    // SAFETY: setfsuid only changes which uid file-permission checks use,
    // for this thread; it is set back right after.
    unsafe { libc::setfsuid(uid) };
    let result = fs::File::open(path).and_then(|f| {
        let mut buf = Vec::new();
        f.take(MAX_PACKAGE + 1).read_to_end(&mut buf).map(|_| buf)
    });
    unsafe { libc::setfsuid(0) };
    let buf = result.map_err(|e| format!("can't read the package: {e}"))?;
    if buf.len() as u64 > MAX_PACKAGE {
        return Err("that package is bigger than 1 GB".into());
    }
    Ok(buf)
}

fn remove(p: &Path) {
    if p.is_dir() && !p.is_symlink() {
        let _ = fs::remove_dir_all(p);
    } else {
        let _ = fs::remove_file(p);
    }
}

/// One line only: a newline in the manifest would add keys to the .desktop file.
fn one_line(v: &Value) -> String {
    v.as_str().map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.starts_with('/') {
        return None;
    }
    let root = fs::canonicalize(root).ok()?;
    let p = fs::canonicalize(root.join(rel)).ok()?;
    (p.starts_with(&root) && p.is_file()).then_some(p)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let ft = e.file_type()?;
        let dest = to.join(e.file_name());
        if ft.is_dir() {
            copy_tree(&e.path(), &dest)?;
        } else if ft.is_file() {
            fs::copy(e.path(), &dest)?;
        } // links and specials: nexa-pack refuses them anyway
    }
    Ok(())
}

fn install_depends(names: &[Value], run_apk: bool) -> Result<(), String> {
    let mut missing = Vec::new();
    for n in names {
        let name = n.as_str().filter(|s| crate::policy::valid_package_name(s)).ok_or_else(|| format!("invalid dependency {n}"))?;
        if !run_apk {
            continue;
        }
        if !Command::new("apk").args(["info", "-e", name]).output().map(|o| o.status.success()).unwrap_or(false) {
            missing.push(name.to_string());
        }
    }
    if !missing.is_empty() {
        let out = Command::new("apk").arg("add").arg("--").args(&missing).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!("couldn't install what the app needs ({}): {}", missing.join(", "),
                               String::from_utf8_lossy(&out.stderr).trim().lines().last().unwrap_or("")));
        }
    }
    Ok(())
}

fn refresh_caches(p: &Places) {
    if !p.system {
        return;
    }
    let _ = Command::new("update-desktop-database").arg(&p.apps).output();
    let _ = Command::new("gtk-update-icon-cache").args(["-f", "-t", "/usr/share/icons/hicolor"]).output();
}

pub fn install(p: &Places, caller_uid: u32, path: &str, allow_unverified: bool, with_depends: bool) -> Result<Value, String> {
    let bytes = read_as(caller_uid, Path::new(path))?;
    fs::create_dir_all(&p.work).map_err(|e| e.to_string())?;
    let work = p.work.join(format!("install-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = install_from(p, &work, &bytes, allow_unverified, with_depends);
    let _ = fs::remove_dir_all(&work);
    result
}

fn install_from(p: &Places, work: &Path, bytes: &[u8], allow_unverified: bool, with_depends: bool) -> Result<Value, String> {
    let pkg = work.join("package.nexa");
    fs::write(&pkg, bytes).map_err(|e| e.to_string())?;
    let payload = work.join("payload");
    let out = Command::new(&p.nexa_pack).arg("extract").arg(&pkg).arg(&payload).arg("--keys").arg(&p.keys).arg("--json")
        .output().map_err(|e| format!("the package verifier isn't installed: {e}"))?;
    let report: Value = String::from_utf8_lossy(&out.stdout).lines().last().and_then(|l| serde_json::from_str(l).ok())
        .ok_or_else(|| String::from_utf8_lossy(&out.stderr).trim().to_string())?;
    if report["ok"] != true {
        return Err(report["error"].as_str().unwrap_or("invalid package").to_string());
    }
    if report["trust"] != "trusted" && !allow_unverified {
        let who = if report["trust"] == "unsigned" { "isn't signed" } else { "is signed by a publisher this system doesn't trust" };
        return Err(format!("{UNVERIFIED}{} {who}. Only install it if you trust where it came from.", one_line(&report["name"])));
    }
    let id = report["id"].as_str().filter(|i| crate::policy::valid_app_id(i)).ok_or("invalid app id")?.to_string();
    let owned = p.owned(&id);
    let previous = p.registered(&id);
    if previous.is_none() {
        if let Some(clash) = owned.iter().find(|x| x.symlink_metadata().is_ok()) {
            return Err(format!("'{id}' conflicts with something already on the system ({})", clash.display()));
        }
        if p.command_dirs.iter().any(|d| d.join(&id).symlink_metadata().is_ok()) {
            return Err(format!("'{id}' is the name of an existing system command"));
        }
    }
    install_depends(report["depends"].as_array().map(Vec::as_slice).unwrap_or(&[]), with_depends && p.system)?;
    // Upgrading: clear out what the previous version placed (only its own names).
    for x in previous.unwrap_or_default() {
        if owned.contains(&x) {
            remove(&x);
        }
    }
    let attempt = (|| -> Result<Vec<PathBuf>, String> {
        let mut placed = Vec::new();
        let bin_src = inside(&payload, &format!("bin/{id}")).ok_or(format!("the package is missing its program (bin/{id})"))?;
        fs::create_dir_all(&p.bin).map_err(|e| e.to_string())?;
        fs::copy(&bin_src, &owned[0]).map_err(|e| e.to_string())?;
        fs::set_permissions(&owned[0], std::os::unix::fs::PermissionsExt::from_mode(0o755)).map_err(|e| e.to_string())?;
        placed.push(owned[0].clone());
        let lib = payload.join("lib");
        if lib.is_dir() && !lib.is_symlink() {
            copy_tree(&lib, &owned[1]).map_err(|e| e.to_string())?;
            placed.push(owned[1].clone());
        }
        if let Some(icon) = report["icon"].as_str().and_then(|i| inside(&payload, i)) {
            let png = icon.extension().map_or(false, |e| e.eq_ignore_ascii_case("png"));
            let dest = if png { owned[4].clone() } else { owned[3].clone() };
            fs::create_dir_all(&p.icons).map_err(|e| e.to_string())?;
            fs::copy(&icon, &dest).map_err(|e| e.to_string())?;
            placed.push(dest);
        }
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={}\nIcon=nexa-{id}\nTerminal={}\nCategories={}\n",
            one_line(&report["name"]), one_line(&report["description"]), owned[0].display(),
            if report["terminal"] == true { "true" } else { "false" },
            Some(one_line(&report["categories"])).filter(|c| !c.is_empty()).unwrap_or_else(|| "Utility;".into()));
        fs::create_dir_all(&p.apps).map_err(|e| e.to_string())?;
        fs::write(&owned[2], desktop).map_err(|e| e.to_string())?;
        placed.push(owned[2].clone());
        fs::create_dir_all(&p.registry).map_err(|e| e.to_string())?;
        fs::write(p.registry_file(&id), json!({
            "id": id, "version": report["version"], "paths": placed, "trust": report["trust"],
            "publisher": report["publisher"], "permissions": report["permissions"],
        }).to_string()).map_err(|e| e.to_string())?;
        Ok(placed)
    })();
    if let Err(e) = attempt {
        // Never leave unregistered files behind: they'd block this id later.
        for x in &owned {
            remove(x);
        }
        let _ = fs::remove_file(p.registry_file(&id));
        return Err(e);
    }
    refresh_caches(p);
    Ok(json!({"done": format!("Installed {} {}", one_line(&report["name"]), one_line(&report["version"])),
              "id": id, "trust": report["trust"], "undo": {"action": "uninstall_app", "args": {"id": id}}}))
}

pub fn uninstall(p: &Places, id: &str) -> Result<Value, String> {
    let paths = p.registered(id).ok_or_else(|| format!("'{id}' isn't an installed app"))?;
    let owned = p.owned(id);
    for x in paths.iter().filter(|x| owned.contains(x)) {
        remove(x);
    }
    let _ = fs::remove_file(p.registry_file(id));
    refresh_caches(p);
    Ok(json!({"done": format!("Removed {id}")}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_values_stay_on_one_line() {
        assert_eq!(one_line(&json!("Video\nExec=/bin/sh  Player")), "Video Exec=/bin/sh Player");
    }

    #[test]
    fn package_paths_stay_inside() {
        let d = std::env::temp_dir().join(format!("nexa-broker-apps-{}", std::process::id()));
        fs::create_dir_all(d.join("p/bin")).unwrap();
        fs::write(d.join("p/bin/x"), "x").unwrap();
        assert!(inside(&d.join("p"), "bin/x").is_some());
        assert!(inside(&d.join("p"), "../p/bin/x").is_some(), "resolves back inside");
        assert!(inside(&d.join("p"), "/etc/passwd").is_none());
        assert!(inside(&d.join("p"), "../../../etc/passwd").is_none());
    }
}
