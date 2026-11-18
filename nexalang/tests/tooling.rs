//! 0.6's tools, through the real `nexac` binary:
//!
//! - `nexac fmt` never changes what a program means (the generated C of
//!   every example, test program, standard module user and app is the same
//!   before and after, apart from line numbers), is idempotent, keeps
//!   comments, and leaves rejected programs rejected with the same message;
//! - `nexac check --json` gives the same mistakes as `nexac check`, with
//!   file, line, column, hint and topic, for every rejected test program;
//! - `nexac explain`: every topic's example mistake is refused with that
//!   topic, and its fixed version is accepted;
//! - `nexac lsp`: a whole editor session over stdin/stdout, and hostile
//!   messages.

#[allow(dead_code)]
#[path = "../src/json.rs"]
mod json;

use json::Json;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The `nexac` under test: this build, or (0.9) another one named by
/// `NEXAC_TEST_EXE`, started through `NEXAC_TEST_WRAPPER` when that's set:
/// `NEXAC_TEST_WINDOWS=1 NEXAC_TEST_WRAPPER=wine NEXAC_TEST_EXE=nexac.exe
/// cargo test --test tooling` runs every test here against the Windows
/// `nexac` (`NEXAC_TEST_WINDOWS` skips only the Unix file mode check).
fn nexac() -> Command {
    let exe = std::env::var_os("NEXAC_TEST_EXE").unwrap_or_else(|| env!("CARGO_BIN_EXE_nexac").into());
    match std::env::var_os("NEXAC_TEST_WRAPPER") {
        Some(w) => {
            let mut c = Command::new(w);
            c.arg(exe);
            c
        }
        None => Command::new(exe),
    }
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nexac-tool-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn nx_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "nx"))
        .collect();
    v.sort();
    v
}

/// Every folder of .nx files in the repository that holds programs (not
/// the deliberately broken module tests, which are checked separately).
fn program_dirs() -> Vec<PathBuf> {
    let r = root();
    let mut dirs = vec![
        r.join("examples"),
        r.join("examples/ui"),
        r.join("tests/fail"),
        r.join("tests/trap"),
        r.join("tests/ui"),
        r.join("tests/ui/trap"),
        r.join("tests/broker"),
        r.join("tests/json"),
        r.join("tests/stress"),
        r.join("tests/modules/app"),
    ];
    for e in std::fs::read_dir(r.join("tests/modules/bad")).unwrap() {
        dirs.push(e.unwrap().path());
    }
    for e in std::fs::read_dir(r.join("../apps")).unwrap() {
        let src = e.unwrap().path().join("src");
        if src.join("main.nx").exists() {
            dirs.push(src);
        }
    }
    dirs.retain(|d| d.is_dir() && !nx_files(d).is_empty());
    dirs
}

/// `emit-c --no-lines`: (exit code, C or the first line of the error).
fn emit(p: &Path) -> (Option<i32>, String) {
    let out = nexac().arg("emit-c").arg("--no-lines").arg(p).output().unwrap();
    if out.status.success() {
        (out.status.code(), String::from_utf8(out.stdout).unwrap())
    } else {
        let e = String::from_utf8_lossy(&out.stderr);
        (out.status.code(), e.lines().next().unwrap_or("").to_string())
    }
}

