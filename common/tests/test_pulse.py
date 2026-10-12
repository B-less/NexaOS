"""Run with: python3 common/tests/test_pulse.py

Nexa Pulse's /proc readers against a fake /proc tree, plus one live read."""
import os, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(HERE)), "pulse"))
from nexa_pulse import stats

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

# ---- live /proc: whatever the host is, the readings must be sane
idle, total = stats.cpu_times()
check("live: cpu_times idle <= total", 0 <= idle <= total)
pct, used, tot = stats.memory()
check("live: memory percent in range", 0 <= pct <= 100 and used <= tot)
check("live: this process is listed", any(p["pid"] == os.getpid() for p in stats.processes()))

# ---- fake /proc
root = tempfile.mkdtemp()
def write(rel, text):
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "w").write(text)

write("stat", "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 100 0 50 800 50 0 0 0 0 0\n")
write("meminfo", "MemTotal:       8000000 kB\nMemFree:  1000000 kB\nMemAvailable:   6000000 kB\n")
write("net/dev", "Inter-|   Receive\n face |bytes ...\n"
      "    lo: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0\n"
      "  eth0: 1000 10 0 0 0 0 0 0 200 5 0 0 0 0 0 0\n"
      " wlan0: 3000 10 0 0 0 0 0 0 800 5 0 0 0 0 0 0\n")
# A name with spaces and parentheses must not shift the fields.
write("4242/stat", "4242 (Web Content (x)) S 1 4242 4242 0 -1 4194560 0 0 0 0 70 30 0 0 20 0 1 0\n")
write("4242/status", "Name:\tWeb Content\nVmRSS:\t  20480 kB\n")
write("77/stat", "77 (gone")  # half-written: skipped, not a crash
write("self/stat", "not a pid dir")
stats.PROC = root

check("cpu_times: idle includes iowait", stats.cpu_times() == (850, 1000), str(stats.cpu_times()))
check("cpu_percent: half busy", abs(stats.cpu_percent((0, 0), (500, 1000)) - 50) < 1e-9)
check("cpu_percent: no time passed -> no crash", stats.cpu_percent((5, 10), (5, 10)) == 0.0)
check("memory: used = total - available", stats.memory() == (25.0, 1953, 7812), str(stats.memory()))
check("net: loopback excluded, interfaces summed", stats.net_bytes() == (4000, 1000), str(stats.net_bytes()))
procs = stats.processes()
check("processes: odd names parsed, broken entries skipped",
      [(p["pid"], p["name"], p["jiffies"], p["rss_kb"]) for p in procs] == [(4242, "Web Content (x)", 100, 20480)],
      str(procs))
cpu = stats.process_cpu({4242: 50}, procs, 200)
check("process_cpu: share of whole machine", abs(cpu[4242] - 25.0) < 1e-9, str(cpu))
check("process_cpu: new process starts at 0", stats.process_cpu({}, procs, 200)[4242] == 0.0)

write("passwd", "root:x:0:0::/root:/bin/sh\nnexa:x:1000:1000::/home/nexa:/bin/sh\n")
check("usernames: by uid", stats.usernames(os.path.join(root, "passwd")) == {0: "root", 1000: "nexa"})
check("human_bytes", (stats.human_bytes(512), stats.human_bytes(1536), stats.human_bytes(5 * 1024 ** 3))
      == ("512 B", "1.5 KB", "5.0 GB"))

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
