//! nexa-pack: build, sign and verify NexaOS .nexa packages.
//!
//!   nexa-pack new <id> [--lang c|rust] [--name "Name"] [--dir PATH]
//!   nexa-pack build [DIR] [--key FILE] [-o FILE] [--no-build]
//!   nexa-pack keygen <NAME>                 writes NAME.key (secret) and NAME.pub
//!   nexa-pack verify <FILE> [--keys DIR] [--json]
//!   nexa-pack extract <FILE> <DEST> [--keys DIR] [--json]
//!   nexa-pack repo build <DIR> [--key FILE]   Nexa Store catalogue (index.json)
//!   nexa-pack repo verify <DIR> [--keys DIR] [--json]
//!
//! The install helper runs `extract --json`: it verifies the package and
//! writes the files from the same bytes it checked. Exit status: 0 valid,
//! 1 invalid or failed, 2 usage error.

mod project;
mod templates;

use nexa_pack::{manifest, package, repo, sign};

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::exit;

const DEFAULT_KEYS: &str = "/etc/nexa/keys";
const USAGE: &str = "usage:
  nexa-pack new <id> [--lang c|rust] [--name NAME] [--dir PATH]
  nexa-pack build [DIR] [--key FILE] [-o FILE] [--no-build]
  nexa-pack keygen <NAME>
  nexa-pack verify <FILE> [--keys DIR] [--json]
  nexa-pack extract <FILE> <DEST> [--keys DIR] [--json]
  nexa-pack repo build <DIR> [--key FILE]
  nexa-pack repo verify <DIR> [--keys DIR] [--json]";

struct Args {
    positional: Vec<String>,
    opts: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(raw: &[String]) -> Args {
        let flags_with_value = ["--lang", "--name", "--dir", "--key", "-o", "--keys"];
        let (mut positional, mut opts, mut i) = (Vec::new(), Vec::new(), 0);
        while i < raw.len() {
            let a = &raw[i];
            if flags_with_value.contains(&a.as_str()) {
                opts.push((a.clone(), raw.get(i + 1).cloned()));
                i += 2;
            } else if a.starts_with('-') && a.len() > 1 {
                opts.push((a.clone(), None));
                i += 1;
            } else {
                positional.push(a.clone());
                i += 1;
            }
        }
        Args { positional, opts }
    }
    fn get(&self, name: &str) -> Option<&str> {
        self.opts.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.as_deref())
    }
    fn has(&self, name: &str) -> bool {
        self.opts.iter().any(|(k, _)| k == name)
    }
}

fn die(msg: &str, code: i32) -> ! {
    eprintln!("nexa-pack: {msg}");
    exit(code)
}

fn report_json(v: &package::Verified) -> String {
    let m = &v.manifest;
    serde_json::json!({
        "ok": true,
        "trust": v.trust.label(),
        "publisher": v.trust.publisher(),
        "id": m.id, "name": m.name, "version": m.version,
        "description": m.description, "categories": m.categories, "icon": m.icon,
        "terminal": m.terminal, "depends": m.depends, "permissions": m.permissions, "opens": m.opens,
        "files": v.files.len(),
        "size": v.files.values().map(|d| d.len()).sum::<usize>(),
    })
    .to_string()
}

fn print_report(v: &package::Verified) {
    let m = &v.manifest;
    println!("{} {} ({})", m.name, m.version, m.id);
    match &v.trust {
        package::Trust::Trusted(id) => println!("  signed by trusted publisher {id}"),
        package::Trust::UnknownPublisher(id) => println!("  signed by {id}, which this system doesn't trust"),
        package::Trust::Unsigned => println!("  NOT signed"),
    }
    println!("  {} files, {} bytes; all match the manifest", v.files.len(), v.files.values().map(|d| d.len()).sum::<usize>());
    if !m.depends.is_empty() {
        println!("  needs: {}", m.depends.join(", "));
    }
    if !m.permissions.is_empty() {
        println!("  asks for: {}", m.permissions.join(", "));
    }
}

fn verify_file(path: &str, args: &Args) -> Result<package::Verified, String> {
    let bytes = fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let keys = PathBuf::from(args.get("--keys").unwrap_or(DEFAULT_KEYS));
    package::verify(&bytes, keys.is_dir().then_some(keys.as_path()))
}

