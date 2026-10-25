"""nexa_common.exits: the desktop writes down apps that end abnormally
(for Why), from real child processes: a crash, a kill, an error code, a
normal close (not written), and the bounded file."""
import json
import os
import signal
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import gi  # noqa: E402
gi.require_version("Gio", "2.0")
from gi.repository import GLib  # noqa: E402
from nexa_common import brain, desktopapps, exits  # noqa: E402

failures = 0


def check(what, ok):
    global failures
    print(("PASS " if ok else "FAIL ") + what)
    failures += not ok


home = tempfile.mkdtemp()
os.environ["HOME"] = home
os.environ["XDG_STATE_HOME"] = os.path.join(home, "state")


def entries():
    try:
        with open(exits.path()) as f:
            return [json.loads(line) for line in f]
    except OSError:
        return []


def run_until(n, timeout=5):
    loop = GLib.MainLoop()
    GLib.timeout_add(50, lambda: (len(entries()) >= n and loop.quit()) or True)
    GLib.timeout_add(timeout * 1000, loop.quit)
    loop.run()


# A crash (SIGSEGV), an error code at start, and a normal close.
desktopapps.launch("sh -c 'kill -SEGV $$'")
desktopapps.launch("sh -c 'exit 3'")
desktopapps.launch("true")
run_until(2)
GLib.MainLoop().get_context().iteration(False)
e = entries()
check("a crash is written down with its signal", any(x.get("signal") == signal.SIGSEGV and x["app"] == "sh" for x in e))
check("an error code is written down", any(x.get("code") == 3 for x in e))
check("a normal close is not", len(e) == 2)

# Killed while running (as the kernel does when memory runs out).
p = exits.watch(subprocess.Popen(["sleep", "30"], start_new_session=True), "nexa-notes")
os.kill(p.pid, signal.SIGKILL)
run_until(3)
check("a kill is written down for the right app", any(x.get("signal") == 9 and x["app"] == "nexa-notes" for x in entries()))
check("the Popen learns how it ended (no zombie, no warning)", p.returncode == -9)

# Started from Nexus/workspaces (brain.launch, through the .desktop file).
apps = os.path.join(home, ".local/share/applications")
os.makedirs(apps)
with open(os.path.join(apps, "nexa-crashy.desktop"), "w") as f:
    f.write("[Desktop Entry]\nType=Application\nName=Crashy\nExec=sh -c 'kill -ABRT $$'\n")
brain.launch("nexa-crashy")
run_until(4)
check("an app started by its .desktop file is followed too", any(x["app"] == "nexa-crashy" and x.get("signal") == 6 for x in entries()))

# Bounded: never more than a few hundred lines.
for i in range(exits.KEEP * 3):
    exits.record("x", i, 9, 1.0, now=1)
with open(exits.path()) as f:
    n = len(f.readlines())
check(f"the record stays bounded ({n} lines)", n <= exits.KEEP * 2)
check("only this user can read it", os.stat(os.path.dirname(exits.path())).st_mode & 0o077 == 0)
sys.exit(1 if failures else 0)
