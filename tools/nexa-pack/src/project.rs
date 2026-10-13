//! nexa.toml projects: build them and gather what goes into the package.
//!
//! [package]   id, name, version, description, categories, icon, terminal
//! [build]     command (run with sh -c in the project), binary (-> bin/<id>),
//!             lib (optional folder -> lib/)
//! [requires]  depends (Alpine packages), permissions

use crate::manifest::{Manifest, ABI, ARCH, FORMAT};
use crate::sign;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub package: PackageSection,
    pub build: BuildSection,
    #[serde(default)]
    pub requires: Requires,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSection {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "utility")]
    pub categories: String,
    pub icon: Option<String>,
    #[serde(default)]
    pub terminal: bool,
}

fn utility() -> String {
    "Utility;".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSection {
    pub command: Option<String>,
    pub binary: String,
    pub lib: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

pub fn load(dir: &Path) -> Result<Project, String> {
    let path = dir.join("nexa.toml");
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn run_build(dir: &Path, p: &Project) -> Result<(), String> {
    let Some(cmd) = &p.build.command else { return Ok(()) };
    eprintln!("nexa-pack: building: {cmd}");
    let status = Command::new("sh").arg("-c").arg(cmd).current_dir(dir).status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("build command failed ({status})"));
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.file_type().is_file() {
        return Err(format!("{} is not a regular file (links aren't allowed)", path.display()));
    }
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn add_tree(root: &Path, dir: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir).map_err(|e| e.to_string())?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().to_string();
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            add_tree(root, &path, prefix, out)?;
        } else {
            out.insert(format!("{prefix}{rel}"), read_file(&path)?);
        }
    }
    Ok(())
}

/// The manifest and files for a built project.
pub fn gather(dir: &Path, p: &Project) -> Result<(Manifest, BTreeMap<String, Vec<u8>>), String> {
    let pk = &p.package;
    let mut files = BTreeMap::new();
    files.insert(format!("bin/{}", pk.id), read_file(&dir.join(&p.build.binary))?);
    if let Some(lib) = &p.build.lib {
        let root = dir.join(lib);
        add_tree(&root, &root, "lib/", &mut files)?;
    }
    let icon = match &pk.icon {
        Some(icon) => {
            let ext = Path::new(icon).extension().and_then(|e| e.to_str()).unwrap_or("");
            let name = format!("icon.{ext}");
            files.insert(name.clone(), read_file(&dir.join(icon))?);
            Some(name)
        }
        None => None,
    };
    let manifest = Manifest {
        format: FORMAT,
        id: pk.id.clone(),
        name: pk.name.clone(),
        version: pk.version.clone(),
        description: pk.description.clone(),
        categories: pk.categories.clone(),
        icon,
        arch: ARCH.into(),
        abi: ABI.into(),
        depends: p.requires.depends.clone(),
        permissions: p.requires.permissions.clone(),
        terminal: pk.terminal,
        files: files.iter().map(|(k, v)| (k.clone(), sign::sha256_hex(v))).collect(),
    };
    manifest.validate()?;
    Ok((manifest, files))
}
