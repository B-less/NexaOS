"""Settings > Magic Layer driven like a person would, with the real
renderer (nexa-living) running: every control changes magic.json and the
renderer follows (its measured numbers show on the page); per-app styles,
"apply to the ticked apps", the style preview, Restore the defaults.
Run with scripts/test-magic-page.sh. Screenshots go to $OUT."""
import json
import os
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
for sub in ("common", "settings-app"):
    sys.path.insert(0, os.path.join(REPO, sub))

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

from nexa_common import magic  # noqa: E402
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


def saved():
    try:
        with open(magic.path()) as f:
            return json.load(f)["living"]
    except (OSError, ValueError, KeyError):
        return {}


def stats():
    return magic.stats() or {}


win = SW.SettingsWindow()
win.show_all()
win._on_nav(None, "Magic Layer")
pump(1)
p = win.magic_page
check("Settings has a Magic Layer page", win.stack.get_visible_child_name() == "Magic Layer")

p.quality.set_active_id("static")
pump(0.3)
check("Quality: Static is saved", saved().get("quality") == "static", saved())
check("...and the renderer follows", pump(8, lambda: stats().get("quality") == "static"), stats())

p.desktop.set_active_id("nebula")
pump(0.3)
check("desktop style saved", saved().get("desktop") == "nebula")
check("a preview of the chosen style appears", pump(6, lambda: p.preview.get_pixbuf() is not None))

p.sliders["speed"].set_value(1.6)
pump(0.1)
check("sliders wait until they stop moving", saved().get("speed") in (None, 1.0))
pump(0.6)
check("...then save", abs(saved().get("speed", 0) - 1.6) < 0.01, saved().get("speed"))

p.app_combos["nexa-settings"].set_active_id("rain")
pump(0.3)
check("an app's own style is saved", saved()["apps"]["nexa-settings"] == "rain")
p.app_checks["nexa-files"].set_active(True)
p.app_checks["nexa-calc"].set_active(True)
p.many_style.set_active_id("off")
p._on_apply_many(None)
pump(0.3)
apps = saved()["apps"]
check("Apply gives every ticked app the style", apps["nexa-files"] == "off" and apps["nexa-calc"] == "off", apps)
check("...and the rows show it", p.app_combos["nexa-files"].get_active_id() == "off")

p.quality.set_active_id("balanced")
pump(0.5)
check("the measured line shows real numbers", pump(12, lambda: "frames a second" in p.measured.get_text()), p.measured.get_text())
subprocess.run(["grim", os.path.join(OUT, "magic-page.png")], check=False)
print("MEASURE page:", p.measured.get_text(), flush=True)

p.enabled.set_active(False)
pump(0.3)
check("the main switch turns it off", saved().get("enabled") is False)
check("...and the renderer stops drawing", pump(8, lambda: stats().get("enabled") == 0), stats())
p._on_reset(None)
pump(0.3)
lv = saved()
check("Restore the defaults", lv.get("enabled") is True and lv.get("quality") == "auto" and lv["apps"]["nexa-files"] == "circuit")
check("...and the controls show them", p.enabled.get_active() and p.quality.get_active_id() == "auto"
      and p.app_combos["nexa-files"].get_active_id() == "circuit")
# A Magic Layer feature that is built has a real switch (Why).
check("Why has a switch, on by default", "why" in p.feature_switches and p.feature_switches["why"].get_active())
p.feature_switches["why"].set_active(False)
pump(1, lambda: False)
check("...that turns it off in magic.json (MicroBrain reads it)",
      magic.load()["features"]["why"]["enabled"] is False)
p.feature_switches["why"].set_active(True)
sys.exit(1 if failures else 0)