#[test]
fn fmt_never_changes_what_a_program_means() {
    let mut compared = 0;
    let mut rejected = 0;
    let mut changed = 0;
    for dir in program_dirs() {
        let tmp = tmpdir(&format!("fmt-{}", compared + rejected));
        // the files, with their layout messed up
        for f in nx_files(&dir) {
            let src = std::fs::read_to_string(&f).unwrap();
            std::fs::write(tmp.join(f.file_name().unwrap()), mess_up(&src)).unwrap();
        }
        let names: Vec<PathBuf> = nx_files(&tmp);
        // format every file of the folder in place
        let out = nexac().arg("fmt").args(&names).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        for f in nx_files(&dir) {
            let copy = tmp.join(f.file_name().unwrap());
            let before = std::fs::read_to_string(&f).unwrap();
            let after = std::fs::read_to_string(&copy).unwrap();
            let messed = mess_up(&before);
            if messed != after {
                changed += 1;
            }
            // the repository's files are formatted, so formatting the
            // messed-up copy gives back exactly the original
            if fmt_ok(&f) {
                assert_eq!(after, before, "{}: nexac fmt didn't restore the layout", f.display());
            }
            // comments survive: every comment line's text is still there
            for line in before.lines() {
                if let Some(i) = line.find("//") {
                    let c = line[i..].trim_end();
                    if !line[..i].contains('"') {
                        assert!(after.contains(c), "{}: lost comment {:?}", f.display(), c);
                    }
                }
            }
            if !main_file(&f) {
                continue;
            }
            let (code_a, c_a) = emit(&f);
            let (code_b, c_b) = emit(&copy);
            assert_eq!(code_a, code_b, "{}", f.display());
            if code_a == Some(0) {
                compared += 1;
                assert!(c_a == c_b, "{}: the generated C changed after nexac fmt", f.display());
            } else {
                rejected += 1;
                // messages may name the folder the file is in
                let a = c_a.replace(dir.to_str().unwrap(), "DIR");
                let b = c_b.replace(tmp.to_str().unwrap(), "DIR");
                assert_eq!(a, b, "{}: a different mistake after nexac fmt", f.display());
            }
        }
        // idempotent: formatting again changes nothing (files that can't be
        // read as NexaLang were left alone, and say so)
        let check = nexac().arg("fmt").arg("--check").args(&names).output().unwrap();
        let cerr = String::from_utf8_lossy(&check.stderr);
        assert!(!cerr.contains("isn't formatted"), "{}: not idempotent:\n{}\n(first run: {})", dir.display(), cerr, err);
        let _ = std::fs::remove_dir_all(&tmp);
    }
    eprintln!("fmt: {} programs give the same C, {} rejected programs the same mistake, {} files changed", compared, rejected, changed);
    assert!(compared >= 30 && rejected >= 60, "too few programs found: {} {}", compared, rejected);
}

/// The same program with its layout messed up: no indentation, extra
/// spaces and tabs between words (not in lines with strings or comments,
/// where spaces mean something), spaces at line ends, more empty lines.
fn mess_up(src: &str) -> String {
    let mut out = String::new();
    for (i, line) in src.lines().enumerate() {
        let mut l = line.trim_start().to_string();
        if !l.contains('"') && !l.contains("//") {
            l = l.replace(' ', if i % 2 == 0 { "  " } else { " \t " });
        }
        out += &l;
        out += if i % 3 == 0 { " \t\n" } else { "\n" };
        if line.trim().is_empty() {
            out += "\n\n";
        }
    }
    out
}

/// Files that parse (only those can be formatted).
fn fmt_ok(f: &Path) -> bool {
    nexac().arg("fmt").arg("--check").arg(f).output().unwrap().status.success()
}

/// The file a folder's program starts from (modules aren't programs).
fn main_file(f: &Path) -> bool {
    let src = std::fs::read_to_string(f).unwrap_or_default();
    src.contains("fn main(") || f.parent().unwrap().ends_with("fail") || f.file_name().unwrap() == "main.nx"
}

