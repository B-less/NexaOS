//! nexac: the NexaLang compiler.
//!
//!   nexac run   app.nx [args...]   build and run
//!   nexac run --check-memory app.nx  also report memory never freed
//!   nexac build app.nx [-o out]    build a native program
//!   nexac check app.nx             only check for mistakes
//!   nexac emit-c app.nx            print the generated C
//!   nexac check --json app.nx      mistakes as JSON, for editors (0.6)
//!   nexac fmt app.nx               lay the file out the one standard way (0.6)
//!   nexac explain [TOPIC]          what a kind of mistake means (0.6)
//!   nexac lsp                      language server on stdin/stdout (0.6)

mod ast;
mod checker;
mod codegen;
mod diag;
mod explain;
mod fmt;
mod json;
mod lsp;
mod lexer;
mod parser;

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `nexac build` and `nexac run` say on Windows (0.9).
const WINDOWS_NO_BUILD: &str = "error: NexaLang programs are built and run on NexaOS (or another Linux), not on Windows: \
the program has no mistakes, but the C it becomes uses NexaOS's system calls.\n  \
= hint: on Windows, nexac can check (nexac check), lay out (nexac fmt) and explain (nexac explain) programs; \
copy the .nx files to NexaOS and run `nexac run FILE.nx` there";

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
         nexac check --json [--stdin] FILE.nx   mistakes as JSON (--stdin: the text comes from stdin)\n  \
         nexac fmt FILE.nx...            lay files out the standard way (in place)\n  \
         nexac fmt --check FILE.nx...    only say which files aren't laid out that way\n  \
         nexac fmt --stdin               format stdin to stdout\n  \
         nexac explain [TOPIC | MESSAGE] what a kind of mistake means (--json for editors)\n  \
         nexac lsp [--stdio]             a language server for editors, on stdin/stdout\n  \
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
    ("ui", include_str!("std/ui.nx")),
    ("editor", include_str!("std/editor.nx")),
    ("network", include_str!("std/network.nx")),
];

const MAX_LINES: usize = 999_999;

fn render_all(ds: &[diag::Diag], mods: &[ast::Module]) -> Vec<diag::Report> {
    ds.iter()
        .map(|d| {
            let m = &mods[d.file.unwrap_or(0)];
            diag::Report::at(d, &m.file, &m.src)
        })
        .collect()
}

fn parse_module(name: &str, file: String, src: String, is_std: bool) -> Result<ast::Module, Vec<diag::Report>> {
    if src.lines().count() > MAX_LINES {
        return Err(vec![diag::Report::general(
            &file,
            format!("{} has more than {} lines; split it into modules", file, MAX_LINES),
        )]);
    }
    let toks = lexer::lex(&src).map_err(|d| vec![diag::Report::at(&d, &file, &src)])?;
    let prog = parser::parse(toks).map_err(|d| vec![diag::Report::at(&d, &file, &src)])?;
    Ok(ast::Module { name: name.to_string(), file, src, is_std, prog })
}

/// Generated C, and whether it needs GTK (the program uses `ui`).
pub struct Compiled {
    pub c: String,
    pub gtk: bool,
    /// it uses `editor`: GtkSourceView 4 too (0.8)
    pub source: bool,
    /// it uses `network`: libcurl (0.10)
    pub net: bool,
}

/// Load the main file and everything it uses, then check and generate C.
pub fn compile_to_c(main_path: &Path) -> Result<Compiled, Vec<diag::Report>> {
    compile_source(main_path, None, true)
}

