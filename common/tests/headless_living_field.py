"""The Living Field (Settings' front page) driven like a person would, with
the real MicroBrain, Nexa Link and Nexa Doctor running: every object shows
a live summary and opens with real controls; arrow keys move by position;
search dims what doesn't match; pin adds a cluster without moving the
rest; the Performance panel changes MicroBrain's mode. Run with
scripts/test-living-field.sh. Screenshots go to $OUT."""
import os
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
for sub in ("common", "settings-app"):
    sys.path.insert(0, os.path.join(REPO, sub))

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, Gtk  # noqa: E402

from nexa_common import brain  # noqa: E402
from nexa_settings import field as F  # noqa: E402
from nexa_settings import window as SW  # noqa: E402

OUT = os.environ.get("OUT", "/tmp")
failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""), flush=True)
    if not cond:
        failures.append(label)


def pump(seconds=0.2, until=None):
    end = time.time() + seconds
    while True:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        if until and until():
            return True
        if time.time() > end:
            return bool(until and until())
        time.sleep(0.02)


def shot(name):
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def key(widget, keyval):
    ev = Gdk.Event.new(Gdk.EventType.KEY_PRESS)
    ev.keyval, ev.window = keyval, widget.get_window()
    ev.set_device(Gdk.Display.get_default().get_default_seat().get_keyboard())
    return widget.emit("key-press-event", ev)


win = SW.SettingsWindow()
win.show_all()
f = win.field
check("Settings opens on the Living Field", win.stack.get_visible_child_name() == "Overview")
pump(8, until=lambda: all(c.summary != "…" for c in f.cells.values()))
check("all 15 objects are there", len(f.cells) == 15, str(list(f.cells)))
check("every object shows a live summary", all(c.summary != "…" for c in f.cells.values()),
      str({k: c.summary for k, c in f.cells.items() if c.summary == "…"}))
perf = f.cells["performance"].summary
check("Performance comes from MicroBrain (GB used and the mode)", " GB used" in perf and "·" in perf, perf)
check("each says its state with a shape too", all(c.shape.get_text() in F.SHAPES.values() for c in f.cells.values()))
check("screen readers get name, summary and state", f.cells["storage"].get_accessible().get_name().startswith("Storage: "))
shot("1-field.png")

empty = []
for k in f.cells:
    f.open(k)
    pump(0.3)
    kids = [w for w in f.panel.get_children() if w.get_visible()]
    if len(kids) < 3 or not f.panel.get_mapped():
        empty.append(k)
check("every object opens in place, on screen, with content", not empty, str(empty))

f.open("performance")
pump(0.5)
shot("2-performance-open.png")
combo = next((w for w in f.panel.get_children() if isinstance(w, Gtk.ComboBoxText)), None)
check("Performance offers the resource modes", combo is not None and combo.get_model().iter_n_children(None) == 5)
if combo:
    combo.set_active_id("battery-saver")
    pump(3, until=lambda: brain.request("mode").get("mode") == "battery-saver")
    check("choosing a mode there changes MicroBrain's", brain.request("mode").get("mode") == "battery-saver")
    combo.set_active_id("auto")
    pump(2)

f.close_panel()
f.cells["performance"].grab_focus()
pump(0.2)
key(f, Gdk.KEY_Right)
pump(0.2)
check("Right arrow moves to the next cluster (Connections)", f.selected in ("network", "bluetooth", "nexa"), str(f.selected))
key(f, Gdk.KEY_Down)
pump(0.2)
check("Down arrow moves down that cluster", f.selected in ("bluetooth", "nexa"), str(f.selected))
key(f, Gdk.KEY_Return)
pump(0.2)

f.search.set_text("wifi")
pump(0.3)
lit = [k for k, c in f.cells.items() if not c.dim]
check("search keeps what matches lit (“wifi” → Network)", lit == ["network"], str(lit))
f.search.set_text("")
pump(0.2)

before = {k: c.get_parent() for k, c in f.cells.items()}
f.open("storage")
pump(0.2)
pin = next(w for w in f.panel.get_children() if isinstance(w, Gtk.Box) and any(
    isinstance(b, Gtk.Button) and b.get_label() == "Pin to the start" for b in w.get_children()))
next(b for b in pin.get_children() if isinstance(b, Gtk.Button) and b.get_label() == "Pin to the start").clicked()
pump(0.5)
check("pinning puts it in a Pinned cluster", F.load_prefs()["pinned"] == ["storage"])
heads = [c.get_child().get_children()[0].get_text() for c in f.grid.get_children()]
check("...first, and the other clusters keep their order", heads[0] == "PINNED" and heads[1:] == ["YOUR SYSTEM", "CONNECTIONS", "YOU", "WORK", "SAFETY"], str(heads))
shot("3-pinned.png")

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
