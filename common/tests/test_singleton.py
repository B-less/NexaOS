"""Single-instance handoff: a second launch gives its files to the running
one instead of starting another copy. Run with: python3 common/tests/test_singleton.py"""
import os
import socket
import sys
import tempfile

os.environ["XDG_RUNTIME_DIR"] = tempfile.mkdtemp()
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

try:
    import gi  # noqa: F401
    from gi.repository import GLib
except (ImportError, ValueError):
    print("  - skipping test_singleton.py (no PyGObject/GLib)")
    sys.exit(0)

from nexa_common import singleton

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)

# 1. The first launch becomes the one instance and starts watching (as an
# app does once its window exists); a later launch hands its file over.
first = singleton.claim("test-app", ["one.txt"])
check("first launch is the instance", isinstance(first, socket.socket))

got = []
loop = GLib.MainLoop()
def on_open(paths):
    got.extend(paths)
    loop.quit()
singleton.serve(first, on_open)

second = singleton.claim("test-app", ["two.txt"])  # a second launch
check("second launch hands off (returns None)", second is None)
GLib.timeout_add(4000, loop.quit)  # safety net
loop.run()
check("the running instance receives the handed-over file",
      got == [os.path.abspath("two.txt")], str(got))

# 2. A stale socket (crashed instance, nobody listening) is taken over, not fatal.
first.close()
singleton._cleanup(singleton._addr("test-app"))
stale = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
stale.bind(singleton._addr("stale-app"))  # bound but never listening/accepting
stale.close()  # leaves the socket file behind, nobody answering
revived = singleton.claim("stale-app", ["x.txt"])
check("a stale socket is taken over, not a dead end", isinstance(revived, socket.socket))
if isinstance(revived, socket.socket):
    revived.close()

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