/// Like `compile_to_c`, but the main file's text can come from an editor
/// (`main_src`) instead of the disk; the modules it uses are read from the
/// disk next to `main_path`. `lines` false makes every source line 0 in
/// the generated C (to compare programs that differ only in layout).
pub fn compile_source(main_path: &Path, main_src: Option<String>, lines: bool) -> Result<Compiled, Vec<diag::Report>> {
    let display = display_name(main_path);
    let main_src = match main_src {
        Some(s) => s,
        None => read_text(main_path).map_err(|m| vec![diag::Report::general(&display, m)])?,
    };
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
            let fail = |msg: String, hint: Option<String>| -> Vec<diag::Report> {
                let mut d = diag::Diag::new(span, msg);
                if let Some(h) = hint {
                    d = d.hint(h);
                }
                vec![diag::Report::at(&d, &mods[i].file, &mods[i].src)]
            };
            let local = dirs[i].join(format!("{}.nx", name));
            if let Some((_, src)) = STD.iter().find(|(n, _)| *n == name) {
                // (a `Files.nx` isn't `files.nx` on NexaOS, so not on Windows either)
                if !mods[i].is_std && local.exists() && differently_spelled(&local).is_none() {
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
            if let Some(real) = differently_spelled(&local) {
                // Windows and macOS find `Helpers.nx` for `use helpers`, NexaOS
                // doesn't: the same message on both, so a program that
                // works on one works on the other (0.9).
                return Err(fail(
                    format!("can't find module `{}`: the file here is called {}, and capitals count", name, real),
                    Some(format!("rename the file to {}.nx, or the module in `use` to match the file", name)),
                ));
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
            let shown = local.strip_prefix(&main_dir).unwrap_or(&local).display().to_string();
            let src = read_text(&local).map_err(|m| vec![diag::Report::general(&shown, m)])?;
            mods.push(parse_module(&name, shown, src, false)?);
            dirs.push(local.parent().map(Path::to_path_buf).unwrap_or_default());
        }
        i += 1;
    }
    check_cycles(&mods)?;
    let checked = checker::check(&mut mods).map_err(|ds| render_all(&ds, &mods))?;
    Ok(Compiled { c: codegen::generate(&mods, &checked, lines), gtk: codegen::uses_ui(&mods), source: codegen::uses_editor(&mods), net: codegen::uses_network(&mods) })
}

/// When `path` exists only because the file system ignores capitals (on
/// Windows), the name the file really has. None when the name matches
/// exactly, or the folder can't be listed.
fn differently_spelled(path: &Path) -> Option<String> {
    let want = path.file_name()?.to_str()?;
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut other = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == want {
            return None;
        }
        if name.eq_ignore_ascii_case(want) || name.to_lowercase() == want.to_lowercase() {
            other = Some(name);
        }
    }
    other
}

/// Modules can't use each other in a circle.
fn check_cycles(mods: &[ast::Module]) -> Result<(), Vec<diag::Report>> {
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
        return Err(vec![diag::Report::at(&d, &mods[i].file, &mods[i].src)]);
    }
    Ok(())
}

/// The file as text, or the message (without `error: `) saying why not.
fn read_text(path: &Path) -> Result<String, String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| format!("{} isn't valid UTF-8 text", path.display())),
        Err(e) => Err(format!("can't read {}: {}", path.display(), e)),
    }
}

fn display_name(path: &Path) -> String {
    path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn to_c_or_exit(path: &str) -> Compiled {
    if !path.ends_with(".nx") {
        eprintln!("error: NexaLang files end in .nx: {}", path);
        exit(2);
    }
    match compile_to_c(Path::new(path)) {
        Ok(c) => c,
        Err(e) => {
            eprint!("{}", diag::render_reports(&e));
            exit(1);
        }
    }
}

/// GTK 3's compiler and linker flags (and GtkSourceView 4's, for programs
/// that `use editor`), from pkg-config (run with an argument list, never
/// a shell). Each flag is one argument. None (with a message) when they
/// aren't installed.
fn gtk_flags(which: &str, source: bool) -> Option<Vec<String>> {
    let mut cmd = Command::new("pkg-config");
    cmd.arg(which).arg("gtk+-3.0");
    if source {
        cmd.arg("gtksourceview-4");
    }
    match cmd.output() {
        Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).split_whitespace().map(String::from).collect()),
        _ if source => {
            eprintln!("error: this program uses `editor`, which needs GTK 3, GtkSourceView 4 and pkg-config (on NexaOS: apk add gtk+3.0-dev gtksourceview4-dev pkgconf)");
            None
        }
        _ => {
            eprintln!("error: this program uses `ui`, which needs GTK 3 and pkg-config (on NexaOS: apk add gtk+3.0-dev pkgconf)");
            None
        }
    }
}

