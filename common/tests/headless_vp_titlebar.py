"""The Video Player's (C) window buttons clicked for real with the virtual
mouse, the same way as headless_titlebar_clicks.py. Its title bar: 38 px high,
8 px padding, 26 px round buttons 6 px apart at the right end.
Run with scripts/test-titlebar.sh."""
import json
import os
import subprocess
import sys
import time

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def sway(*cmd):
    return subprocess.run(["swaymsg", *cmd], capture_output=True, text=True).stdout


def node(pid):
    stack = [(json.loads(sway("-t", "get_tree")), False)]
    while stack:
        n, scratch = stack.pop()
        scratch = scratch or n.get("name") == "__i3_scratch"
        if n.get("pid") == pid:
            return n, scratch
        stack.extend((c, scratch) for c in n.get("nodes", []) + n.get("floating_nodes", []))
    return None, False


vp = subprocess.Popen([os.environ.get("VPOINTER", "/tmp/vpointer")], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
vp.stdout.readline()


def mouse(cmd):
    vp.stdin.write(cmd + "\n")
    vp.stdin.flush()
    vp.stdout.readline()


def button_xy(rect, index):
    """Centre of the index-th button from the right (0 close, 1 maximize, 2 minimize)."""
    return rect["x"] + rect["width"] - 8 - 13 - index * (26 + 6), rect["y"] + 19


app = subprocess.Popen(["/src/apps/video-player/build/video-player"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
for _ in range(100):
    n, _s = node(app.pid)
    if n:
        break
    time.sleep(0.2)
time.sleep(1.0)
n, _s = node(app.pid)
if n is None:
    pids = [(w.get("pid"), w.get("app_id")) for w in json.loads(sway("-t", "get_tree")).get("nodes", [])]
    print("windows:", sway("-t", "get_tree").count('"pid"'), "app alive:", app.poll() is None, pids)
check("the Video Player opened", n is not None)
before = n["rect"]["width"], n["rect"]["height"]

x, y = button_xy(n["rect"], 0)
mouse(f"move {x} {y}")
time.sleep(0.4)
subprocess.run(["grim", "-g", f"{n['rect']['x']},{n['rect']['y']} {n['rect']['width']}x40", os.path.join(os.environ.get("OUT", "/tmp"), "vp-titlebar-close-hover.png")])

x, y = button_xy(n["rect"], 1)
mouse(f"move {x} {y}")
mouse("click")
time.sleep(0.8)
n, _s = node(app.pid)
out = json.loads(sway("-t", "get_outputs"))[0]["rect"]
check("Maximize fills the screen", n["rect"]["width"] >= out["width"] - 4, str(n["rect"]))
x, y = button_xy(n["rect"], 1)
mouse(f"move {x} {y}")
mouse("click")
time.sleep(0.8)
n, _s = node(app.pid)
check("...and back to the same size", (n["rect"]["width"], n["rect"]["height"]) == before, f"{n['rect']} vs {before}")

x, y = button_xy(n["rect"], 2)
mouse(f"move {x} {y}")
mouse("click")
time.sleep(0.8)
_n, scratch = node(app.pid)
check("Minimize hides it", scratch)

sway(f"[pid={app.pid}] scratchpad show")
time.sleep(0.8)
n, scratch = node(app.pid)
x, y = button_xy(n["rect"], 0)
mouse(f"move {x} {y}")
mouse("click")
try:
    app.wait(timeout=5)
    closed = True
except subprocess.TimeoutExpired:
    closed = False
    app.kill()
check("Close quits it", closed)

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