fn fail_json(json: bool, msg: &str) -> ! {
    if json {
        println!("{}", serde_json::json!({"ok": false, "error": msg}));
        exit(1)
    }
    die(msg, 1)
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = raw.first().cloned() else { die(USAGE, 2) };
    let args = Args::parse(&raw[1..]);
    match cmd.as_str() {
        "new" => {
            let id = args.positional.first().unwrap_or_else(|| die(USAGE, 2));
            let lang = args.get("--lang").unwrap_or("c");
            let default_name = id.split(['-', '_']).map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }).collect::<Vec<_>>().join(" ");
            let name = args.get("--name").map(String::from).unwrap_or(default_name);
            let dir = PathBuf::from(args.get("--dir").unwrap_or(id));
            templates::create(&dir, id, &name, lang).unwrap_or_else(|e| die(&e, 1));
            println!("Created {} ({lang}). Next: cd {} && nexa-pack build", dir.display(), dir.display());
        }
        "build" => {
            let dir = PathBuf::from(args.positional.first().map(String::as_str).unwrap_or("."));
            let proj = project::load(&dir).unwrap_or_else(|e| die(&e, 1));
            if !args.has("--no-build") {
                project::run_build(&dir, &proj).unwrap_or_else(|e| die(&e, 1));
            }
            let (manifest, files) = project::gather(&dir, &proj).unwrap_or_else(|e| die(&e, 1));
            let key_path = args.get("--key").map(String::from).or_else(|| std::env::var("NEXA_SIGNING_KEY").ok());
            let key = key_path.map(|p| sign::read_secret(Path::new(&p)).unwrap_or_else(|e| die(&e, 1)));
            let bytes = package::write(&manifest, &files, key.as_ref()).unwrap_or_else(|e| die(&e, 1));
            let out = args.get("-o").map(PathBuf::from)
                .unwrap_or_else(|| dir.join("dist").join(format!("{}-{}.nexa", manifest.id, manifest.version)));
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|e| die(&e.to_string(), 1));
            }
            fs::write(&out, &bytes).unwrap_or_else(|e| die(&e.to_string(), 1));
            match &key {
                Some(k) => println!("{} ({} bytes), signed by {}", out.display(), bytes.len(), sign::key_id(&k.verifying_key())),
                None => println!("{} ({} bytes), NOT signed (use --key or NEXA_SIGNING_KEY)", out.display(), bytes.len()),
            }
        }
        "keygen" => {
            let name = args.positional.first().unwrap_or_else(|| die(USAGE, 2));
            let key = sign::generate().unwrap_or_else(|e| die(&e.to_string(), 1));
            let secret = format!("{name}.key");
            let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&secret)
                .unwrap_or_else(|e| die(&format!("{secret}: {e}"), 1));
            f.write_all(sign::secret_text(&key).as_bytes()).unwrap_or_else(|e| die(&e.to_string(), 1));
            fs::write(format!("{name}.pub"), sign::public_text(&key.verifying_key())).unwrap_or_else(|e| die(&e.to_string(), 1));
            let id = sign::key_id(&key.verifying_key());
            println!("key id {id}\n  {secret}   secret: keep it private\n  {name}.pub   public: install as /etc/nexa/keys/{id}.pub to trust it");
        }
        "verify" => {
            let path = args.positional.first().unwrap_or_else(|| die(USAGE, 2));
            let json = args.has("--json");
            match verify_file(path, &args) {
                Ok(v) if json => println!("{}", report_json(&v)),
                Ok(v) => print_report(&v),
                Err(e) => fail_json(json, &e),
            }
        }
        "extract" => {
            let (Some(path), Some(dest)) = (args.positional.first(), args.positional.get(1)) else { die(USAGE, 2) };
            let json = args.has("--json");
            let v = verify_file(path, &args).unwrap_or_else(|e| fail_json(json, &e));
            package::extract(&v, Path::new(dest)).unwrap_or_else(|e| fail_json(json, &e));
            if json { println!("{}", report_json(&v)) } else { print_report(&v) }
        }
        "repo" => {
            let (Some(sub), Some(dir)) = (args.positional.first(), args.positional.get(1)) else { die(USAGE, 2) };
            let keys = PathBuf::from(args.get("--keys").unwrap_or(DEFAULT_KEYS));
            let trust = keys.is_dir().then_some(keys.as_path());
            match sub.as_str() {
                "build" => {
                    let key = args.get("--key").map(|k| sign::read_secret(Path::new(k)).unwrap_or_else(|e| die(&e, 1)));
                    let index = repo::build(Path::new(dir), key.as_ref(), trust).unwrap_or_else(|e| die(&e, 1));
                    println!("{}/index.json: {} apps{}", dir, index.apps.len(),
                             key.map(|k| format!(", signed by {}", sign::key_id(&k.verifying_key()))).unwrap_or_else(|| ", NOT signed".into()));
                }
                "verify" => {
                    let json = args.has("--json");
                    match repo::verify_dir(Path::new(dir), trust) {
                        Ok((index, t)) if json => println!("{}", serde_json::json!({"ok": true, "trust": t.label(), "publisher": t.publisher(), "apps": index.apps.len()})),
                        Ok((index, t)) => println!("{} apps; catalogue {}", index.apps.len(), match &t {
                            package::Trust::Trusted(id) => format!("signed by trusted publisher {id}"),
                            package::Trust::UnknownPublisher(id) => format!("signed by {id}, which this system doesn't trust"),
                            package::Trust::Unsigned => "NOT signed".into(),
                        }),
                        Err(e) => fail_json(json, &e),
                    }
                }
                _ => die(USAGE, 2),
            }
        }
        "-h" | "--help" | "help" => println!("{USAGE}"),
        _ => die(USAGE, 2),
    }
}