/// libcurl's flags, for programs that `use network` (0.10).
fn curl_flags(which: &str) -> Option<Vec<String>> {
    match Command::new("pkg-config").arg(which).arg("libcurl").output() {
        Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).split_whitespace().map(String::from).collect()),
        _ => {
            eprintln!("error: this program uses `network`, which needs libcurl and pkg-config (on NexaOS: apk add curl-dev pkgconf)");
            None
        }
    }
}

/// Compile C to `out` with the hardened flags. Uses $CC or `cc`; starts
/// the compiler with an argument list, never through a shell. Programs
/// that use `ui` also get GTK's flags; every hardened flag stays. False
/// (with a message, and nothing left behind but `out`'s folder) when it
/// can't.
fn build_c(prog: &Compiled, out: &Path, check_memory: bool, ship: bool) -> bool {
    if cfg!(windows) {
        // 0.9: the generated C uses POSIX (sockets, signals, process
        // groups) and GTK's Unix parts; see NEXALANG.md's decisions.
        eprintln!("{}", WINDOWS_NO_BUILD);
        return false;
    }
    let c = &prog.c;
    let c_path = out.with_extension("nexa.c");
    // `nexac build app.nx -o build/app` makes `build/` when it's missing
    // (0.5: nexa-pack's projects build into a folder that starts out empty).
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("error: can't make the folder {}: {}", dir.display(), e);
            return false;
        }
    }
    let (mut cflags, mut libs) = if prog.gtk {
        let Some(c) = gtk_flags("--cflags", prog.source) else { return false };
        let Some(l) = gtk_flags("--libs", prog.source) else { return false };
        (c, l)
    } else {
        (Vec::new(), Vec::new())
    };
    if prog.net {
        let Some(c) = curl_flags("--cflags") else { return false };
        let Some(l) = curl_flags("--libs") else { return false };
        cflags.extend(c);
        libs.extend(l);
    }
    if let Err(e) = std::fs::write(&c_path, c) {
        eprintln!("error: can't write {}: {}", c_path.display(), e);
        return false;
    }
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = Command::new(&cc);
    cmd.args(HARDENED);
    if check_memory {
        cmd.arg("-DNX_LEAK_CHECK");
    }
    cmd.args(&cflags);
    cmd.arg(&c_path).arg("-o").arg(out).arg("-lm");
    cmd.args(&libs);
    // A shipped program carries no symbol table and no debug info: the names
    // of its functions, and the C toolchain's own source paths, don't belong
    // in something you hand out, and leaving them out makes it harder to read
    // back. Error messages don't need them — the runtime prints the .nx file
    // and line from its own table (nx_files[], see runtime.h). `nexac run`
    // builds a throwaway in /tmp and keeps the symbols; so does setting
    // NEXAC_KEEP_SYMBOLS, which the valgrind test uses to name the program's
    // own frames.
    if ship && !check_memory && std::env::var_os("NEXAC_KEEP_SYMBOLS").is_none() {
        cmd.arg("-s");
    }
    let status = cmd.status();
    let _ = std::fs::remove_file(&c_path);
    match status {
        Ok(s) if s.success() => true,
        Ok(_) => {
            eprintln!("error: the C compiler failed on generated code. This is a bug in nexac; please report it with your .nx file.");
            false
        }
        Err(e) => {
            eprintln!("error: can't start the C compiler `{}`: {} (install gcc, or set CC)", cc, e);
            false
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
        "emit-c" if args.len() == 2 => print!("{}", to_c_or_exit(&args[1]).c),
        // every source line written as 0: for comparing layouts (tests)
        "emit-c" if args.len() == 3 && args[1] == "--no-lines" => match compile_source(Path::new(&args[2]), None, false) {
            Ok(c) => print!("{}", c.c),
            Err(e) => {
                eprint!("{}", diag::render_reports(&e));
                exit(1);
            }
        },
        "check" if args.len() >= 3 && args[1] == "--json" => exit(check_json(&args[2..])),
        "fmt" if args.len() >= 2 => exit(fmt_cmd(&args[1..])),
        "explain" => exit(explain_cmd(&args[1..])),
        // `--stdio`: what many editors' language clients add (0.9)
        "lsp" if args.len() == 1 || (args.len() == 2 && args[1] == "--stdio") => exit(lsp::serve()),
        "build" if args.len() == 2 || (args.len() == 4 && args[2] == "-o") => {
            let c = to_c_or_exit(&args[1]);
            let out = if args.len() == 4 {
                PathBuf::from(&args[3])
            } else {
                PathBuf::from(Path::new(&args[1]).file_stem().unwrap())
            };
            if !build_c(&c, &out, false, true) {
                exit(1);
            }
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
            if !build_c(&c, &exe, check_memory, false) {
                let _ = std::fs::remove_dir_all(&dir);  // nothing left in /tmp (0.8)
                exit(1);
            }
            // The program is deleted as soon as it has started (a running
            // program doesn't need its file), so nothing is left behind
            // even when nexac is stopped while it runs (0.8: a code
            // editor's Stop button stops both).
            // its output line by line, as it happens, even through a pipe
            let child = Command::new(&exe).args(&raw[first + 1..]).env("NEXALANG_LINE_OUTPUT", "1").spawn();
            let _ = std::fs::remove_dir_all(&dir);
            if let Err(e) = &child {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    eprintln!(
                        "error: can't start the program: this system doesn't allow running programs from {} \
                         (an installed NexaOS app can't run programs it builds; run `nexac run {}` in a terminal)",
                        std::env::temp_dir().display(),
                        file
                    );
                    exit(1);
                }
            }
            let status = child.and_then(|mut c| c.wait());
            match status {
                Ok(s) => match s.code() {
                    Some(c) => exit(c),
                    None => {
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::ExitStatusExt;
                            eprintln!("error: the program was stopped by signal {}", s.signal().unwrap_or(0));
                        }
                        #[cfg(not(unix))]
                        eprintln!("error: the program was stopped");
                        exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("error: can't start the program: {}", e);
                    exit(1);
                }
            }
        }
        _ => usage(),
    }
}

