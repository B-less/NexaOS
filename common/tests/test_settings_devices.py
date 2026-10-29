"""The helpers behind Settings > Display, Sound, Mouse & Touchpad,
Notifications and Accessibility: what they write and how they read
(no GTK, no compositor, no sound server needed)."""
import json
import os
import sys
import tempfile

HOME = tempfile.mkdtemp(prefix="settings-devices-")
os.environ["HOME"] = HOME
os.environ.pop("XDG_CONFIG_HOME", None)
for v in ("SWAYSOCK", "HYPRLAND_INSTANCE_SIGNATURE", "WAYLAND_DISPLAY"):
    os.environ.pop(v, None)
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from nexa_common import accessconf, displays, inputconf, notifyconf, sound, themes  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def read(*parts):
    with open(os.path.join(HOME, ".config", *parts)) as f:
        return f.read()


# ---- Display ----
screen = {"name": "eDP-1", "id": "BOE|0x0747|", "enabled": True, "scale": 1.25, "transform": "normal", "x": 0, "y": 0,
          "make": "BOE", "model": "0x0747", "mode": {"width": 1920, "height": 1080, "refresh": 60008},
          "modes": [{"width": 1920, "height": 1080, "refresh": 60008, "preferred": True},
                    {"width": 1920, "height": 1080, "refresh": 48006, "preferred": False},
                    {"width": 1280, "height": 720, "refresh": 60000, "preferred": False}]}
res = displays.resolutions(screen)
check("resolutions: largest first, rates best first", res[0] == (1920, 1080, [60008, 48006]) and res[1][:2] == (1280, 720), str(res))
check("describe uses make and model", displays.describe(screen) == "BOE 0x0747")
snap = displays.snapshot(screen)
check("snapshot can put the screen back", snap == {"name": "eDP-1", "enabled": True, "scale": 1.25, "transform": "normal",
                                                    "x": 0, "y": 0, "width": 1920, "height": 1080, "refresh": 60008}, str(snap))
displays.save([screen])
saved = json.loads(read("nexa", "displays.json"))
check("save keeps screens by id (for nexa-display apply-saved)", saved["heads"][0]["id"] == "BOE|0x0747|"
      and "name" not in saved["heads"][0] and saved["heads"][0]["scale"] == 1.25, str(saved))
check("load_saved reads it back", displays.load_saved()["BOE|0x0747|"]["width"] == 1920)
displays.save_night(True, 3500, "21:30", "06:45", restart=False)
check("night light saved for nexa-nightlight", read("nexa", "nightlight.conf") == "enabled=1\ntemp=3500\nfrom=21:30\nto=06:45\n")
check("...and read back", displays.load_night() == {"enabled": True, "temp": 3500, "from": "21:30", "to": "06:45"})
try:
    displays.save_night(True, 3500, "9pm", "07:00", restart=False)
    bad_time = False
except ValueError:
    bad_time = True
check("a time that isn't HH:MM is refused", bad_time)
displays.save_night(True, 99999, "20:00", "07:00", restart=False)
check("warmth is kept between 2500 and 6000 K", displays.load_night()["temp"] == 6000)

# ---- Sound ----
sinks = json.dumps([
    {"name": "alsa_output.pci.analog", "description": "Built-in speakers", "mute": False,
     "volume": {"front-left": {"value_percent": "60%"}, "front-right": {"value_percent": "70%"}}},
    {"name": "bluez_output.headphones", "description": "Headphones", "mute": True, "volume": {"mono": {"value_percent": "40%"}}},
])
outs = sound.parse_devices(sinks, "bluez_output.headphones")
check("outputs: names, volume averaged over channels, the default marked",
      [(o["description"], o["volume"], o["mute"], o["default"]) for o in outs]
      == [("Built-in speakers", 65, False, False), ("Headphones", 40, True, True)], str(outs))
sources = json.dumps([{"name": "alsa_output.pci.analog.monitor", "description": "Monitor"},
                      {"name": "alsa_input.pci.mic", "description": "Microphone", "volume": {}}])
check("inputs leave out the outputs' monitor copies", [i["description"] for i in sound.parse_devices(sources, "", inputs=True)]
      == ["Microphone"])
apps = sound.parse_apps(json.dumps([{"index": 42, "mute": False, "volume": {"l": {"value_percent": "80%"}},
                                     "properties": {"application.name": "Music"}}]))
check("apps playing sound", apps == [{"index": 42, "app": "Music", "icon": "audio-x-generic", "volume": 80, "mute": False}], str(apps))
check("volumes are capped at 150 %", sound._pct(400) == "150%" and sound._pct(-3) == "0%")

# ---- Mouse & Touchpad ----
p = inputconf.save({"mouse_speed": 3, "tap_to_click": False, "left_handed": True, "bogus": 1}, apply_now=False)
check("speeds clamped, unknown keys dropped", p["mouse_speed"] == 1.0 and "bogus" not in p)
sway = read("sway", "config.d", "30-nexa-input.conf")
check("Sway gets input commands (tap off, left-handed)", "input type:touchpad tap disabled" in sway
      and "input type:pointer left_handed enabled" in sway and "pointer_accel 1.0" in sway, sway)
hypr = read("hypr", "nexa-input.conf")
check("Hyprland gets an input block", "sensitivity = 1.0" in hypr and "tap-to-click = false" in hypr
      and "left_handed = true" in hypr, hypr)
check("load reads the saved choices", inputconf.load()["left_handed"] is True)

# ---- Notifications ----
n = notifyconf.clean({"timeout": 99, "position": "middle", "muted_apps": ["Chat", 'bad"name', "Chat", "Mail"]})
check("notification settings checked", n == {"timeout": 6, "position": "top-right", "muted_apps": ["Chat", "Mail"]}, str(n))
os.makedirs(os.path.join(HOME, ".config", "nexa"), exist_ok=True)
with open(notifyconf.prefs_path(), "w") as f:
    json.dump({"timeout": 0, "position": "bottom-left", "muted_apps": ["Chat"]}, f)
mako = themes.mako_config(themes.THEMES["horizon"], 0.8)
check("mako gets position, 'until closed', and hides muted apps' pop-ups",
      "anchor=bottom-left" in mako and "default-timeout=0" in mako and '[app-name="Chat"]\ninvisible=1' in mako
      and "[mode=do-not-disturb]" in mako, mako)

# ---- Accessibility ----
a = accessconf.clean({"text_scale": 1.3, "cursor_size": 48, "reduce_motion": True, "high_contrast": "yes"})
check("accessibility settings checked", a["text_scale"] == 1.3 and a["cursor_size"] == 48 and a["reduce_motion"]
      and a["high_contrast"] is False, str(a))
with open(accessconf.prefs_path(), "w") as f:
    json.dump(a, f)
gtk = themes.gtk_settings(themes.THEMES["horizon"])
check("GTK gets the bigger text, pointer and no animations", "Noto Sans 13" in gtk and "gtk-cursor-theme-size=48" in gtk
      and "gtk-enable-animations=false" in gtk, gtk)
check("Hyprland: pointer size and animations off", "XCURSOR_SIZE,48" in accessconf.hypr_conf(a) and "enabled = false" in accessconf.hypr_conf(a))
check("Sway: pointer size", "xcursor_theme Adwaita 48" in accessconf.sway_conf(a))
check("the High Contrast theme is there", themes.THEMES["contrast"]["bg"] == (0, 0, 0)
      and themes.THEMES["contrast"]["text"] == "#ffffff")

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
