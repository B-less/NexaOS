"""Nexus in a headless Sway: tile highlighting follows the pointer (and
keyboard focus only while navigating with the keyboard), Restart / Shut
down need a second click, and search finds every Settings page.
Run with scripts/test-desktop-card.sh (the nexa-apptest container)."""
import os
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "shell"), os.path.join(REPO, "common")]

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, Gtk  # noqa: E402

from nexa_shell import nexus, search  # noqa: E402

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


apps_dir = "/usr/local/share/applications"
os.makedirs(apps_dir, exist_ok=True)
for app_id, name in (("nexa-files", "Files"), ("nexa-notes", "Notes"), ("nexa-calc", "Calculator")):
    open(os.path.join(apps_dir, app_id + ".desktop"), "w").write(
        f"[Desktop Entry]\nType=Application\nName={name}\nExec=true\nIcon=text-x-generic\n")

called = []
nexus.power = lambda action: called.append(action)

app = Gtk.Application(application_id="org.nexaos.test.nexushover")
app.register(None)
nx = nexus.Nexus(app)
nx.show_all()
pump(0.6)
tiles = []


def walk(w):
    if isinstance(w, nexus.HexTile):
        tiles.append(w)
    if isinstance(w, Gtk.Container):
        w.foreach(walk)


walk(nx.content)
check("the honeycomb has tiles", len(tiles) >= 2, str(len(tiles)))
a, b = tiles[0], tiles[1]


def crossing(tile, kind, detail=Gdk.NotifyType.NONLINEAR):
    ev = Gdk.Event.new(kind)
    ev.detail = detail
    ev.window = tile.get_window()
    tile.event(ev)
    pump(0.05)


crossing(a, Gdk.EventType.ENTER_NOTIFY)
check("pointer over a tile lights it", a._hot() and not b._hot())
crossing(a, Gdk.EventType.LEAVE_NOTIFY, Gdk.NotifyType.INFERIOR)
check("...moving onto its own icon keeps it lit", a._hot())
crossing(a, Gdk.EventType.LEAVE_NOTIFY)
check("...leaving turns it off (no stuck highlight)", not a._hot())
b.grab_focus()
nx.set_focus_visible(False)
pump(0.05)
check("a focused tile isn't lit while using the mouse", not b._hot())
nx.set_focus_visible(True)
check("...but is while moving around with the keyboard", b._hot())
crossing(a, Gdk.EventType.ENTER_NOTIFY)
nx.set_focus_visible(False)
check("only the tile under the pointer is lit", a._hot() and not b._hot())

# Restart needs two clicks.
power_buttons = []


def find_power(w):
    if isinstance(w, Gtk.Button) and w.get_name() == "nx-power":
        power_buttons.append(w)
    if isinstance(w, Gtk.Container):
        w.foreach(find_power)


find_power(nx.body)
restart = next(btn for btn in power_buttons if btn.get_tooltip_text() == "Restart")
restart.clicked()
pump(0.05)
labels = []
restart.foreach(lambda box: box.foreach(lambda w: labels.append(w.get_text()) if isinstance(w, Gtk.Label) else None))
check("one click on Restart only asks to confirm", called == [] and "Click again" in labels, str((called, labels)))
restart.clicked()
check("...the second click restarts", called == ["reboot"], str(called))

for query, page in (("privacy", "privacy"), ("users", "users"), ("updates", "updates"), ("startup", "startup"), ("camera permission", "privacy")):
    found = [r for r in search.search([], query) if r["kind"] == "setting"]
    check(f"search '{query}' finds Settings > {page.title()}", any(r["command"] == ["nexa-settings", page] for r in found), str(found))

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