/// `nexac check --json [--stdin] FILE.nx`: one JSON object on stdout.
/// Exit 0 with no mistakes, 1 with mistakes, 2 for a wrong command line.
fn check_json(args: &[String]) -> i32 {
    let (stdin, file) = match args {
        [f] => (false, f),
        [s, f] if s == "--stdin" => (true, f),
        _ => usage(),
    };
    let path = Path::new(file);
    let shown = display_name(path);
    let result = if !file.ends_with(".nx") {
        Err(vec![diag::Report::general(&shown, format!("NexaLang files end in .nx: {}", file))])
    } else if stdin {
        let mut bytes = Vec::new();
        use std::io::Read;
        match std::io::stdin().read_to_end(&mut bytes) {
            Ok(_) => match String::from_utf8(bytes) {
                Ok(src) => compile_source(path, Some(src), true),
                Err(_) => Err(vec![diag::Report::general(&shown, format!("{} isn't valid UTF-8 text", shown))]),
            },
            Err(e) => Err(vec![diag::Report::general(&shown, format!("can't read the text: {}", e))]),
        }
    } else {
        compile_to_c(path)
    };
    let reports = result.err().unwrap_or_default();
    let diags: Vec<json::Json> = reports
        .iter()
        .map(|r| {
            let t = explain::classify(&r.msg);
            json::Json::obj(vec![
                ("file", json::Json::str(r.file.clone())),
                ("line", json::Json::num(r.line)),
                ("column", json::Json::num(r.col)),
                ("length", json::Json::num(r.length)),
                ("message", json::Json::str(r.msg.clone())),
                ("hint", json::Json::opt_str(&r.hint)),
                ("topic", json::Json::str(t.name)),
                ("summary", json::Json::str(t.summary)),
            ])
        })
        .collect();
    let out = json::Json::obj(vec![
        ("nexac", json::Json::str(VERSION)),
        ("file", json::Json::str(shown)),
        ("ok", json::Json::Bool(reports.is_empty())),
        ("diagnostics", json::Json::Arr(diags)),
    ]);
    println!("{}", out.write());
    if reports.is_empty() {
        0
    } else {
        1
    }
}

