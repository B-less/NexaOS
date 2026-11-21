"""Open With can't end up behind the file manager, and the pointer leaving
a window doesn't take its focus (click to focus), in a real compositor.

Open With used to be a dialog window. GTK 3 can't tell Hyprland or Sway
that a dialog is modal, so clicking the file manager raised it over the
dialog, while GTK, blocking the click for the dialog, left the file
manager seemingly stuck. Open With is now a popover over the file: part of
the file manager's own window.

1. Open With opens no window of its own, only a popover in the file
   manager's window.
2. Clicking the file manager beside it closes it: nothing is left behind,
   and the file manager takes clicks again.
3. Clicking Open launches the app that's picked, with the file.
4. A window clicked keeps the keyboard when the pointer moves over another
   window (focus_follows_mouse no).

Run with scripts/test-files-open-with.sh (headless Sway, nexa-apptest)."""
import json
import os
import subprocess
import sys
import tempfile
import time

HOME = tempfile.mkdtemp(prefix="files-openwith-")
os.environ["HOME"] = HOME
os.environ["XDG_DATA_HOME"] = os.path.join(HOME, ".local/share")
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "file-manager"), os.path.join(REPO, "common")]

# one app that opens text, so Open With has something to list
apps = os.path.join(HOME, ".local/share/applications")
os.makedirs(apps)
with open(os.path.join(apps, "test-viewer.desktop"), "w") as f:
    f.write("[Desktop Entry]\nType=Application\nName=Test Viewer\nExec=true %f\nMimeType=text/plain;\n")
NOTE = os.path.join(HOME, "note.txt")
with open(NOTE, "w") as f:
    f.write("hello\n")

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

from nexa_files import window as W  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def pump(seconds=0.2):
    end = time.time() + seconds
    while time.time() < end:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        time.sleep(0.01)


_vp = subprocess.Popen([os.environ.get("VPOINTER", "/tmp/vpointer")], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
_vp.stdout.readline()


def mouse(cmd):
    _vp.stdin.write(cmd + "\n")
    _vp.stdin.flush()
    end = time.time() + 5
    while time.time() < end:
        pump(0.02)
        if _vp.stdout.readline():
            return


def click(x, y):
    mouse(f"move {int(x)} {int(y)}")
    mouse("press")
    mouse("release")
    pump(0.5)


def windows():
    """name -> sway node, for every window on screen"""
    tree = json.loads(subprocess.run(["swaymsg", "-t", "get_tree"], capture_output=True, text=True).stdout)
    found, stack = {}, [tree]
    while stack:
        n = stack.pop()
        for f in n.get("floating_nodes", []):
            if f.get("name"):
                found[f["name"]] = f
        stack.extend(n.get("nodes", []) + n.get("floating_nodes", []))
    return found


def on_screen(win, widget, x, y):
    """A point of `widget` (its own coordinates) on the screen."""
    rect = windows()["Files"]["rect"]
    wx, wy = widget.translate_coordinates(win, int(x), int(y))
    bx, by = win.get_titlebar().translate_coordinates(win, 0, 0)
    return rect["x"] + wx - bx, rect["y"] + wy - by


def find(widget, kind, label=None):
    if isinstance(widget, kind) and (label is None or widget.get_label() == label):
        return widget
    if isinstance(widget, Gtk.Container):
        for c in widget.get_children():
            hit = find(c, kind, label)
            if hit is not None:
                return hit
    return None


win = W.FileManagerWindow(HOME)
win._error = lambda *a, **k: None
launched = []
win._launch_with = lambda app, gfile, ct, d: launched.append((app.get_display_name(), gfile.get_path()))
win.show_all()
pump(1.5)


def select_note():
    row = next(r for r in win.store if r[W.COL_PATH] == NOTE)
    win._unselect_all()
    (win.grid.select_path if win.is_grid else win.tree.get_selection().select_path)(row.path)
    pump(0.3)


# 1. a popover in the window, no window of its own
select_note()
before = sorted(windows())
win._on_open_with(None)
pump(1.0)
pop = getattr(win, "_open_with_popover", None)
check("Open With shows over the file", pop is not None and pop.get_visible())
check("Open With opens no window of its own", sorted(windows()) == before, f"before {before}, now {sorted(windows())}")
check("it lists the app that opens text", pop is not None and find(pop, Gtk.Label, "Test Viewer") is not None)

# 2. a click on the file manager beside it closes it; nothing stays behind
if pop is not None:
    # a corner of the file list (inside the window, not in its shadow)
    # that the popover doesn't cover
    a = pop.get_allocation()
    px, py = pop.translate_coordinates(win.view, 0, 0)
    v = win.view.get_allocation()
    corners = [(12, v.height - 12), (v.width - 12, v.height - 12), (12, 12), (v.width - 12, 12)]
    spot = next(c for c in corners if not (px <= c[0] < px + a.width and py <= c[1] < py + a.height))
    click(*on_screen(win, win.view, *spot))
    pump(0.8)
    check("a click on the file manager closes Open With", not pop.get_visible())
    check("nothing is left on screen but the file manager", sorted(windows()) == before, str(sorted(windows())))
    check("the file manager has the focus", windows()["Files"].get("focused"))

# 3. Open launches the app picked, with the file
select_note()
win._on_open_with(None)
pump(1.0)
pop = win._open_with_popover
btn = find(pop, Gtk.Button, "Open")
r = btn.get_allocation()
click(*on_screen(win, btn, r.width / 2, r.height / 2))
pump(0.8)
check("Open launches the app with the file", launched == [("Test Viewer", NOTE)], str(launched))
check("and Open With closes", not pop.get_visible())

# 4. click to focus: the pointer passing over another window changes nothing
other = Gtk.Window(title="Other")
other.set_default_size(300, 200)
other.show_all()
pump(1.5)
w = windows()
f, o = w["Files"]["rect"], w["Other"]["rect"]
spots = [(f["x"] + 15, f["y"] + f["height"] - 15), (f["x"] + f["width"] - 15, f["y"] + f["height"] - 15)]
x, y = next(s for s in spots if not (o["x"] <= s[0] < o["x"] + o["width"] and o["y"] <= s[1] < o["y"] + o["height"]))
click(x, y)
check("clicking the file manager focuses it", windows()["Files"].get("focused"))
mouse(f"move {o['x'] + o['width'] // 2} {o['y'] + o['height'] // 2}")
pump(0.8)
w = windows()
check("the pointer over another window leaves the focus where it was",
      w["Files"].get("focused") and not w["Other"].get("focused"),
      "focused: " + str([name for name, n in w.items() if n.get("focused")]))

_vp.stdin.close()
print("ALL PASS" if not failures else f"{len(failures)} FAILED")
sys.exit(1 if failures else 0)
