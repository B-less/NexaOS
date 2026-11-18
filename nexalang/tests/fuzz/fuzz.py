"""Fuzz nexac: mutate real NexaLang programs, compile each mutant, and for
every one the compiler accepts, build it with the exact hardened flags
(`nexac build`, -O2 -Werror) and run a sanitizer build of it (ASan + UBSan
+ the NexaLang leak counter). Test-only; not part of NexaOS.

    python3 tests/fuzz/fuzz.py NEXAC COUNT [SEED [--tools-only]]

A finding is any of: nexac crashing (a Rust panic, a signal), the C
compiler failing on generated code, AddressSanitizer or UBSan reporting
anything, or memory left over when a program ends normally. Window
programs (`use ui`) run on a private headless X server (Xvfb) and are
stopped after a moment (they wait for a person). Findings are kept in
tests/fuzz/findings/ (not committed) and summarised at the end.

Every mutant, accepted or not, also goes through 0.6's tools:
`nexac check --json` (valid JSON, read by Python's own json; every
mistake has a topic; the same mistakes as the text output), `nexac fmt`
(no crash; formatting twice changes nothing; the formatted program gives
the same generated C, or the same mistake), `nexac explain` on each
message, and a language server session (`nexac lsp`: the same
diagnostics, the same formatting, hover, a clean shutdown).
"""
import concurrent.futures as cf
import json, os, random, re, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
NEXAC = os.path.abspath(sys.argv[1])
COUNT = int(sys.argv[2])
SEED = int(sys.argv[3]) if len(sys.argv) > 3 else 1
# --tools-only: only 0.6's tools, no C builds (much faster)
TOOLS_ONLY = "--tools-only" in sys.argv[4:]
FINDINGS = os.path.join(ROOT, "tests", "fuzz", "findings")

def gtk(which):
    r = subprocess.run(["pkg-config", which, "gtk+-3.0"], capture_output=True, text=True)
    return r.stdout.split() if r.returncode == 0 else None

GTK_CFLAGS, GTK_LIBS = gtk("--cflags"), gtk("--libs")

def seeds():
    out = []
    for d in ["examples", "examples/ui", "tests/ui", "tests/trap", "tests/modules/app", "tests/broker", "tests/json", "tests/stress", "tests/main_err"]:
        full = os.path.join(ROOT, d)
        for n in sorted(os.listdir(full)):
            if n.endswith(".nx"):
                p = os.path.join(full, n)
                out.append((p, open(p, encoding="utf-8").read()))
    # NexaOS apps written in NexaLang (0.5)
    apps = os.path.join(ROOT, "..", "apps")
    for n in sorted(os.listdir(apps)) if os.path.isdir(apps) else []:
        p = os.path.join(apps, n, "src", "main.nx")
        if os.path.isfile(p):
            out.append((p, open(p, encoding="utf-8").read()))
    return out

TOKENS = ["0", "1", "-1", "9223372036854775807", "-9223372036854775808", "0.5", "1e308", '""', '"x"', '"\\{"',
          "true", "false", "none", "some(1)", "ok()", 'err("e")', "[]", "[1, 2]", "len(xs)", "x", "y", "i",
          "+", "-", "*", "/", "%", "==", "<", "and", "or", "not", "(", ")", "[", "]", "{", "}", ",", ":", "?",
          ".", "..", "=", "+=", "let", "var", "fn", "return", "if", "else", "while", "for", "in", "match",
          "break", "continue", "=>", "_", "struct", "enum", "pub", "use", "\n", "ui.wait()", "ui.label(1, \"t\")",
          "ui.set_items(2, [\"a\"])", "ui.select(2, 0)", "ui.status(\"s\")", "ui.focus(1)", "ui.get_text(1)",
          "ui.button(1, \"b\")", "ui.list(1, \"e\")", "ui.wait_ms(1)", "broker.no_args()", "json.parse(\"[1]\")",
          "files.size(\"x\")", "files.size(\"\")", "ui.display(1, \"9:00\")",
          # 0.7: removing widgets, code, the clipboard (file choosers wait for a person: stopped like a window)
          "ui.clear(1)", "ui.remove(2)", "ui.remove(1)", "ui.code(1, \"x\\n\")", "ui.to_clipboard(\"c\")",
          "ui.focus(2)", "ui.save_file(\"t\", \"a.nx\")", "ui.open_file(\"t\", \"*.nx\")"]

