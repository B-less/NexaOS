"""Settings > Display, Sound, Mouse & Touchpad, Wi-Fi, Notifications,
Default Apps, Accessibility and Time & Keyboard, opened like a person
would in a headless Sway: the Display page changes the real screen through
nexa-display (keep and put back), the other pages write what the session
reads. Run with scripts/test-settings-devices.sh. Screenshots go to $OUT."""
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

from nexa_common import displays  # noqa: E402
from nexa_settings import window as SW  # noqa: E402

OUT = os.environ.get("OUT", "/tmp")
HOME = os.environ["HOME"]
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
    subprocess.run(["grim", os.path.join(OUT, name + ".png")], check=False)


def conf(*parts):
    try:
        with open(os.path.join(HOME, ".config", *parts)) as f:
            return f.read()
    except OSError:
        return ""


win = SW.SettingsWindow()
win.set_default_size(1100, 760)
win.show_all()
pump(1.0)

# ---- the sidebar has every new page ----
for name in ("Display", "Sound", "Mouse & Touchpad", "Wi-Fi", "Notifications", "Default Apps", "Accessibility"):
    check(f"sidebar: {name}", name in win.nav_buttons and win.stack.get_child_by_name(name) is not None)

# ---- Display: the real screen, through nexa-display ----
win._on_nav(None, "Display")
page = win.device_pages["Display"]
pump(6, until=lambda: page.screens)
check("Display lists the screen", [s["name"] for s in page.screens] == ["HEADLESS-1"], str(page.screens))
w = page.widgets.get("HEADLESS-1", {})
check("...with resolution, refresh, scale and rotation", {"res", "rate", "scale", "rot"} <= set(w), str(w.keys()))
pump(0.3)
shot("settings-display")
w["scale"].set_active_id("1.5")
page.keep_dialog = lambda: True
page.apply()
pump(8, until=lambda: abs(displays.screens()[0]["scale"] - 1.5) < 0.01 and os.path.exists(displays.saved_path()))
check("Apply + Keep: the screen is at 150 % now", abs(displays.screens()[0]["scale"] - 1.5) < 0.01,
      f"{page.status.get_text()} {displays.screens()[0]['scale']}")
check("...and saved for the next sign-in", json.loads(conf("nexa", "displays.json"))["heads"][0]["scale"] == 1.5)
pump(1, until=lambda: page.widgets.get("HEADLESS-1"))
page.widgets["HEADLESS-1"]["rot"].set_active_id("90")
page.keep_dialog = lambda: False
page.apply()
pump(8, until=lambda: "Put back" in page.status.get_text())
pump(3, until=lambda: displays.screens()[0]["transform"] == "normal")
check("Apply + Put back (or the 15 s running out): as it was", displays.screens()[0]["transform"] == "normal"
      and abs(displays.screens()[0]["scale"] - 1.5) < 0.01, str(displays.screens()))
page.night_on.set_active(True)
page.night_from.set_text("21:00")
page.save_night()
check("night light saved", "enabled=1" in conf("nexa", "nightlight.conf") and "from=21:00" in conf("nexa", "nightlight.conf"))

# ---- Sound: no sound server in the test box: says so, doesn't crash ----
win._on_nav(None, "Sound")
pump(4, until=lambda: win.device_pages["Sound"].status.get_text())
check("Sound without a sound server explains itself", "isn't available" in win.device_pages["Sound"].status.get_text(),
      win.device_pages["Sound"].status.get_text())

# ---- Mouse & Touchpad ----
win._on_nav(None, "Mouse & Touchpad")
inp = win.device_pages["Mouse & Touchpad"]
inp.set(tap_to_click=False, left_handed=True)
check("touchpad and mouse choices written for Sway", "tap disabled" in conf("sway", "config.d", "30-nexa-input.conf")
      and "left_handed enabled" in conf("sway", "config.d", "30-nexa-input.conf"))
check("...and for Hyprland", "tap-to-click = false" in conf("hypr", "nexa-input.conf"))
pump(0.3)
shot("settings-input")

# ---- Wi-Fi: no iwd here ----
win._on_nav(None, "Wi-Fi")
pump(4, until=lambda: win.device_pages["Wi-Fi"].status.get_text())
check("Wi-Fi without iwd explains itself", "isn't available" in win.device_pages["Wi-Fi"].status.get_text(),
      win.device_pages["Wi-Fi"].status.get_text())

# ---- Notifications ----
win._on_nav(None, "Notifications")
nt = win.device_pages["Notifications"]
nt.set(position="bottom-right", timeout=10)
nt.mute("Chat", True)
saved = json.loads(conf("nexa", "notifications.json"))
check("notifications: position, time and a muted app are saved for Signals",
      saved == {"timeout": 10, "position": "bottom-right", "muted_apps": ["Chat"]}, str(saved))
pump(0.3)
shot("settings-notifications")

# ---- Default Apps ----
win._on_nav(None, "Default Apps")
pump(0.3)
check("Default Apps builds (with whatever apps are installed)", win.device_pages["Default Apps"].page.get_children())

# ---- Accessibility ----
win._on_nav(None, "Accessibility")
acc = win.device_pages["Accessibility"]
acc.set(text_scale=1.3, cursor_size=32)
pump(6, until=lambda: "Applied" in acc.status.get_text() or "Couldn't" in acc.status.get_text())
check("bigger text and pointer reach GTK's settings", "Noto Sans 13" in conf("gtk-3.0", "settings.ini")
      and "gtk-cursor-theme-size=32" in conf("gtk-3.0", "settings.ini"), acc.status.get_text())
check("...and Sway's pointer", "Adwaita 32" in conf("sway", "config.d", "31-nexa-access.conf"))
acc.set(high_contrast=True)
pump(6, until=lambda: "Applied" in acc.status.get_text())
check("High contrast switches to the High Contrast theme", json.loads(conf("nexa", "theme.json"))["theme"] == "contrast")
acc.set(high_contrast=False)
pump(6, until=lambda: json.loads(conf("nexa", "theme.json"))["theme"] != "contrast")
check("...and off puts the previous theme back", json.loads(conf("nexa", "theme.json"))["theme"] == "horizon",
      conf("nexa", "theme.json"))
pump(0.3)
shot("settings-accessibility")

# ---- Time & Keyboard: 12-hour clock ----
win._on_nav(None, "Time & Keyboard")
win.clock24_switch.set_active(False)
pump(0.3)
check("12-hour clock saved for the top bar", conf("nexa", "clock").strip() == "12h")
check("the manual time is there and usable while automatic time is off",
      win.set_time_btn.get_sensitive() == (not win.ntp_switch.get_active()))

# ---- Overview: Display and Sound objects ----
win._on_nav(None, "Overview")
win.field.refresh()
cell = win.field.cells.get("display")
pump(8, until=lambda: cell is not None and "night light" in cell.summary)
check("Overview: a Display object with the screen's size", cell is not None and "1280 × 800" in cell.summary
      and "150 %" in cell.summary and "night light" in cell.summary, getattr(cell, "summary", None))
check("Overview: Network opens the Wi-Fi page now", SW.LivingField and __import__("nexa_settings.field", fromlist=["BY_KEY"]).BY_KEY["network"][4] == "Wi-Fi")
pump(0.3)
shot("settings-overview")
win.destroy()
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