#[test]
fn the_standard_modules_and_apps_are_formatted() {
    let mut files = nx_files(&root().join("src/std"));
    files.push(root().join("../apps/focus-timer/src/main.nx"));
    files.extend(nx_files(&root().join("examples")));
    let out = nexac().arg("fmt").arg("--check").args(&files).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn fmt_command_line() {
    let tmp = tmpdir("fmtcli");
    let p = tmp.join("a.nx");
    std::fs::write(&p, "fn main(){\nprint( 1 )   // one\n}").unwrap();
    // --check says so and changes nothing
    let out = nexac().arg("fmt").arg("--check").arg(&p).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("isn't formatted"));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "fn main(){\nprint( 1 )   // one\n}");
    // in place, keeping the file's permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let out = nexac().arg("fmt").arg(&p).output().unwrap();
    assert!(out.status.success());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "fn main() {\n    print(1)  // one\n}\n");
    // (a Windows nexac, under Wine, has only Windows' read-only flag to keep)
    #[cfg(unix)]
    if std::env::var_os("NEXAC_TEST_WINDOWS").is_none() {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o640);
    }
    // nothing else left in the folder
    assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 1);
    // stdin to stdout
    let mut child = nexac().arg("fmt").arg("--stdin").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"fn main(){print(2)}").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "fn main() { print(2) }\n");
    // a file that doesn't parse is left alone, with the mistake shown
    let bad = tmp.join("bad.nx");
    std::fs::write(&bad, "fn main() {\n  print(1\n").unwrap();
    let out = nexac().arg("fmt").arg(&bad).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let e = String::from_utf8_lossy(&out.stderr);
    assert!(e.contains("error: ") && e.contains("wasn't formatted"), "{}", e);
    assert_eq!(std::fs::read_to_string(&bad).unwrap(), "fn main() {\n  print(1\n");
    // not UTF-8, not .nx, missing
    let latin = tmp.join("latin.nx");
    std::fs::write(&latin, b"fn main() { print(\"\xe9\") }\n").unwrap();
    for (f, want) in [(latin.clone(), "isn't valid UTF-8"), (tmp.join("x.txt"), "end in .nx"), (tmp.join("gone.nx"), "can't read")] {
        let out = nexac().arg("fmt").arg(&f).output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains(want), "{}", f.display());
    }
    // a link: the file it points to is formatted, the link stays a link
    #[cfg(unix)]
    {
        let real = tmp.join("real.nx");
        std::fs::write(&real, "fn main(){ }").unwrap();
        let link = tmp.join("link.nx");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(nexac().arg("fmt").arg(&link).output().unwrap().status.success());
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "fn main() { }\n");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

fn check_json(p: &Path) -> (Option<i32>, Json) {
    let out = nexac().arg("check").arg("--json").arg(p).output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.lines().count(), 1, "one line of JSON");
    (out.status.code(), json::parse(text.trim_end()).unwrap())
}

