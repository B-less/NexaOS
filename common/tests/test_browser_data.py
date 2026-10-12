"""Run with: python3 common/tests/test_browser_data.py

The browser's history store and download helpers (no GTK, no WebKit)."""
import os, stat, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(HERE)), "browser"))
from nexa_browser import downloads, history, tor

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

d = tempfile.mkdtemp()
path = os.path.join(d, "sub", "history.json")
h = history.History(path)
h.add("https://alpinelinux.org/", "Alpine Linux", now=100)
h.add("https://duckduckgo.com/?q=nexa", "nexa at DuckDuckGo", now=200)
h.add("https://alpinelinux.org/", "", now=300)
h.add("about:blank", now=400)
h.add("file:///home/nexa/secret.pdf", now=500)
check("history: survives a restart", history.History(path).entries == h.entries)
check("history: file is private to the user", stat.S_IMODE(os.stat(path).st_mode) == 0o600)
check("history: about:/file: pages not recorded", len(h.entries) == 2, str(h.entries))
check("history: revisit counts, keeps the title", h.entries["https://alpinelinux.org/"] == {"title": "Alpine Linux", "visits": 2, "last": 300})
check("history: recent is newest first", [r["url"] for r in h.recent()] == ["https://alpinelinux.org/", "https://duckduckgo.com/?q=nexa"])
check("history: search matches title words", [r["url"] for r in h.search("alpine LINUX")] == ["https://alpinelinux.org/"])
check("history: search matches URL", [r["url"] for r in h.search("duckduckgo")] == ["https://duckduckgo.com/?q=nexa"])
check("history: empty search finds nothing", h.search("  ") == [])
h.set_title("https://duckduckgo.com/?q=nexa", "Results")
check("history: title updates are saved", history.History(path).entries["https://duckduckgo.com/?q=nexa"]["title"] == "Results")
h.clear()
check("history: clear empties the file", history.History(path).entries == {})
open(path, "w").write("{broken")
check("history: corrupt file starts fresh instead of crashing", history.History(path).entries == {})
old = history.MAX_ENTRIES; history.MAX_ENTRIES = 3
h = history.History(path)
for i in range(5):
    h.add(f"https://e{i}.example/", now=i)
check("history: capped, oldest dropped", sorted(h.entries) == ["https://e2.example/", "https://e3.example/", "https://e4.example/"])
history.MAX_ENTRIES = old

check("download name: path parts stripped", downloads.safe_name("../../etc/passwd") == "passwd")
check("download name: no hidden files", downloads.safe_name(".bashrc") == "bashrc")
check("download name: windows paths stripped", downloads.safe_name("C:\\x\\a.zip") == "a.zip")
check("download name: never empty", downloads.safe_name("") == "download" and downloads.safe_name(None) == "download")
open(os.path.join(d, "a.zip"), "w").close()
open(os.path.join(d, "a (2).zip"), "w").close()
open(os.path.join(d, "x.tar.gz"), "w").close()
check("download path: numbered when taken", downloads.unique_path(d, "a.zip") == os.path.join(d, "a (3).zip"))
check("download path: .tar.gz kept together", downloads.unique_path(d, "x.tar.gz") == os.path.join(d, "x (2).tar.gz"))
check("download path: free name used as is", downloads.unique_path(d, "new.iso") == os.path.join(d, "new.iso"))

t = [0.0]
m = downloads.SpeedMeter(window=3.0, clock=lambda: t[0])
check("speed: zero before two samples", (m.update(0), m.rate())[1] == 0.0)
for sec, total in ((1, 1_000_000), (2, 2_000_000), (3, 3_000_000), (4, 4_000_000), (5, 4_000_000)):
    t[0] = sec; m.update(total)
check("speed: averages over the last 3 s", abs(m.rate() - 2_000_000 / 3) < 1, str(m.rate()))
check("status: size, speed, time left",
      downloads.status_line(3 * 1024 ** 2, 10 * 1024 ** 2, 1024 ** 2) == "3.0 MB of 10.0 MB · 1.0 MB/s · 7 s left",
      downloads.status_line(3 * 1024 ** 2, 10 * 1024 ** 2, 1024 ** 2))
check("status: unknown size", downloads.status_line(2048, 0, 0) == "2.0 KB")
check("time: minutes", downloads.human_time(125) == "2 min 5 s")

log = os.path.join(d, "tor.log")
check("tor: no log yet -> 0%", tor.progress(log) == (0, ""))
open(log, "w").write("Oct 12 [notice] Bootstrapped 0% (starting): Starting\n"
                     "Oct 12 [notice] Bootstrapped 45% (requesting_descriptors): Asking for relay descriptors\n"
                     "Oct 12 [notice] Bootstrapped 100% (done): Done\n")
check("tor: newest bootstrap line wins", tor.progress(log) == (100, "Done"), str(tor.progress(log)))
cmd = tor.command()
check("tor: SOCKS on loopback only, Tor Browser's port",
      cmd[cmd.index("--SocksPort") + 1] == "127.0.0.1:9150" and tor.PROXY_URI == "socks5://127.0.0.1:9150")
check("tor: client only, no system torrc", "--ClientOnly" in cmd and cmd[cmd.index("--defaults-torrc") + 1] == tor.torrc_path())
os.environ["XDG_RUNTIME_DIR"] = d
check("tor: not running without a pid file", not tor.running())
os.makedirs(tor.runtime_dir())
open(tor.pid_path(), "w").write(str(os.getpid()))
check("tor: a pid that isn't tor doesn't count", not tor.running())

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
