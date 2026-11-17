//! End-to-end tests through the real `nexac` binary.
//!
//! examples/*.nx   build (hardened flags, -Werror), print exactly *.out,
//!                 and free every byte (--check-memory). Optional *.in is
//!                 fed as input, *.args gives the arguments.
//!                 Also run under AddressSanitizer + UBSan, and Valgrind.
//! tests/fail/*.nx must be rejected; first line `// error: TEXT` must appear
//! tests/trap/*.nx must stop with exit 1 and `// trap: TEXT`
//! tests/stress    must run in bounded memory.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn nexac() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nexac"))
}

fn files(dir: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root().join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "nx"))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no tests in {}", dir);
    v
}

fn expectation(p: &Path, key: &str) -> String {
    let src = std::fs::read_to_string(p).unwrap();
    let first = src.lines().next().unwrap_or("");
    let prefix = format!("// {}: ", key);
    first.strip_prefix(&prefix).unwrap_or_else(|| panic!("{} must start with `{}`", p.display(), prefix)).to_string()
}

fn input_of(p: &Path) -> Vec<u8> {
    std::fs::read(p.with_extension("in")).unwrap_or_default()
}

/// Arguments from *.args; `{tmp}` becomes a new empty folder for this run.
fn args_of(p: &Path) -> Vec<String> {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    std::fs::read_to_string(p.with_extension("args"))
        .map(|s| {
            s.split_whitespace()
                .map(|a| {
                    if a.contains("{tmp}") {
                        let n = RUN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let d = std::env::temp_dir().join(format!("nexac-run-{}-{}", std::process::id(), n));
                        let _ = std::fs::remove_dir_all(&d);
                        a.replace("{tmp}", &d.to_string_lossy())
                    } else {
                        a.to_string()
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every runnable program the memory tools check: the examples plus the
/// multi-file program.
fn programs() -> Vec<PathBuf> {
    let mut v = files("examples");
    v.push(root().join("tests/modules/app/main.nx"));
    v
}

fn expected_of(p: &Path) -> String {
    let out = p.with_extension("out");
    let f = if out.exists() { out } else { p.parent().unwrap().join("expected.out") };
    std::fs::read_to_string(f).unwrap()
}

/// Run a command with the given input on stdin.
fn run_with_input(mut cmd: Command, input: &[u8]) -> Output {
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn have(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nexac-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn examples_print_expected_output_and_free_all_memory() {
    let mut compilers = vec!["gcc"];
    if have("clang") {
        compilers.push("clang");
    }
    for cc in compilers {
        for p in programs() {
            let mut cmd = nexac();
            cmd.env("CC", cc).arg("run").arg("--check-memory").arg(&p).args(args_of(&p));
            let out = run_with_input(cmd, &input_of(&p));
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "[{}] {} failed ({:?}):\n{}", cc, p.display(), out.status.code(), stderr);
            assert_eq!(String::from_utf8_lossy(&out.stdout), expected_of(&p), "[{}] {}", cc, p.display());
        }
    }
}

#[test]
fn examples_are_clean_under_address_and_undefined_sanitizers() {
    let dir = tmpdir("asan");
    for p in programs() {
        let c = nexac().arg("emit-c").arg(&p).output().unwrap();
        assert!(c.status.success());
        let cfile = dir.join("prog.c");
        std::fs::write(&cfile, &c.stdout).unwrap();
        let exe = dir.join("prog");
        let st = Command::new("gcc")
            .args(["-std=gnu11", "-O1", "-g", "-fsanitize=address,undefined", "-fno-sanitize-recover=all"])
            .args(["-fno-omit-frame-pointer", "-DNX_LEAK_CHECK", "-Wall", "-Wextra", "-Werror"])
            .arg(&cfile)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .status()
            .unwrap();
        assert!(st.success(), "{} didn't build with sanitizers", p.display());
        let mut cmd = Command::new(&exe);
        cmd.args(args_of(&p)).env("ASAN_OPTIONS", "detect_leaks=1:abort_on_error=0");
        let out = run_with_input(cmd, &input_of(&p));
        assert!(out.status.success(), "{} under sanitizers:\n{}", p.display(), String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected_of(&p), "{}", p.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn examples_are_clean_under_valgrind() {
    if !have("valgrind") {
        eprintln!("valgrind not installed; skipped");
        return;
    }
    let dir = tmpdir("vg");
    for p in programs() {
        let exe = dir.join("prog");
        let st = nexac().arg("build").arg(&p).arg("-o").arg(&exe).status().unwrap();
        assert!(st.success());
        let mut cmd = Command::new("valgrind");
        cmd.args(["-q", "--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=all"]).arg(&exe).args(args_of(&p));
        let out = run_with_input(cmd, &input_of(&p));
        assert!(out.status.success(), "{} under valgrind:\n{}", p.display(), String::from_utf8_lossy(&out.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn long_running_program_uses_flat_memory() {
    let dir = tmpdir("stress");
    let exe = dir.join("churn");
    let st = nexac().arg("build").arg(root().join("tests/stress/churn.nx")).arg("-o").arg(&exe).status().unwrap();
    assert!(st.success());
    // 64 MB of address space is far more than it needs and far less than
    // it would use if memory leaked. The program path is ours, not input.
    let out = Command::new("sh")
        .arg("-c")
        .arg("ulimit -v 65536 && exec \"$0\"")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(out.status.success(), "stress run failed: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).split_whitespace().nth(1), Some("5"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_programs_are_rejected_with_a_clear_message() {
    for p in files("tests/fail") {
        let want = expectation(&p, "error");
        let out = nexac().arg("check").arg(&p).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{} should be rejected", p.display());
        assert!(stderr.contains(&want), "{}: expected `{}` in:\n{}", p.display(), want, stderr);
        assert!(stderr.contains("-->"), "{}: error has no position:\n{}", p.display(), stderr);
    }
}

#[test]
fn runtime_errors_stop_cleanly() {
    for p in files("tests/trap") {
        let want = expectation(&p, "trap");
        let out = nexac().arg("run").arg(&p).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{}: {}", p.display(), stderr);
        assert!(stderr.starts_with("nexa: runtime error in "), "{}: {}", p.display(), stderr);
        assert!(stderr.contains(&want), "{}: expected `{}` in:\n{}", p.display(), want, stderr);
    }
}

#[test]
fn output_before_a_trap_is_not_lost() {
    let out = nexac().arg("run").arg(root().join("tests/trap/assert.nx")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "before\n");
}

#[test]
fn leak_check_catches_a_leak() {
    // Prove the leak counter works: drop one release from generated code.
    let c = nexac().arg("emit-c").arg(root().join("examples/values.nx")).output().unwrap().stdout;
    let c = String::from_utf8(c).unwrap();
    let main_at = c.find("static void nx_fn_main").unwrap();
    let rel = main_at + c[main_at..].find("nx_arr_release(").unwrap();
    let mut leaky = c.clone();
    leaky.insert_str(rel, "if (0) ");
    let dir = tmpdir("leak");
    std::fs::write(dir.join("p.c"), leaky).unwrap();
    let st = Command::new("gcc")
        .args(["-std=gnu11", "-O2", "-DNX_LEAK_CHECK"])
        .arg(dir.join("p.c"))
        .arg("-o")
        .arg(dir.join("p"))
        .arg("-lm")
        .status()
        .unwrap();
    assert!(st.success());
    let out = Command::new(dir.join("p")).output().unwrap();
    assert_eq!(out.status.code(), Some(3), "leak not detected");
    assert!(String::from_utf8_lossy(&out.stderr).contains("never freed"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn generated_c_is_reproducible() {
    let mut all = files("examples");
    all.extend(files("examples/ui"));
    all.extend(files("tests/ui"));
    for p in all {
        let a = nexac().arg("emit-c").arg(&p).output().unwrap().stdout;
        let b = nexac().arg("emit-c").arg(&p).output().unwrap().stdout;
        assert!(!a.is_empty());
        assert_eq!(a, b, "{}", p.display());
    }
}

#[test]
fn build_writes_a_program() {
    let dir = tmpdir("build");
    let exe = dir.join("hello");
    let st = nexac().arg("build").arg(root().join("examples/hello.nx")).arg("-o").arg(&exe).status().unwrap();
    assert!(st.success());
    let out = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Hello from NexaLang on NexaOS!\n");
    assert!(!dir.join("hello.nexa.c").exists(), "temporary C file left behind");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn module_mistakes_are_reported_in_the_right_file() {
    let bad = root().join("tests/modules/bad");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&bad).unwrap().map(|e| e.unwrap().path()).collect();
    cases.sort();
    assert!(cases.len() >= 11);
    for case in cases {
        let expect = std::fs::read_to_string(case.join("expect.txt")).unwrap();
        let (file, msg) = expect.trim_end().split_once('|').unwrap();
        let out = nexac().arg("check").arg(case.join("main.nx")).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{}", case.display());
        assert!(stderr.contains(msg), "{}: expected `{}` in:\n{}", case.display(), msg, stderr);
        assert!(stderr.contains(&format!("--> {}", file)), "{}: expected it in {}:\n{}", case.display(), file, stderr);
    }
}

#[test]
fn an_error_reaching_the_end_of_main_is_printed() {
    let out = nexac().arg("run").arg("--check-memory").arg(root().join("tests/main_err/divide.nx")).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "half of 10 is 5\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "error: `x` isn't a number\n");
}

#[test]
fn broker_client_talks_to_a_broker() {
    if !have("python3") {
        eprintln!("python3 not installed; skipped");
        return;
    }
    let dir = tmpdir("broker");
    let sock = dir.join("broker.sock");
    let mut fake = Command::new("python3")
        .arg(root().join("tests/broker/fake_broker.py"))
        .arg(&sock)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // wait for "ready"
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(fake.stdout.take().unwrap()), &mut line).unwrap();
    assert_eq!(line.trim(), "ready");
    let out = nexac()
        .arg("run")
        .arg("--check-memory")
        .arg(root().join("tests/broker/client.nx"))
        .env("NEXA_BROKER_SOCKET", &sock)
        .output()
        .unwrap();
    let _ = fake.wait();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = String::from_utf8_lossy(&out.stdout).to_string();
    let want = std::fs::read_to_string(root().join("tests/broker/expected.out")).unwrap();
    // the last request goes out after the broker stopped: only the start is fixed
    for (g, w) in got.lines().zip(want.lines()) {
        assert!(g.starts_with(w), "got `{}`, expected `{}`", g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count());
    // and with no broker at all
    let out = nexac()
        .arg("run")
        .arg(root().join("tests/broker/client.nx"))
        .env("NEXA_BROKER_SOCKET", dir.join("missing.sock"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("set_brightness: error: the system broker isn't running"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- windows (`use ui`, GTK 3) -------------------------------------------
//
// tests/ui/*.steps drive a program with real key presses on a private
// headless X server (tests/ui/drive.py: Xvfb + xdotool); the first line
// names the program (`# program: PATH`), `# broker` starts the stand-in
// broker, and NAME.out is the exact output expected. Each is run three
// ways: built by nexac with the hardened flags and the leak counter;
// with AddressSanitizer + UBSan; and under Valgrind. GTK and its font
// libraries keep caches for the life of the process, so for windows a
// leak only counts when the program's own code is on the stack (every
// NexaLang byte is still counted exactly by the leak counter).

fn have_gtk() -> bool {
    Command::new("pkg-config").args(["--exists", "gtk+-3.0"]).status().is_ok_and(|s| s.success())
}

fn have_display_tools() -> bool {
    ["Xvfb", "xdotool"].iter().all(|t| Command::new("which").arg(t).output().is_ok_and(|o| o.status.success()))
}

fn gtk_flags(which: &str) -> Vec<String> {
    let o = Command::new("pkg-config").arg(which).arg("gtk+-3.0").output().unwrap();
    String::from_utf8(o.stdout).unwrap().split_whitespace().map(String::from).collect()
}

/// Window programs: the examples, and the test programs.
fn ui_programs() -> Vec<PathBuf> {
    let mut v = files("examples/ui");
    v.extend(files("tests/ui"));
    v
}

struct Steps {
    file: PathBuf,
    program: PathBuf,
    broker: bool,
}

fn ui_steps() -> Vec<Steps> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root().join("tests/ui"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "steps"))
        .collect();
    v.sort();
    v.into_iter()
        .map(|file| {
            let src = std::fs::read_to_string(&file).unwrap();
            let prog = src.lines().next().and_then(|l| l.strip_prefix("# program: ")).unwrap_or_else(|| panic!("{} must start with `# program: PATH`", file.display()));
            Steps { program: root().join(prog), broker: src.lines().any(|l| l == "# broker"), file }
        })
        .collect()
}

/// A stand-in broker on a new socket, for as long as the value lives.
struct FakeBroker {
    child: std::process::Child,
    sock: PathBuf,
}

impl FakeBroker {
    fn start(dir: &Path) -> FakeBroker {
        let sock = dir.join("broker.sock");
        let mut child = Command::new("python3").arg(root().join("tests/broker/fake_broker.py")).arg(&sock).stdout(Stdio::piped()).spawn().unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(child.stdout.take().unwrap()), &mut line).unwrap();
        assert_eq!(line.trim(), "ready");
        FakeBroker { child, sock }
    }
}

impl Drop for FakeBroker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `cmd` driven by the steps; returns its output.
fn drive(s: &Steps, dir: &Path, cmd: &[&std::ffi::OsStr], wait_s: u32, shots: Option<&Path>) -> Output {
    drive_env(s, dir, cmd, wait_s, shots, &[])
}

fn drive_env(s: &Steps, dir: &Path, cmd: &[&std::ffi::OsStr], wait_s: u32, shots: Option<&Path>, env: &[(&str, &str)]) -> Output {
    let broker = if s.broker { Some(FakeBroker::start(dir)) } else { None };
    let mut c = Command::new("python3");
    c.arg(root().join("tests/ui/drive.py")).arg(&s.file).arg("--").args(cmd);
    c.env("NX_WINDOW_WAIT", wait_s.to_string());
    if let Some(b) = &broker {
        c.env("NEXA_BROKER_SOCKET", &b.sock);
    }
    if let Some(d) = shots {
        c.env("NX_SHOTS", d);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

/// Leak reports (LeakSanitizer or Valgrind) where the program's own code
/// allocated the memory: after the allocator (and GLib's allocation
/// wrappers), one of the next two frames is in the program (`marker`).
/// Leaks allocated deep inside GTK, pango or fontconfig (their caches) are
/// theirs, even though the program is further down every stack.
fn own_leaks(report: &str, starts: &[&str], marker: &str) -> Vec<String> {
    let blocks = leak_blocks(report, starts);
    blocks
        .into_iter()
        .filter(|b| {
            let frames: Vec<&str> = b
                .lines()
                .filter(|l| {
                    let t = l.trim_start_matches(|c: char| c == '=' || c.is_ascii_digit()).trim_start();
                    t.starts_with('#') || t.starts_with("at 0x") || t.starts_with("by 0x")
                })
                .filter(|l| !(l.contains("libsanitizer") || l.contains("vgpreload") || l.contains("libglib-2.0")))
                .take(2)
                .collect();
            frames.iter().any(|f| f.contains(marker))
        })
        .collect()
}

fn leak_blocks(report: &str, starts: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    let mut cur: Option<String> = None;
    for line in report.lines() {
        let body = line.trim_start_matches(|c: char| c == '=' || c.is_ascii_digit()).trim();
        if starts.iter().any(|s| body.contains(s)) {
            found.extend(cur.take());
            cur = Some(String::new());
        }
        if body.is_empty() {
            found.extend(cur.take());
            continue;
        }
        if let Some(b) = cur.as_mut() {
            b.push_str(line);
            b.push('\n');
        }
    }
    found.extend(cur);
    found
}

#[test]
fn window_programs_build_with_the_hardened_flags() {
    if !have_gtk() {
        eprintln!("GTK 3 isn't installed (pkg-config gtk+-3.0); skipped");
        return;
    }
    let dir = tmpdir("ui-build");
    let mut compilers = vec!["gcc"];
    if have("clang") {
        compilers.push("clang");
    }
    for cc in compilers {
        for p in ui_programs() {
            let exe = dir.join("prog");
            let out = nexac().env("CC", cc).arg("build").arg(&p).arg("-o").arg(&exe).output().unwrap();
            assert!(out.status.success(), "[{}] {} didn't build:\n{}", cc, p.display(), String::from_utf8_lossy(&out.stderr));
        }
    }
    // a program without `ui` doesn't get GTK at all
    let exe = dir.join("hello");
    assert!(nexac().arg("build").arg(root().join("examples/hello.nx")).arg("-o").arg(&exe).status().unwrap().success());
    let deps = Command::new("ldd").arg(&exe).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    assert!(!deps.contains("gtk"), "hello is linked with GTK:\n{}", deps);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_window_without_a_display_is_an_error_not_a_crash() {
    if !have_gtk() {
        eprintln!("GTK 3 isn't installed; skipped");
        return;
    }
    let home = tmpdir("ui-nodisplay");
    let out = nexac()
        .arg("run")
        .arg("--check-memory")
        .arg(root().join("examples/ui/tasks.nx"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("HOME", &home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "error: can't open a window: there's no display to show it on (run it from the desktop)\n"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn windows_driven_by_the_keyboard_do_what_they_should_and_free_all_memory() {
    if !have_gtk() || !have_display_tools() || !have("python3") {
        eprintln!("GTK 3, Xvfb, xdotool or python3 missing; skipped");
        return;
    }
    let dir = tmpdir("ui-drive");
    for s in ui_steps() {
        let mut cmd: Vec<&std::ffi::OsStr> = vec![env!("CARGO_BIN_EXE_nexac").as_ref(), "run".as_ref(), "--check-memory".as_ref()];
        cmd.push(s.program.as_os_str());
        let out = drive(&s, &dir, &cmd, 30, None);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{} failed ({:?}):\n{}", s.file.display(), out.status.code(), stderr);
        assert_eq!(String::from_utf8_lossy(&out.stdout), std::fs::read_to_string(s.file.with_extension("out")).unwrap(), "{}", s.file.display());
        assert!(!stderr.contains("CRITICAL") && !stderr.contains("Gtk-WARNING"), "{}: GTK complained:\n{}", s.file.display(), stderr);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn windows_are_clean_under_address_and_undefined_sanitizers() {
    if !have_gtk() || !have_display_tools() || !have("python3") {
        eprintln!("GTK 3, Xvfb, xdotool or python3 missing; skipped");
        return;
    }
    let dir = tmpdir("ui-asan");
    for s in ui_steps() {
        let c = nexac().arg("emit-c").arg(&s.program).output().unwrap();
        assert!(c.status.success());
        let cfile = dir.join("prog.c");
        std::fs::write(&cfile, &c.stdout).unwrap();
        let exe = dir.join("prog-asan");
        let st = Command::new("gcc")
            .args(["-std=gnu11", "-O1", "-g", "-fsanitize=address,undefined", "-fno-sanitize-recover=all"])
            .args(["-fno-omit-frame-pointer", "-DNX_LEAK_CHECK", "-Wall", "-Wextra", "-Werror"])
            .args(gtk_flags("--cflags"))
            .arg(&cfile)
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .args(gtk_flags("--libs"))
            .status()
            .unwrap();
        assert!(st.success(), "{} didn't build with sanitizers", s.program.display());
        // leaks are judged below (only the program's own count), so they
        // don't change the exit code here; memory errors still do
        let env = [
            ("ASAN_OPTIONS", "detect_leaks=1:fast_unwind_on_malloc=0:malloc_context_size=60:abort_on_error=0"),
            ("LSAN_OPTIONS", "exitcode=0"),
        ];
        let out = drive_env(&s, &dir, &[exe.as_os_str()], 60, None, &env);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{} under sanitizers ({:?}):\n{}", s.file.display(), out.status.code(), stderr);
        assert!(!stderr.contains("ERROR: AddressSanitizer") && !stderr.contains("runtime error"), "{}:\n{}", s.file.display(), stderr);
        let mine = own_leaks(&stderr, &["Direct leak of", "Indirect leak of"], "prog.c");
        assert!(mine.is_empty(), "{}: leaks from the program's own code:\n{}", s.file.display(), mine.join("\n"));
        assert_eq!(String::from_utf8_lossy(&out.stdout), std::fs::read_to_string(s.file.with_extension("out")).unwrap(), "{}", s.file.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn windows_are_clean_under_valgrind() {
    if !have_gtk() || !have_display_tools() || !have("python3") || !have("valgrind") {
        eprintln!("GTK 3, Xvfb, xdotool, python3 or valgrind missing; skipped");
        return;
    }
    let dir = tmpdir("ui-vg");
    for s in ui_steps() {
        let exe = dir.join("prog-vg");
        assert!(nexac().arg("build").arg(&s.program).arg("-o").arg(&exe).status().unwrap().success());
        let log = dir.join("vg.log");
        let supp = format!("--suppressions={}", root().join("tests/ui/valgrind.supp").display());
        let logf = format!("--log-file={}", log.display());
        let args: Vec<&std::ffi::OsStr> = vec![
            "valgrind".as_ref(),
            "-q".as_ref(),
            "--error-exitcode=9".as_ref(),
            supp.as_ref(),
            logf.as_ref(),
            "--num-callers=50".as_ref(),
            "--leak-check=full".as_ref(),
            "--show-leak-kinds=definite".as_ref(),
            "--errors-for-leak-kinds=none".as_ref(),
            exe.as_os_str(),
        ];
        let out = drive(&s, &dir, &args, 180, None);
        let report = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(out.status.success(), "{} under valgrind ({:?}):\n{}\n{}", s.file.display(), out.status.code(), String::from_utf8_lossy(&out.stderr), report);
        let mine = own_leaks(&report, &["definitely lost"], &exe.display().to_string());
        assert!(mine.is_empty(), "{}: leaks from the program's own code:\n{}", s.file.display(), mine.join("\n"));
        assert_eq!(String::from_utf8_lossy(&out.stdout), std::fs::read_to_string(s.file.with_extension("out")).unwrap(), "{}", s.file.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn window_misuse_stops_with_a_message_in_the_programs_own_file() {
    if !have_gtk() || !have_display_tools() || !have("python3") {
        eprintln!("GTK 3, Xvfb, xdotool or python3 missing; skipped");
        return;
    }
    let dir = tmpdir("ui-trap");
    let empty = dir.join("none.steps");
    std::fs::write(&empty, "# program: none\n").unwrap();
    for p in files("tests/ui/trap") {
        let want = expectation(&p, "trap");
        let s = Steps { file: empty.clone(), program: p.clone(), broker: false };
        let cmd: Vec<&std::ffi::OsStr> = vec![env!("CARGO_BIN_EXE_nexac").as_ref(), "run".as_ref(), "--check-memory".as_ref(), p.as_os_str()];
        let out = drive(&s, &dir, &cmd, 30, None);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{}:\n{}", p.display(), stderr);
        let file = p.file_name().unwrap().to_string_lossy();
        assert!(stderr.contains(&format!("nexa: runtime error in {} at line", file)), "{}: not reported in its own file:\n{}", p.display(), stderr);
        assert!(stderr.contains(&want), "{}: expected `{}`, got:\n{}", p.display(), want, stderr);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn windows_follow_the_theme_live() {
    if !have_gtk() || !have_display_tools() || !have("python3") || !have("convert") {
        eprintln!("GTK 3, Xvfb, xdotool, python3 or ImageMagick missing; skipped");
        return;
    }
    let dir = tmpdir("ui-theme");
    let shots = dir.join("shots");
    let s = Steps { file: root().join("tests/ui/theme.steps"), program: root().join("tests/ui/widgets.nx"), broker: false };
    let cmd: Vec<&std::ffi::OsStr> = vec![env!("CARGO_BIN_EXE_nexac").as_ref(), "run".as_ref(), s.program.as_os_str()];
    let out = drive(&s, &dir, &cmd, 30, Some(&shots));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // the window's background, below the list: the theme's nexa_bg each time
    let pixel = |name: &str| -> String {
        let o = Command::new("convert").arg(shots.join(format!("{name}.png"))).args(["-format", "%[pixel:p{258,117}]", "info:"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).to_string()
    };
    assert_eq!(pixel("theme-start"), "srgb(26,42,26)", "the theme set at start (#1a2a1a)");
    assert_eq!(pixel("theme-changed"), "srgb(242,240,230)", "the theme after a live change (#f2f0e6)");
    // and the primary button follows the accent colour (#5ee6a8, then #c2185b)
    let accent = |name: &str| -> bool {
        let want = if name == "theme-start" { "srgb(94,230,168)" } else { "srgb(194,24,91)" };
        let o = Command::new("convert")
            .arg(shots.join(format!("{name}.png")))
            .args(["-crop", "560x640+0+0", "-format", "%c", "histogram:info:"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).contains(want)
    };
    assert!(accent("theme-start") && accent("theme-changed"), "the accent colour didn't follow the theme");
    let _ = std::fs::remove_dir_all(&dir);
}
