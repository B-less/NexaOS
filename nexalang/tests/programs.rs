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
    for p in files("examples") {
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
