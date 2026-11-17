//! nexac: the NexaLang compiler.
//!
//!   nexac run   app.nx [args...]   build and run
//!   nexac run --check-memory app.nx  also report memory never freed
//!   nexac build app.nx [-o out]    build a native program
//!   nexac check app.nx             only check for mistakes
//!   nexac emit-c app.nx            print the generated C

mod ast;
mod checker;
mod codegen;
mod diag;
mod lexer;
mod parser;

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The hardened flags WRITING_NEXA_APPS.md requires for C. Never remove any.
const HARDENED: &[&str] = &[
    "-std=gnu11",
    "-O2",
    "-Wall",
    "-Wextra",
    "-Werror",
    "-fstack-protector-strong",
    "-D_FORTIFY_SOURCE=2",
    "-fPIE",
    "-pie",
    "-Wl,-z,relro,-z,now",
];

fn usage() -> ! {
    eprintln!(
        "nexac {VERSION}: the NexaLang compiler\n\n\
         usage:\n  \
         nexac run   FILE.nx [ARGS...]   build and run\n  \
         nexac run --check-memory FILE.nx   also report memory never freed\n  \
         nexac build FILE.nx [-o OUT]    build a program\n  \
         nexac check FILE.nx             check for mistakes only\n  \
         nexac emit-c FILE.nx            print the generated C\n  \
         nexac --version"
    );
    exit(2)
}

/// Standard modules, written in NexaLang and built into the compiler.
const STD: &[(&str, &str)] = &[
    ("files", include_str!("std/files.nx")),
    ("time", include_str!("std/time.nx")),
    ("json", include_str!("std/json.nx")),
    ("broker", include_str!("std/broker.nx")),
];

const MAX_LINES: usize = 999_999;

fn render_all(ds: &[diag::Diag], mods: &[ast::Module]) -> String {
    let mut s: String = ds
        .iter()
        .map(|d| {
            let m = &mods[d.file.unwrap_or(0)];
            d.render(&m.file, &m.src)
        })
        .collect::<Vec<_>>()
        .join("\n");
    if ds.len() > 1 {
        s += &format!("\n{} errors\n", ds.len());
    }
    s
}

fn parse_module(name: &str, file: String, src: String, is_std: bool) -> Result<ast::Module, String> {
    if src.lines().count() > MAX_LINES {
        return Err(format!("error: {} has more than {} lines; split it into modules\n", file, MAX_LINES));
    }
    let toks = lexer::lex(&src).map_err(|d| d.render(&file, &src))?;
    let prog = parser::parse(toks).map_err(|d| d.render(&file, &src))?;
    Ok(ast::Module { name: name.to_string(), file, src, is_std, prog })
}

/// Load the main file and everything it uses, then check and generate C.
pub fn compile_to_c(main_path: &Path) -> Result<String, String> {
    let main_src = read_text(main_path)?;
    let display = display_name(main_path);
    let main_dir = main_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let main_canon = std::fs::canonicalize(main_path).ok();
    let mut mods = vec![parse_module("", display, main_src, false)?];
    // where each user module lives, to find the modules it uses
    let mut dirs: Vec<PathBuf> = vec![main_dir.clone()];
    let mut i = 0;
    while i < mods.len() {
        let uses = mods[i].prog.uses.clone();
        for (name, span) in uses {
            if mods.iter().any(|m| m.name == name) {
                continue;
            }
            let fail = |msg: String, hint: Option<String>| -> String {
                let mut d = diag::Diag::new(span, msg);
                if let Some(h) = hint {
                    d = d.hint(h);
                }
                d.render(&mods[i].file, &mods[i].src)
            };
            let local = dirs[i].join(format!("{}.nx", name));
            if let Some((_, src)) = STD.iter().find(|(n, _)| *n == name) {
                if !mods[i].is_std && local.exists() {
                    return Err(fail(
                        format!("`{}` is a standard module, but there's also a {}.nx here", name, name),
                        Some(format!("rename your {}.nx so it's clear which one you mean", name)),
                    ));
                }
                mods.push(parse_module(&name, format!("std/{}.nx", name), src.to_string(), true)?);
                dirs.push(PathBuf::new());
                continue;
            }
            if mods[i].is_std {
                return Err(fail(format!("unknown standard module `{}`", name), None));
            }
            if !local.exists() {
                let std_names: Vec<&str> = STD.iter().map(|(n, _)| *n).collect();
                return Err(fail(
                    format!("can't find module `{}`: there's no {} next to this file", name, local.display()),
                    Some(format!("the standard modules are: {}", std_names.join(", "))),
                ));
            }
            if main_canon.is_some() && std::fs::canonicalize(&local).ok() == main_canon {
                return Err(fail("a module can't use the main program".into(), None));
            }
            let src = read_text(&local)?;
            let shown = local.strip_prefix(&main_dir).unwrap_or(&local).display().to_string();
            mods.push(parse_module(&name, shown, src, false)?);
            dirs.push(local.parent().map(Path::to_path_buf).unwrap_or_default());
        }
        i += 1;
    }
    check_cycles(&mods)?;
    let checked = checker::check(&mut mods).map_err(|ds| render_all(&ds, &mods))?;
    Ok(codegen::generate(&mods, &checked))
}