def mutate(rng, src):
    lines = src.split("\n")
    for _ in range(rng.randint(1, 3)):
        k = rng.randrange(11)
        if not lines:
            break
        i = rng.randrange(len(lines))
        if k == 8:
            # layout only (for nexac fmt): spaces and tabs around a line
            lines[i] = rng.choice(["", " ", "\t", "        "]) + lines[i].strip() + rng.choice(["", " ", "\t"])
            continue
        if k == 9:
            # a comment after the code, or on its own line
            if rng.random() < 0.5:
                lines[i] += rng.choice(["  // c", "// é 😀", " //", "\t// x {y}"])
            else:
                lines.insert(i, rng.choice(["// own", "    // own", ""]))
            continue
        if k == 10:
            # empty lines, and a line split after an operator or a comma
            if rng.random() < 0.5:
                lines.insert(i, "\n" * rng.randint(1, 3))
            else:
                lines[i] = re.sub(r"([,(+]) ", lambda m: m.group(1) + "\n", lines[i], count=1)
            continue
        if k == 0:
            del lines[i]
        elif k == 1:
            lines.insert(rng.randrange(len(lines) + 1), lines[i])
        elif k == 2:
            j = rng.randrange(len(lines))
            lines[i], lines[j] = lines[j], lines[i]
        else:
            words = re.findall(r'"(?:[^"\\]|\\.)*"|\w+|[^\s\w]', lines[i])
            if not words:
                continue
            w = rng.randrange(len(words))
            if k in (3, 4):
                words[w] = rng.choice(TOKENS)
            elif k == 5:
                del words[w]
            elif k == 6:
                words.insert(w, rng.choice(TOKENS))
            else:
                other = re.findall(r"\b[a-z_]\w*\b", src)
                if other:
                    words[w] = rng.choice(other)
            lines[i] = " ".join(words)
    return "\n".join(lines)

def run(cmd, **kw):
    try:
        return subprocess.run(cmd, capture_output=True, timeout=kw.pop("timeout", 60), **kw)
    except subprocess.TimeoutExpired:
        return None

# what the tools saw (threads only add to these)
import collections, threading
STATS = collections.Counter()
STATS_LOCK = threading.Lock()

def lsp_session(prog, text):
    """One editor session; returns (diagnostics, formatting edits) or raises."""
    p = subprocess.Popen([NEXAC, "lsp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    def send(obj):
        body = json.dumps(obj).encode()
        p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        p.stdin.flush()
    def recv():
        n = None
        while True:
            line = p.stdout.readline()
            if not line:
                raise RuntimeError("the server stopped")
            if line in (b"\r\n", b"\n"):
                break
            if line.lower().startswith(b"content-length:"):
                n = int(line.split(b":")[1])
        return json.loads(p.stdout.read(n))
    try:
        uri = "file://" + prog
        send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        recv()
        send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
              "params": {"textDocument": {"uri": uri, "languageId": "nexalang", "version": 1, "text": text}}})
        diags = recv()["params"]["diagnostics"]
        send({"jsonrpc": "2.0", "id": 2, "method": "textDocument/formatting", "params": {"textDocument": {"uri": uri}}})
        edits = recv()["result"]
        for d in diags[:3]:
            send({"jsonrpc": "2.0", "id": 3, "method": "textDocument/hover",
                  "params": {"textDocument": {"uri": uri}, "position": d["range"]["start"]}})
            h = recv()
            if "error" in h:
                raise RuntimeError("hover failed: %r" % h)
        send({"jsonrpc": "2.0", "id": 4, "method": "shutdown"})
        recv()
        send({"jsonrpc": "2.0", "method": "exit"})
        p.stdin.close()
        if p.wait(timeout=30) != 0:
            raise RuntimeError("exit status %r" % p.returncode)
        err = p.stderr.read().decode(errors="replace")
        if "panicked" in err:
            raise RuntimeError(err)
        return diags, edits
    finally:
        if p.poll() is None:
            p.kill()
            p.wait()