/// `nexac fmt`: see fmt.rs. Exit 0 when everything is (now) formatted,
/// 1 when a file has a mistake that stops formatting or, with --check,
/// isn't formatted; 2 for a wrong command line.
fn fmt_cmd(args: &[String]) -> i32 {
    if args == ["--stdin"] {
        use std::io::{Read, Write};
        let mut bytes = Vec::new();
        if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
            eprintln!("error: can't read the text: {}", e);
            return 1;
        }
        let Ok(src) = String::from_utf8(bytes) else {
            eprintln!("error: the text isn't valid UTF-8");
            return 1;
        };
        return match fmt::format(&src) {
            Ok(out) => {
                let _ = std::io::stdout().write_all(out.as_bytes());
                0
            }
            Err(d) => {
                eprint!("{}", d.render("<stdin>", &src));
                1
            }
        };
    }
    let check = args[0] == "--check";
    let files = if check { &args[1..] } else { args };
    if files.is_empty() || files.iter().any(|f| f.starts_with('-')) {
        usage();
    }
    let mut status = 0;
    for file in files {
        let path = Path::new(file);
        if !file.ends_with(".nx") {
            eprintln!("error: NexaLang files end in .nx: {}", file);
            status = 1;
            continue;
        }
        let src = match read_text(path) {
            Ok(s) => s,
            Err(m) => {
                eprintln!("error: {}", m);
                status = 1;
                continue;
            }
        };
        let out = match fmt::format(&src) {
            Ok(o) => o,
            Err(d) => {
                eprint!("{}", d.render(file, &src));
                eprintln!("  = {} wasn't formatted: fix this first", file);
                status = 1;
                continue;
            }
        };
        if out == src {
            continue;
        }
        if check {
            eprintln!("{} isn't formatted (nexac fmt {} would change it)", file, file);
            status = 1;
            continue;
        }
        match write_all_or_nothing(path, out.as_bytes()) {
            Ok(()) => eprintln!("formatted {}", file),
            Err(e) => {
                eprintln!("error: can't write {}: {}", file, e);
                status = 1;
            }
        }
    }
    status
}

/// Replace a file without ever leaving it half-written: write a new file
/// next to it (same permissions), flush it to disk, rename it over the
/// old one. A link is followed, so the file it points to is replaced.
fn write_all_or_nothing(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let target = std::fs::canonicalize(path)?;
    let perms = std::fs::metadata(&target)?.permissions();
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = target.with_file_name(format!(".{}.nexac-fmt-{}", name, std::process::id()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.set_permissions(perms)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `nexac explain`, `nexac explain TOPIC`, `nexac explain "a message"`,
/// and `--json` for each.
fn explain_cmd(args: &[String]) -> i32 {
    let (as_json, rest) = match args.first().map(String::as_str) {
        Some("--json") => (true, &args[1..]),
        _ => (false, args),
    };
    if rest.len() > 1 {
        usage();
    }
    let topic_json = |t: &explain::Topic| {
        json::Json::obj(vec![
            ("topic", json::Json::str(t.name)),
            ("title", json::Json::str(t.title)),
            ("summary", json::Json::str(t.summary)),
            ("explanation", json::Json::str(t.body)),
            ("wrong", json::Json::str(t.wrong)),
            ("right", json::Json::str(t.right)),
        ])
    };
    let Some(what) = rest.first() else {
        if as_json {
            println!("{}", json::Json::Arr(explain::TOPICS.iter().map(topic_json).collect()).write());
        } else {
            print!("{}", explain::list());
        }
        return 0;
    };
    let topic = explain::find(what).or_else(|| {
        let t = explain::classify(what);
        // a single word that isn't a topic is a typo, not a message
        (t.name != "other" && what.contains(' ')).then_some(t)
    });
    match topic {
        Some(t) if as_json => println!("{}", topic_json(t).write()),
        Some(t) => print!("{}", explain::render(t)),
        None => {
            eprintln!("error: there's no topic called `{}`, and it isn't a message nexac gives\n", what);
            eprint!("{}", explain::list());
            return 2;
        }
    }
    0
}
