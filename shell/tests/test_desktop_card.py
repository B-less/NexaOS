"""The desktop "For you" card and Nexus' Suggested row in a headless Sway,
against the real nexa-patternd holding a synthetic month of use.

Run with scripts/test-desktop-card.sh (the nexa-apptest container)."""
import json
import os
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "shell"), os.path.join(REPO, "common")]
HOME = tempfile.mkdtemp(prefix="card-home-")
os.environ["HOME"] = HOME
os.environ["XDG_CONFIG_HOME"] = os.path.join(HOME, ".config")
# A fixed Tuesday, 09:10 local: the synthetic month's weekday work hours.
os.environ["NEXA_PATTERNS_NOW"] = str(int(time.mktime((2026, 10, 20, 9, 10, 0, 0, 0, -1))))

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gtk, GtkLayerShell  # noqa: E402

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


def shot(name):
    out = os.environ.get("OUT")
    if out:
        subprocess.run(["grim", os.path.join(out, name)], check=False)


apps_dir = "/usr/local/share/applications"
os.makedirs(apps_dir, exist_ok=True)
names = {"nexa-mail": ("Mail", "mail-unread"), "nexa-browser": ("Nexa Browser", "web-browser"),
         "foot": ("Terminal", "utilities-terminal"), "nexa-files": ("Files", "system-file-manager"),
         "nexa-edit": ("Editor", "accessories-text-editor"), "nexa-music": ("Music", "audio-x-generic"),
         "video-player": ("Video Player", "video-x-generic"), "nexa-calc": ("Calculator", "accessories-calculator"),
         "nexa-notes": ("Notes", "text-x-generic"), "nexa-pulse": ("Nexa Pulse", "utilities-system-monitor"),
         "nexa-settings": ("Settings", "preferences-system"), "nexa-viewer": ("Viewer", "image-x-generic")}
for app_id, (name, icon) in names.items():
    open(os.path.join(apps_dir, app_id + ".desktop"), "w").write(
        f"[Desktop Entry]\nType=Application\nName={name}\nExec=true\nIcon={icon}\n")

daemon = os.environ.get("NEXA_PATTERND", os.path.join(REPO, "build", "native", "nexa-patternd"))
data = os.path.join(HOME, ".local/share/nexa-patterns")
os.makedirs(data)
subprocess.run([daemon, "synthetic", "30", os.path.join(data, "journal.bin")], check=True)
os.makedirs(os.path.join(HOME, ".config/nexa"))
json.dump({"enabled": True}, open(os.path.join(HOME, ".config/nexa/patterns.json"), "w"))
proc = subprocess.Popen([daemon, "serve"], stderr=subprocess.DEVNULL)
for _ in range(100):
    if os.path.exists(os.path.join(os.environ["XDG_RUNTIME_DIR"], "nexa-patterns.sock")):
        break
    time.sleep(0.05)

from nexa_common import patterns  # noqa: E402
from nexa_shell import desktop  # noqa: E402

try:
    expected = patterns.suggestions("card")["apps"]
    card = desktop.DesktopCard()
    pump(1.0)
    buttons = [b for b in card.card.get_children()[1].get_children()] if len(card.card.get_children()) > 1 else []
    check("the card is on the desktop", card.get_visible() and card.get_mapped())
    check("on the bottom layer: above the wallpaper, under every window",
          GtkLayerShell.get_layer(card) == GtkLayerShell.Layer.BOTTOM)
    check("it never takes the keyboard", GtkLayerShell.get_keyboard_mode(card) == GtkLayerShell.KeyboardMode.NONE)
    check("one button per suggestion (1-3)", 1 <= len(buttons) == len(expected) <= 3, f"{len(buttons)} vs {len(expected)}")
    check("each says why when hovered", all(b.get_tooltip_text() and b.get_tooltip_text().startswith("You") for b in buttons),
          str([b.get_tooltip_text() for b in buttons]))
    check("top-right by default", GtkLayerShell.get_anchor(card, GtkLayerShell.Edge.TOP)
          and GtkLayerShell.get_anchor(card, GtkLayerShell.Edge.RIGHT))
    shot("card-desktop.png")

    first = buttons[0].suggestion
    card.dont_suggest(first)
    pump(0.5)
    left = [b.suggestion["app"] for b in card.card.get_children()[1].get_children()] if len(card.card.get_children()) > 1 else []
    check("Don't suggest removes it from the card (the rest stay)", first["app"] not in left and left, str(left))
    patterns.never_suggest(first["app"], False)

    card.move_to("bottom-right")
    pump(0.3)
    check("Move the card: bottom-right, saved", GtkLayerShell.get_anchor(card, GtkLayerShell.Edge.BOTTOM)
          and not GtkLayerShell.get_anchor(card, GtkLayerShell.Edge.TOP)
          and patterns.load_config()["card_position"] == "bottom-right")

    card.hide_today()
    pump(0.3)
    check("Hide for today: the card goes away", not card.get_visible())
    patterns.update_config(hidden_until=0, card_position="top-right")
    card.refresh()
    pump(0.5)
    check("...and comes back when allowed", card.get_visible())

    patterns.update_config(enabled=False)
    card.refresh()
    pump(0.3)
    check("Patterns off: no card at all", not card.get_visible())
    patterns.update_config(enabled=True)
    card.refresh()
    pump(0.5)

    # A normal window opened over the desktop covers the card.
    win = Gtk.Window(title="A window")
    win.set_default_size(1280, 760)
    win.add(Gtk.Label(label="An app window: the card is underneath"))
    win.show_all()
    pump(1.0)
    shot("card-under-window.png")
    win.destroy()
    pump(0.3)

    # Nexus shows the same guesses as a Suggested row.
    from nexa_shell import nexus
    app = Gtk.Application(application_id="org.nexaos.test.nexus")
    app.register(None)
    nx = nexus.Nexus(app)
    labels = []

    def walk(w):
        if isinstance(w, Gtk.Label):
            labels.append(w.get_text())
        if isinstance(w, Gtk.Container):
            w.foreach(walk)
    walk(nx.content)
    check("Nexus has a 'Suggested for you' row", "Suggested for you" in labels, str(labels[:6]))
    nx.show_all()
    pump(1.0)
    shot("nexus-suggested.png")
    nx.destroy()
finally:
    proc.terminate()
    proc.wait()

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
