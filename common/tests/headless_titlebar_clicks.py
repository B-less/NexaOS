"""The window buttons (minimize, maximize, close) clicked for real: a
virtual mouse (common/tests/vpointer.c, the wlr virtual-pointer protocol)
moves onto each button and clicks, so the click goes through the
compositor exactly like a real mouse - not a synthetic GTK signal. Run with scripts/test-titlebar.sh (headless Sway in
the nexa-apptest container)."""
import json
import os
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO, "common"))

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

from nexa_common import titlebar  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def pump(seconds=0.3):
    end = time.time() + seconds
    while time.time() < end:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        time.sleep(0.01)


def sway(*cmd):
    return subprocess.run(["swaymsg", *cmd], capture_output=True, text=True).stdout


def node():
    """This window's node in sway's tree, and whether it's in the scratchpad."""
    stack = [(json.loads(sway("-t", "get_tree")), False)]
    while stack:
        n, scratch = stack.pop()
        scratch = scratch or n.get("name") == "__i3_scratch"
        if n.get("pid") == os.getpid():
            return n, scratch
        stack.extend((c, scratch) for c in n.get("nodes", []) + n.get("floating_nodes", []))
    return None, False


def click(win, bar, btn, button="click"):
    """Press and release `button` with sway's pointer on the middle of `btn`."""
    n, _ = node()
    rect = n["rect"]
    bx, by = btn.translate_coordinates(win, 0, 0)
    hx, hy = bar.translate_coordinates(win, 0, 0)
    alloc = btn.get_allocation()
    x = rect["x"] + (bx - hx) + alloc.width // 2
    y = rect["y"] + (by - hy) + alloc.height // 2
    mouse(f"move {x} {y}")
    pump(0.15)
    mouse(button)
    pump(0.6)


# The virtual mouse (common/tests/vpointer.c), held for the whole test.
_vp = subprocess.Popen([os.environ.get("VPOINTER", "/tmp/vpointer")], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
print("virtual mouse:", _vp.stdout.readline().strip())


def mouse(cmd):
    _vp.stdin.write(cmd + "\n")
    _vp.stdin.flush()
    end = time.time() + 5
    while time.time() < end:
        pump(0.02)
        line = _vp.stdout.readline()
        if line:
            return line.strip()
    return None


closed = []
win = Gtk.Window(title="Titlebar test")
win.set_default_size(480, 320)
bar = titlebar.attach_titlebar(win, "Titlebar test")
win.add(Gtk.Label(label="content"))
win.connect("delete-event", lambda *_a: closed.append(True) or True)  # keep it open to count
win.show_all()
pump(1.0)
n, scratch = node()
check("the window is on screen", n is not None and not scratch, str(n and n.get("rect")))
buttons = {b.get_tooltip_text(): b for b in bar.get_children() if isinstance(b, Gtk.Button)}
check("it has the three buttons", set(buttons) == {"Minimize", "Maximize", "Close"}, str(list(buttons)))
drawn = {name for name, b in buttons.items() if b.get_mapped() and b.get_child() and b.get_child().get_mapped()}
check("...all three drawn (not an empty button)", drawn == {"Minimize", "Maximize", "Close"}, str(drawn))

before = n["rect"]["width"], n["rect"]["height"]
click(win, bar, buttons["Maximize"])
n, _ = node()
out = json.loads(sway("-t", "get_outputs"))[0]["rect"]
check("Maximize fills the screen", n["rect"]["width"] >= out["width"] - 4, f"{n['rect']} on {out}")
click(win, bar, buttons["Maximize"])
n, _ = node()
check("...and a second click puts it back", (n["rect"]["width"], n["rect"]["height"]) == before, f"{n['rect']} vs {before}")

click(win, bar, buttons["Close"])
check("Close asks the window to close", closed == [True], str(closed))

click(win, bar, buttons["Minimize"])
_, scratch = node()
check("Minimize hides it (the Dock brings it back)", scratch)

# A fixed-size window (the Calculator) offers no Maximize: it only moved.
fixed = Gtk.Window(title="Fixed")
fixed_bar = titlebar.attach_titlebar(fixed, "Fixed")
fixed.set_resizable(False)
fixed.add(Gtk.Label(label="fixed"))
fixed.show_all()
pump(0.5)
shown = {b.get_tooltip_text() for b in fixed_bar.get_children() if isinstance(b, Gtk.Button) and b.get_visible()}
check("a fixed-size window has Minimize and Close, no Maximize", shown == {"Minimize", "Close"}, str(shown))

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