def tools(prog, text, work):
    """0.6's tools on one mutant; returns a finding's text, or None."""
    c = run([NEXAC, "check", "--json", prog])
    if c is None:
        return "check --json hangs"
    if c.returncode not in (0, 1) or b"panicked" in c.stderr:
        return "check --json crashed: " + c.stderr.decode(errors="replace")
    try:
        report = json.loads(c.stdout.decode())
    except Exception as e:
        return "check --json printed bad JSON (%s): %r" % (e, c.stdout[:2000])
    diags = report["diagnostics"]
    if (c.returncode == 0) != report["ok"] or report["ok"] != (not diags):
        return "check --json: ok, exit status and diagnostics disagree"
    text_err = run([NEXAC, "check", prog]).stderr.decode(errors="replace")
    for d in diags:
        if d["topic"] == "other":
            return "a message without a topic: " + d["message"]
        if d["line"] and ("error: %s\n  --> %s:%d:%d\n" % (d["message"], d["file"], d["line"], d["column"])) not in text_err:
            return "check --json and check disagree: %r\n%s" % (d, text_err)
        e = run([NEXAC, "explain", d["message"]])
        if e is None or e.returncode != 0:
            return "explain refused its own message: " + d["message"]
    f = run([NEXAC, "fmt", "--stdin"], input=text.encode())
    if f is None:
        return "fmt hangs"
    if f.returncode not in (0, 1) or b"panicked" in f.stderr:
        return "fmt crashed: " + f.stderr.decode(errors="replace")
    formatted = f.stdout.decode() if f.returncode == 0 else None
    if formatted is not None:
        again = run([NEXAC, "fmt", "--stdin"], input=f.stdout)
        if again is None or again.returncode != 0 or again.stdout != f.stdout:
            return "fmt isn't idempotent:\n" + formatted
        before = run([NEXAC, "emit-c", "--no-lines", prog])
        open(prog, "w", encoding="utf-8").write(formatted)
        after = run([NEXAC, "emit-c", "--no-lines", prog])
        open(prog, "w", encoding="utf-8").write(text)
        if before.returncode != after.returncode:
            return "fmt changed whether the program is accepted:\n" + formatted
        if before.returncode == 0 and before.stdout != after.stdout:
            return "fmt changed the generated C:\n" + formatted
        if before.returncode != 0 and before.stderr.split(b"\n")[0] != after.stderr.split(b"\n")[0]:
            return "fmt changed the mistake:\n%s\n%s" % (before.stderr[:300], after.stderr[:300])
    try:
        ldiags, edits = lsp_session(prog, text)
    except Exception as e:
        return "lsp: %s" % e
    if len(ldiags) != len(diags):
        return "lsp and check --json found different mistakes: %d vs %d" % (len(ldiags), len(diags))
    if formatted is not None and formatted != text and (len(edits) != 1 or edits[0]["newText"] != formatted):
        return "lsp formatting differs from fmt"
    if (formatted is None or formatted == text) and edits:
        return "lsp formatting edited a file it shouldn't"
    with STATS_LOCK:
        STATS["checked"] += 1
        STATS["accepted"] += report["ok"]
        STATS["formatted"] += formatted is not None
        STATS["layout changed"] += formatted is not None and formatted != text
        STATS["diagnostics"] += len(diags)
    return None

