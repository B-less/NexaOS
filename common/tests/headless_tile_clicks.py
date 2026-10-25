"""App icons clicked for real, on the icon itself (not the name under it):
Nexus's hexagon tiles, the desktop "For you" card's apps and the Store's
app cards. A virtual mouse (common/tests/vpointer.c) clicks through the
compositor like a real one. The icon used to sit on a layer with its own
input surface, which took the click: only the name opened the app.
Run with scripts/test-tile-clicks.sh (headless Sway, nexa-apptest)."""
import json
import os
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
for sub in ("common", "shell", "store"):
    sys.path.insert(0, os.path.join(REPO, sub))

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

from nexa_common import titlebar  # noqa: E402
from nexa_shell import desktop, nexus  # noqa: E402
from nexa_store import window as store  # noqa: E402

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


def node():
    stack = [json.loads(subprocess.run(["swaymsg", "-t", "get_tree"], capture_output=True, text=True).stdout)]
    while stack:
        n = stack.pop()
        if n.get("pid") == os.getpid():
            return n
        stack.extend(n.get("nodes", []) + n.get("floating_nodes", []))
    return None


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


def click_on(win, bar, widget):
    """Click the middle of `widget` (the icon, not its button)."""
    rect = node()["rect"]
    wx, wy = widget.translate_coordinates(win, 0, 0)
    hx, hy = bar.translate_coordinates(win, 0, 0)
    a = widget.get_allocation()
    mouse(f"move {rect['x'] + wx - hx + a.width // 2} {rect['y'] + wy - hy + a.height // 2}")
    pump(0.2)
    mouse("click")
    pump(0.5)


clicked = []
win = Gtk.Window(title="Tile clicks")
bar = titlebar.attach_titlebar(win, "Tile clicks")
row = Gtk.Box(spacing=40)
row.set_border_width(30)
win.add(row)

colours = {"accent": (0.3, 0.64, 1.0), "surface": (0.1, 0.12, 0.2), "crit": (1, 0.36, 0.48), "text": (0.9, 0.9, 1)}
tile = nexus.HexTile({"name": "Files", "comment": "Files", "icon": "nexa-files"}, colours, lambda app: clicked.append("nexus"))
row.pack_start(tile, False, False, 0)

card_btn = Gtk.Button()
card_icon = desktop.HexIcon("nexa-files", (0.3, 0.64, 1.0))
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
box.pack_start(card_icon, False, False, 0)
box.pack_start(Gtk.Label(label="Files"), False, False, 0)
card_btn.add(box)
card_btn.connect("clicked", lambda *_a: clicked.append("for-you"))
row.pack_start(card_btn, False, False, 0)

store_btn = Gtk.Button()
store_icon = store.HexIcon(None, store.CARD_ICON)
sbox = Gtk.Box(spacing=12)
sbox.pack_start(store_icon, False, False, 0)
sbox.pack_start(Gtk.Label(label="Some app"), False, False, 0)
store_btn.add(sbox)
store_btn.connect("clicked", lambda *_a: clicked.append("store"))
row.pack_start(store_btn, False, False, 0)

win.show_all()
pump(1.0)
check("the test window is on screen", node() is not None)

overlay = tile.area.get_parent()
click_on(win, bar, overlay)
check("Nexus: clicking the app's icon opens it", clicked.count("nexus") == 1, str(clicked))
click_on(win, bar, card_icon)
check("desktop card: clicking the icon opens it", clicked.count("for-you") == 1, str(clicked))
click_on(win, bar, store_icon)
check("Store: clicking an app's icon opens its page", clicked.count("store") == 1, str(clicked))

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