/// Modules can't use each other in a circle.
fn check_cycles(mods: &[ast::Module]) -> Result<(), String> {
    fn visit(i: usize, mods: &[ast::Module], state: &mut Vec<u8>, path: &mut Vec<usize>) -> Option<(usize, diag::Span, Vec<usize>)> {
        state[i] = 1;
        path.push(i);
        for (u, span) in &mods[i].prog.uses {
            let Some(j) = mods.iter().position(|m| m.name == *u) else { continue };
            if state[j] == 1 {
                let start = path.iter().position(|&p| p == j).unwrap_or(0);
                return Some((i, *span, path[start..].to_vec()));
            }
            if state[j] == 0 {
                if let Some(r) = visit(j, mods, state, path) {
                    return Some(r);
                }
            }
        }
        path.pop();
        state[i] = 2;
        None
    }
    let mut state = vec![0u8; mods.len()];
    if let Some((i, span, cycle)) = visit(0, mods, &mut state, &mut Vec::new()) {
        let names: Vec<String> = cycle
            .iter()
            .chain(std::iter::once(&cycle[0]))
            .map(|&k| if mods[k].name.is_empty() { mods[k].file.clone() } else { mods[k].name.clone() })
            .collect();
        let d = diag::Diag::new(span, format!("modules use each other in a circle: {}", names.join(" -> ")))
            .hint("move what they share into a third module that both use");
        return Err(d.render(&mods[i].file, &mods[i].src));
    }
    Ok(())
}

fn read_text(path: &Path) -> Result<String, String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| format!("error: {} isn't valid UTF-8 text\n", path.display())),
        Err(e) => Err(format!("error: can't read {}: {}\n", path.display(), e)),
    }
}

fn display_name(path: &Path) -> String {
    path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn to_c_or_exit(path: &str) -> String {
    if !path.ends_with(".nx") {
        eprintln!("error: NexaLang files end in .nx: {}", path);
        exit(2);
    }
    match compile_to_c(Path::new(path)) {
        Ok(c) => c,
        Err(e) => {
            eprint!("{}", e);
            exit(1);
        }
    }
}

/// Compile C to `out` with the hardened flags. Uses $CC or `cc`; starts
/// the compiler with an argument list, never through a shell.
fn build_c(c: &str, out: &Path, check_memory: bool) {
    let c_path = out.with_extension("nexa.c");
    if let Err(e) = std::fs::write(&c_path, c) {
        eprintln!("error: can't write {}: {}", c_path.display(), e);
        exit(1);
    }
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = Command::new(&cc);
    cmd.args(HARDENED);
    if check_memory {
        cmd.arg("-DNX_LEAK_CHECK");
    }
    let status = cmd.arg(&c_path).arg("-o").arg(out).arg("-lm").status();
    let _ = std::fs::remove_file(&c_path);
    match status {
        Ok(s) if s.success() => {}
        Ok(_) => {
            eprintln!("error: the C compiler failed on generated code. This is a bug in nexac; please report it with your .nx file.");
            exit(1);
        }
        Err(e) => {
            eprintln!("error: can't start the C compiler `{}`: {} (install gcc, or set CC)", cc, e);
            exit(1);
        }
    }
}

fn main() {
    // Arguments meant for the program are passed on untouched (they may not
    // be UTF-8); nexac's own arguments are read as text.
    let raw: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let args: Vec<String> = raw.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let Some(cmd) = args.first() else { usage() };
    match cmd.as_str() {
        "--version" | "-V" => println!("nexac {}", VERSION),
        "check" if args.len() == 2 => {
            to_c_or_exit(&args[1]);
            eprintln!("ok: no mistakes found in {}", args[1]);
        }
        "emit-c" if args.len() == 2 => print!("{}", to_c_or_exit(&args[1])),
        "build" if args.len() == 2 || (args.len() == 4 && args[2] == "-o") => {
            let c = to_c_or_exit(&args[1]);
            let out = if args.len() == 4 {
                PathBuf::from(&args[3])
            } else {
                PathBuf::from(Path::new(&args[1]).file_stem().unwrap())
            };
            build_c(&c, &out, false);
        }
        "run" if args.len() >= 2 => {
            let check_memory = args[1] == "--check-memory";
            let first = if check_memory { 2 } else { 1 };
            let Some(file) = args.get(first) else { usage() };
            let c = to_c_or_exit(file);
            let dir = std::env::temp_dir().join(format!("nexac-{}", std::process::id()));
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("error: can't make a build folder: {}", e);
                exit(1);
            }
            let exe = dir.join("program");
            build_c(&c, &exe, check_memory);
            let status = Command::new(&exe).args(&raw[first + 1..]).status();
            let _ = std::fs::remove_dir_all(&dir);
            match status {
                Ok(s) => exit(s.code().unwrap_or(1)),
                Err(e) => {
                    eprintln!("error: can't start the program: {}", e);
                    exit(1);
                }
            }
        }
        _ => usage(),
    }
}