def one(job):
    n, path, text = job
    work = tempfile.mkdtemp(prefix="nxfuzz-")
    try:
        # keep the mutant next to its siblings so `use helper` still finds them
        sib = os.path.dirname(path)
        for f in os.listdir(sib):
            if f.endswith(".nx"):
                shutil.copy(os.path.join(sib, f), work)
        prog = os.path.join(work, os.path.basename(path))
        open(prog, "w", encoding="utf-8").write(text)
        problem = tools(prog, text, work)
        if problem:
            return ("tool-finding", n, path, text, problem)
        if TOOLS_ONLY:
            return ("tools-ok", n, path, text, "")
        c = run([NEXAC, "emit-c", prog])
        if c is None:
            return ("compiler-hang", n, path, text, "")
        err = c.stderr.decode(errors="replace")
        if c.returncode not in (0, 1) or "panicked" in err:
            return ("compiler-crash", n, path, text, err)
        if c.returncode != 0:
            return ("rejected", n, path, text, "")
        uses_ui = "#include <gtk/gtk.h>" in c.stdout.decode(errors="replace")
        if uses_ui and not GTK_CFLAGS:
            return ("skipped", n, path, text, "")
        exe = os.path.join(work, "prog")
        b = run([NEXAC, "build", prog, "-o", exe], timeout=180)
        if b is None or b.returncode != 0:
            return ("c-build-failure", n, path, text, b.stderr.decode(errors="replace") if b else "timeout")
        cfile = os.path.join(work, "prog.c")
        open(cfile, "wb").write(c.stdout)
        asan = os.path.join(work, "prog-asan")
        flags = ["gcc", "-std=gnu11", "-O1", "-g", "-fsanitize=address,undefined", "-fno-sanitize-recover=all",
                 "-fno-omit-frame-pointer", "-DNX_LEAK_CHECK", "-Wall", "-Wextra", "-Werror"]
        if uses_ui:
            flags += GTK_CFLAGS
        flags += [cfile, "-o", asan, "-lm"] + (GTK_LIBS if uses_ui else [])
        s = run(flags, timeout=180)
        if s is None or s.returncode != 0:
            return ("c-build-failure", n, path, text, (s.stderr.decode(errors="replace") if s else "timeout") + " (sanitizer build)")
        base = os.path.splitext(path)[0]
        stdin = open(base + ".in", "rb").read() if os.path.exists(base + ".in") else b""
        args = open(base + ".args").read().split() if os.path.exists(base + ".args") else []
        args = [a.replace("{tmp}", os.path.join(work, "t")) for a in args]
        env = dict(os.environ, HOME=work, NEXA_BROKER_SOCKET=os.path.join(work, "no-broker.sock"),
                   ASAN_OPTIONS="detect_leaks=1:abort_on_error=0", LSAN_OPTIONS="exitcode=0")
        cmd = [asan] + args
        if uses_ui:
            env["NO_AT_BRIDGE"] = "1"
            env["GDK_BACKEND"] = "x11"
            cmd = ["xvfb-run", "-a", "-s", "-nolisten tcp", "timeout", "-s", "TERM", "4"] + cmd
        r = run(cmd, input=stdin, env=env, timeout=30)
        ran = "ran-window" if uses_ui else "ran"
        if r is None:
            return (ran, n, path, text, "")  # endless loop: not a memory finding
        out = r.stderr.decode(errors="replace")
        trapped = "nexa: runtime error in" in out or r.returncode in (124, 143, -15)
        if "ERROR: AddressSanitizer" in out or re.search(r"\.c:\d+:\d+: runtime error: ", out):
            return ("memory-error", n, path, text, out[-4000:])
        if not trapped and ("nexa: leak check" in out or ("LeakSanitizer" in out and "prog.c" in out and not uses_ui)):
            return ("leak", n, path, text, out[-4000:])
        return (ran, n, path, text, "")
    finally:
        shutil.rmtree(work, ignore_errors=True)

def main():
    rng = random.Random(SEED)
    src = seeds()
    jobs = []
    for n in range(COUNT):
        path, text = rng.choice(src)
        jobs.append((n, path, mutate(rng, text)))
    tally = {}
    found = []
    with cf.ThreadPoolExecutor(max_workers=os.cpu_count() or 2) as pool:
        for res in pool.map(one, jobs):
            kind = res[0]
            tally[kind] = tally.get(kind, 0) + 1
            if kind not in ("rejected", "ran", "ran-window", "skipped", "tools-ok"):
                found.append(res)
    os.makedirs(FINDINGS, exist_ok=True)
    for kind, n, path, text, err in found:
        base = os.path.join(FINDINGS, f"{kind}-{n}")
        open(base + ".nx", "w", encoding="utf-8").write(text)
        open(base + ".txt", "w", encoding="utf-8").write(f"from {path}\n{err}")
    print(f"seed {SEED}: {COUNT} mutants: " + ", ".join(f"{k} {v}" for k, v in sorted(tally.items())))
    print("  tools: " + ", ".join(f"{k} {v}" for k, v in sorted(STATS.items())))
    if found:
        print(f"{len(found)} finding(s) in {FINDINGS}")
        sys.exit(1)

main()
