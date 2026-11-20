"""The Windows nexac must be the same compiler as the Linux one (0.9).

    python3 tests/windows/differential.py LINUX_NEXAC WINDOWS_NEXAC [MUTANTS [SEED]]

WINDOWS_NEXAC is a program that runs nexac.exe (for example a two-line
script running it under Wine). For every .nx file in the repository
(examples, tests, apps) both must give byte-for-byte the same:

  - `nexac check --json` (and exit code) for the file,
  - `nexac emit-c` (the generated C, which decides what the program does),
  - `nexac fmt --stdin` (output and exit code), also for the file with
    Windows line ends (`\\r\\n`),
  - `nexac explain --json` for each mistake's message;

and for MUTANTS mutated programs (tests/fuzz/fuzz.py's mutations) the
same `check --json --stdin`, `fmt --stdin` and `explain --json`. Paths are
given as Linux paths; under Wine they're on drive Z:. The one expected
difference is a full path printed in a message (`\\` between folders on
Windows), which is compared with `\\` turned into `/` and reported apart.
Test-only; not part of NexaOS.
"""
import concurrent.futures as cf
import importlib.util, json, os, random, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
LINUX = os.path.abspath(sys.argv[1])
WINDOWS = os.path.abspath(sys.argv[2])
MUTANTS = int(sys.argv[3]) if len(sys.argv) > 3 else 0
SEED = int(sys.argv[4]) if len(sys.argv) > 4 else 1

spec = importlib.util.spec_from_file_location("fuzz", os.path.join(ROOT, "tests", "fuzz", "fuzz.py"))
fuzz = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fuzz)


def run(exe, args, stdin=b""):
    r = subprocess.run([exe] + args, input=stdin, capture_output=True, timeout=120)
    return (r.returncode, r.stdout, r.stderr)


def files():
    out = []
    for dirpath, dirnames, names in os.walk(ROOT):
        dirnames[:] = sorted(d for d in dirnames if d not in ("target", "node_modules", "findings"))
        out += [os.path.join(dirpath, n) for n in sorted(names) if n.endswith(".nx")]
    apps = os.path.join(ROOT, "..", "apps")
    for n in sorted(os.listdir(apps)):
        p = os.path.join(apps, n, "src")
        if os.path.isdir(p):
            out += [os.path.join(p, f) for f in sorted(os.listdir(p)) if f.endswith(".nx")]
    return out


def slashes(b):
    return b.replace(b"\\\\", b"/").replace(b"\\", b"/")


def compare(what, cmd, stdin=b""):
    """('same' | 'paths' | 'DIFFERENT', what, details)"""
    a = run(LINUX, cmd, stdin)
    b = run(WINDOWS, cmd, stdin)
    if a == b:
        return ("same", what, a)
    if a[0] == b[0] and slashes(a[1]) == slashes(b[1]) and slashes(a[2]) == slashes(b[2]):
        return ("paths", what, a)
    return ("DIFFERENT", what, f"{cmd}\nlinux:   {a}\nwindows: {b}")


def messages(check_stdout):
    try:
        return [d["message"] for d in json.loads(check_stdout)["diagnostics"]]
    except Exception:
        return []


def one_file(path):
    res = []
    r = compare(path, ["check", "--json", path])
    res.append(r)
    res.append(compare(path, ["emit-c", path]))
    text = open(path, "rb").read()
    res.append(compare(path, ["fmt", "--stdin"], text))
    res.append(compare(path + " (\\r\\n)", ["fmt", "--stdin"], text.replace(b"\n", b"\r\n")))
    for m in messages(run(LINUX, ["check", "--json", path])[1])[:2]:
        res.append(compare(path + " explain", ["explain", "--json", m]))
    return res


def one_mutant(job):
    n, path, text = job
    data = text.encode("utf-8")
    res = [compare(f"mutant {n} of {path}", ["check", "--json", "--stdin", path], data)]
    res.append(compare(f"mutant {n} fmt", ["fmt", "--stdin"], data))
    for m in messages(run(LINUX, ["check", "--json", "--stdin", path], data)[1])[:1]:
        res.append(compare(f"mutant {n} explain", ["explain", "--json", m]))
    return res


def main():
    tally = {"same": 0, "paths": 0, "DIFFERENT": 0}
    bad = []
    paths = []
    fs = files()
    rng = random.Random(SEED)
    seeds = fuzz.seeds()
    jobs = []
    for n in range(MUTANTS):
        p, t = rng.choice(seeds)
        jobs.append((n, p, fuzz.mutate(rng, t)))
    with cf.ThreadPoolExecutor(max_workers=os.cpu_count() or 2) as pool:
        for res in list(pool.map(one_file, fs)) + list(pool.map(one_mutant, jobs)):
            for kind, what, detail in res:
                tally[kind] += 1
                if kind == "DIFFERENT":
                    bad.append(detail)
                elif kind == "paths":
                    paths.append(what)
    print(f"{len(fs)} files, {MUTANTS} mutants (seed {SEED}): {tally['same']} identical, "
          f"{tally['paths']} differing only in a path's \\, {tally['DIFFERENT']} different")
    for w in paths[:20]:
        print("  path only:", w)
    for d in bad[:20]:
        print("DIFFERENT:", d)
    sys.exit(1 if bad else 0)


main()