#[test]
fn check_json_matches_the_text_messages() {
    let mut n = 0;
    let mut dirs = vec![root().join("tests/fail")];
    for e in std::fs::read_dir(root().join("tests/modules/bad")).unwrap() {
        dirs.push(e.unwrap().path());
    }
    for dir in dirs {
        for p in nx_files(&dir) {
            if dir.ends_with("fail") || p.file_name().unwrap() == "main.nx" {
                let text = nexac().arg("check").arg(&p).output().unwrap();
                let stderr = String::from_utf8_lossy(&text.stderr).to_string();
                let (code, j) = check_json(&p);
                assert_eq!(code, Some(1), "{}", p.display());
                assert_eq!(j.get("ok"), Some(&Json::Bool(false)));
                let ds = j.get("diagnostics").unwrap().as_arr().unwrap();
                assert!(!ds.is_empty());
                // the text output has the same message and position, in order
                for d in ds {
                    let msg = d.get("message").unwrap().as_str().unwrap();
                    let file = d.get("file").unwrap().as_str().unwrap();
                    let line = d.get("line").unwrap().as_u32().unwrap();
                    let col = d.get("column").unwrap().as_u32().unwrap();
                    assert!(stderr.contains(&format!("error: {}\n  --> {}:{}:{}\n", msg, file, line, col)), "{}: {:?} not in\n{}", p.display(), d, stderr);
                    if let Some(h) = d.get("hint").unwrap().as_str() {
                        assert!(stderr.contains(&format!("= hint: {}", h)));
                    }
                    let topic = d.get("topic").unwrap().as_str().unwrap();
                    assert!(topic != "other", "{}: {} has no topic", p.display(), msg);
                    assert!(!d.get("summary").unwrap().as_str().unwrap().is_empty());
                    assert!(d.get("length").unwrap().as_u32().unwrap() >= 1);
                }
                n += 1;
            }
        }
    }
    assert!(n >= 70, "{}", n);
    // a good program
    let (code, j) = check_json(&root().join("examples/hello.nx"));
    assert_eq!(code, Some(0));
    assert_eq!(j.get("ok"), Some(&Json::Bool(true)));
    assert_eq!(j.get("diagnostics").unwrap().as_arr().unwrap().len(), 0);
    // files that can't be read have line 0
    let tmp = tmpdir("json");
    let latin = tmp.join("latin.nx");
    std::fs::write(&latin, b"\xff").unwrap();
    for (p, want) in [(latin, "isn't valid UTF-8"), (tmp.join("gone.nx"), "can't read"), (tmp.join("a.txt"), "end in .nx")] {
        let (code, j) = check_json(&p);
        assert_eq!(code, Some(1));
        let d = &j.get("diagnostics").unwrap().as_arr().unwrap()[0];
        assert!(d.get("message").unwrap().as_str().unwrap().contains(want));
        assert_eq!(d.get("line").unwrap().as_u32(), Some(0));
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn check_json_reads_unsaved_text_from_stdin() {
    // the modules next to the named file are still found on the disk
    let main = root().join("tests/modules/app/main.nx");
    let src = std::fs::read_to_string(&main).unwrap();
    let run = |text: &str| -> (Option<i32>, Json) {
        let mut child = nexac().args(["check", "--json", "--stdin"]).arg(&main).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        (out.status.code(), json::parse(String::from_utf8(out.stdout).unwrap().trim_end()).unwrap())
    };
    let (code, j) = run(&src);
    assert_eq!(code, Some(0), "{:?}", j);
    let (code, j) = run(&(src.clone() + "\nfn extra() {\n    let é = 1\n}\n"));
    assert_eq!(code, Some(1));
    let d = &j.get("diagnostics").unwrap().as_arr().unwrap()[0];
    assert_eq!(d.get("file").unwrap().as_str(), Some("main.nx"));
    assert_eq!(d.get("line").unwrap().as_u32(), Some(src.lines().count() as u32 + 3));
    assert_eq!(d.get("column").unwrap().as_u32(), Some(9));
    assert_eq!(d.get("topic").unwrap().as_str(), Some("syntax"));
}

#[test]
fn every_explanation_example_does_what_it_says() {
    let out = nexac().args(["explain", "--json"]).output().unwrap();
    assert!(out.status.success());
    let topics = json::parse(String::from_utf8(out.stdout).unwrap().trim_end()).unwrap();
    let topics = topics.as_arr().unwrap().to_vec();
    assert!(topics.len() >= 20);
    let tmp = tmpdir("explain");
    for t in &topics {
        let name = t.get("topic").unwrap().as_str().unwrap();
        let wrong = t.get("wrong").unwrap().as_str().unwrap();
        let right = t.get("right").unwrap().as_str().unwrap();
        // the text form works too and names the command
        let text = nexac().arg("explain").arg(name).output().unwrap();
        assert!(text.status.success());
        assert!(String::from_utf8_lossy(&text.stdout).contains(&format!("({})", name)));
        if wrong.is_empty() {
            continue;
        }
        let p = tmp.join(format!("{}.nx", name.replace('-', "_")));
        std::fs::write(&p, wrong).unwrap();
        let (code, j) = check_json(&p);
        assert_eq!(code, Some(1), "{}: the wrong example is accepted", name);
        let first = &j.get("diagnostics").unwrap().as_arr().unwrap()[0];
        assert_eq!(first.get("topic").unwrap().as_str(), Some(name), "{}: {:?}", name, first);
        std::fs::write(&p, right).unwrap();
        let (code, j) = check_json(&p);
        assert_eq!(code, Some(0), "{}: the right example is refused: {:?}", name, j);
        // examples are formatted
        assert!(nexac().arg("fmt").arg("--check").arg(&p).output().unwrap().status.success(), "{} isn't formatted", name);
    }
    // a message instead of a topic
    let out = nexac().args(["explain", "can't change `x`: it was made with `let` (or is a parameter)"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("(let-var)"));
    // a typo
    let out = nexac().args(["explain", "lett-var"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("there's no topic"));
    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------- the language server ----------

struct Lsp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
}

impl Lsp {
    fn start() -> Lsp {
        let mut child = nexac().arg("lsp").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Lsp { child, out }
    }
    fn raw(&mut self, bytes: &[u8]) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(bytes).unwrap();
        stdin.flush().unwrap();
    }
    fn send(&mut self, msg: &str) {
        self.raw(format!("Content-Length: {}\r\n\r\n{}", msg.len(), msg).as_bytes());
    }
    fn recv(&mut self) -> Json {
        let mut len = 0;
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "the server stopped");
            let l = line.trim_end();
            if l.is_empty() {
                break;
            }
            if let Some(v) = l.strip_prefix("Content-Length: ") {
                len = v.parse().unwrap();
            }
        }
        let mut body = vec![0; len];
        self.out.read_exact(&mut body).unwrap();
        json::parse(std::str::from_utf8(&body).unwrap()).unwrap()
    }
}

fn s(x: &str) -> String {
    Json::str(x).write()
}

#[test]
fn language_server_session() {
    let tmp = tmpdir("lsp");
    let path = tmp.join("a b é.nx");
    std::fs::write(&path, "").unwrap();
    let uri = format!("file://{}", tmp.to_str().unwrap().replace(' ', "%20")) + "/a%20b%20%C3%A9.nx";
    let mut l = Lsp::start();
    // before initialize
    l.send(r#"{"jsonrpc":"2.0","id":0,"method":"textDocument/hover","params":{}}"#);
    assert_eq!(l.recv().path(&["error", "code"]).unwrap().as_f64(), Some(-32002.0));
    l.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#);
    let init = l.recv();
    assert_eq!(init.path(&["result", "capabilities", "documentFormattingProvider"]), Some(&Json::Bool(true)));
    assert_eq!(init.path(&["result", "capabilities", "textDocumentSync", "change"]).unwrap().as_f64(), Some(1.0));
    l.send(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#);
    // a mistake after a letter that is two UTF-16 units: the column counts them
    let bad = "fn main() {\n    let x = \"😀\" + cuont\n}\n";
    l.send(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":{},"languageId":"nexalang","version":1,"text":{}}}}}}}"#,
        s(&uri),
        s(bad)
    ));
    let pd = l.recv();
    assert_eq!(pd.get("method").unwrap().as_str(), Some("textDocument/publishDiagnostics"));
    assert_eq!(pd.path(&["params", "uri"]).unwrap().as_str(), Some(uri.as_str()));
    let ds = pd.path(&["params", "diagnostics"]).unwrap().as_arr().unwrap();
    assert_eq!(ds.len(), 1, "{:?}", ds);
    let d = &ds[0];
    assert!(d.get("message").unwrap().as_str().unwrap().starts_with("unknown name `cuont`"));
    assert_eq!(d.get("code").unwrap().as_str(), Some("unknown"));
    assert_eq!(d.path(&["range", "start", "line"]).unwrap().as_f64(), Some(1.0));
    // "    let x = "😀" + cuont": c is the 19th character, the 20th UTF-16 unit
    assert_eq!(d.path(&["range", "start", "character"]).unwrap().as_f64(), Some(19.0));
    assert_eq!(d.path(&["range", "end", "character"]).unwrap().as_f64(), Some(24.0));
    // hover on the mistake explains it; elsewhere nothing
    l.send(&format!(r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{{"textDocument":{{"uri":{}}},"position":{{"line":1,"character":21}}}}}}"#, s(&uri)));
    let h = l.recv();
    let md = h.path(&["result", "contents", "value"]).unwrap().as_str().unwrap();
    assert!(md.contains("cuont") && md.contains("nexac explain unknown"), "{}", md);
    l.send(&format!(r#"{{"jsonrpc":"2.0","id":3,"method":"textDocument/hover","params":{{"textDocument":{{"uri":{}}},"position":{{"line":0,"character":1}}}}}}"#, s(&uri)));
    assert_eq!(l.recv().get("result"), Some(&Json::Null));
    // change to a program that's right but badly laid out
    let messy = "fn main(){\nprint( 1 )\n}";
    l.send(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":{},"version":2}},"contentChanges":[{{"text":{}}}]}}}}"#,
        s(&uri),
        s(messy)
    ));
    let pd = l.recv();
    assert_eq!(pd.path(&["params", "diagnostics"]).unwrap().as_arr().unwrap().len(), 0);
    l.send(&format!(r#"{{"jsonrpc":"2.0","id":4,"method":"textDocument/formatting","params":{{"textDocument":{{"uri":{}}},"options":{{"tabSize":4,"insertSpaces":true}}}}}}"#, s(&uri)));
    let f = l.recv();
    let edits = f.get("result").unwrap().as_arr().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].get("newText").unwrap().as_str(), Some("fn main() {\n    print(1)\n}\n"));
    assert_eq!(edits[0].path(&["range", "end", "line"]).unwrap().as_f64(), Some(2.0));
    assert_eq!(edits[0].path(&["range", "end", "character"]).unwrap().as_f64(), Some(1.0));
    // hostile messages: bad JSON, not UTF-8, no Content-Length, wrong
    // types, an unknown method; the server answers and carries on
    l.send("{not json");
    assert_eq!(l.recv().path(&["error", "code"]).unwrap().as_f64(), Some(-32700.0));
    l.raw(b"Content-Length: 2\r\n\r\n\xff\xfe");
    assert_eq!(l.recv().path(&["error", "code"]).unwrap().as_f64(), Some(-32700.0));
    l.raw(b"X-Nothing: 1\r\n\r\n");
    assert_eq!(l.recv().path(&["error", "code"]).unwrap().as_f64(), Some(-32700.0));
    l.send(r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":5,"text":[]}}}"#);
    l.send(r#"{"jsonrpc":"2.0","id":5,"method":"textDocument/hover","params":{"position":{"line":-1,"character":1e300}}}"#);
    assert_eq!(l.recv().get("result"), Some(&Json::Null));
    l.send(r#"{"jsonrpc":"2.0","id":6,"method":"workspace/symbol","params":{}}"#);
    assert_eq!(l.recv().path(&["error", "code"]).unwrap().as_f64(), Some(-32601.0));
    l.send(&("[".repeat(100_000)));
    assert_eq!(l.recv().path(&["error", "code"]).unwrap().as_f64(), Some(-32700.0));
    // a huge message is skipped, and the next one still works
    l.raw(format!("Content-Length: {}\r\n\r\n", (64 << 20) + 1).as_bytes());
    l.raw(&vec![b' '; (64 << 20) + 1]);
    assert_eq!(l.recv().path(&["error", "code"]).unwrap().as_f64(), Some(-32700.0));
    // a document that isn't open
    l.send(r#"{"jsonrpc":"2.0","id":7,"method":"textDocument/formatting","params":{"textDocument":{"uri":"file:///nope.nx"}}}"#);
    assert_eq!(l.recv().path(&["error", "code"]).unwrap().as_f64(), Some(-32602.0));
    // closing clears the mistakes; shutdown then exit ends with status 0
    l.send(&format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didClose","params":{{"textDocument":{{"uri":{}}}}}}}"#, s(&uri)));
    assert_eq!(l.recv().path(&["params", "diagnostics"]).unwrap().as_arr().unwrap().len(), 0);
    l.send(r#"{"jsonrpc":"2.0","id":8,"method":"shutdown"}"#);
    assert_eq!(l.recv().get("result"), Some(&Json::Null));
    l.send(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    assert_eq!(l.child.wait().unwrap().code(), Some(0));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn language_server_finds_modules_and_reports_their_mistakes() {
    // an open file whose module has a mistake: shown at the top, naming it
    let main = root().join("tests/modules/bad/error_in_module/main.nx");
    let text = std::fs::read_to_string(&main).unwrap();
    let mut l = Lsp::start();
    l.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    l.recv();
    let uri = format!("file://{}", main.canonicalize().unwrap().to_str().unwrap());
    l.send(&format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":{},"text":{}}}}}}}"#, s(&uri), s(&text)));
    let pd = l.recv();
    let ds = pd.path(&["params", "diagnostics"]).unwrap().as_arr().unwrap();
    assert!(!ds.is_empty());
    let m = ds[0].get("message").unwrap().as_str().unwrap();
    assert!(m.starts_with("in ") && m.contains(".nx:"), "{}", m);
    assert_eq!(ds[0].path(&["range", "start", "line"]).unwrap().as_f64(), Some(0.0));
    // end of input without shutdown: status 1
    drop(l.child.stdin.take());
    assert_eq!(l.child.wait().unwrap().code(), Some(1));
}

// ---------- syntax colours ----------

fn have_cmd(cmd: &str, arg: &str) -> bool {
    Command::new(cmd).arg(arg).output().is_ok_and(|o| o.status.success())
}

/// tests/highlight/sample.nx has every kind of word; sample.expected shows
/// the colour each character should get. Both editors' grammars must give
/// exactly that.
#[test]
fn gtksourceview_colours_the_sample_as_expected() {
    let dir = root().join("tests/highlight");
    let ok = Command::new("pkg-config").args(["--exists", "gtksourceview-4"]).status().is_ok_and(|s| s.success());
    if !ok || !have_cmd("xvfb-run", "--help") {
        eprintln!("SKIPPED: needs gtksourceview-4 (libgtksourceview-4-dev) and xvfb-run");
        return;
    }
    let tmp = tmpdir("lang");
    let exe = tmp.join("check_lang");
    let flags = String::from_utf8(Command::new("pkg-config").args(["--cflags", "--libs", "gtksourceview-4"]).output().unwrap().stdout).unwrap();
    let st = Command::new("gcc")
        .args(["-std=gnu11", "-O2", "-Wall", "-Wextra", "-Werror"])
        .arg(dir.join("check_lang.c"))
        .arg("-o")
        .arg(&exe)
        .args(flags.split_whitespace())
        .status()
        .unwrap();
    assert!(st.success());
    // GTK wants a display to register its types (no window is opened)
    let out = Command::new("xvfb-run")
        .arg("-a")
        .arg(&exe)
        .arg(root().join("editors/gtksourceview"))
        .arg(&dir)
        .arg(dir.join("sample.nx"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{}", stderr);
    assert!(!stderr.contains("WARNING") && !stderr.contains("CRITICAL"), "{}", stderr);
    let want = std::fs::read_to_string(dir.join("sample.expected")).unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), want);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn the_vscode_grammar_colours_the_sample_the_same_way() {
    let dir = root().join("tests/highlight");
    let mods = dir.join("node_modules");
    if !have_cmd("node", "--version") || !mods.join("vscode-textmate").exists() {
        eprintln!("SKIPPED: needs node, and `npm ci` in tests/highlight (vscode-textmate, VS Code's own grammar engine)");
        return;
    }
    let out = Command::new("node")
        .arg(dir.join("tm_check.js"))
        .arg(&mods)
        .arg(root().join("editors/vscode/syntaxes/nexalang.tmLanguage.json"))
        .arg(dir.join("sample.nx"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let want = std::fs::read_to_string(dir.join("sample.expected")).unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), want);
    // the grammar is valid JSON with the scope VS Code needs
    let g = json::parse(&std::fs::read_to_string(root().join("editors/vscode/syntaxes/nexalang.tmLanguage.json")).unwrap()).unwrap();
    assert_eq!(g.get("scopeName").unwrap().as_str(), Some("source.nexalang"));
    json::parse(&std::fs::read_to_string(root().join("editors/vscode/language-configuration.json")).unwrap()).unwrap();
}

/// The VS Code extension's own tests (0.9: `editors/vscode/test`, run with
/// node's test runner), against the nexac under test: its logic, what it
/// asks VS Code to do, and whole sessions with `nexac lsp` and `explain`.
#[test]
fn the_vscode_extension_passes_its_tests() {
    let dir = root().join("editors/vscode");
    if !have_cmd("node", "--version") || !dir.join("node_modules/vscode-languageclient").exists() {
        eprintln!("SKIPPED: needs node, and `npm ci` in editors/vscode");
        return;
    }
    if std::env::var_os("NEXAC_TEST_WRAPPER").is_some() {
        eprintln!("SKIPPED: the extension's tests start nexac themselves; give NEXAC_TEST_EXE a script instead of a wrapper");
        return;
    }
    let exe = std::env::var_os("NEXAC_TEST_EXE").unwrap_or_else(|| env!("CARGO_BIN_EXE_nexac").into());
    let mut cmd = Command::new("node");
    cmd.arg("--test").current_dir(&dir).env("NEXAC", exe);
    for t in ["lib", "extension", "nexac"] {
        cmd.arg(format!("test/{}.test.js", t));
    }
    if std::env::var_os("NEXAC_TEST_WINDOWS").is_some() {
        cmd.env("NEXAC_WINDOWS", "1");
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}\n{}", stdout, String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("# fail 0") && stdout.contains("# pass 12"), "{}", stdout);
}
