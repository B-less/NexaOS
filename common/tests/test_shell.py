"""Run with: python3 common/tests/test_shell.py

Themes engine and app search. (Deck's parsers are C: tools-c/nexa-deck.)"""
import json, os, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(HERE)), "shell"))
from nexa_common import desktopapps, themes

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

# ---- themes
home = tempfile.mkdtemp()
check("defaults with no saved settings", themes.load_settings(home) == {"theme": "horizon", "opacity": themes.DEFAULT_OPACITY, "glass": True, "icon_style": "colour", "glass_tint": "tinted"})
applied = themes.apply("nebula", 0.5, home=home, reload=False)
check("apply returns what it applied", applied == {"theme": "nebula", "opacity": 0.5, "glass": True, "icon_style": "colour", "glass_tint": "tinted"}, str(applied))
for rel in ("waybar/style.css", "foot/foot.ini", "sway/config.d/20-nexa-theme.conf",
            "gtk-3.0/settings.ini", "hypr/nexa-theme.conf", "swaylock/config", "nexa/theme.json"):
    check(f"apply writes ~/.config/{rel}", os.path.isfile(os.path.join(home, ".config", rel)))
lock = dict(l.split("=", 1) for l in open(os.path.join(home, ".config/nexa/lock.conf")).read().splitlines() if "=" in l)
check("the lock screen gets the theme's colours (nexa/lock.conf)",
      lock.get("accent") == themes.THEMES["nebula"]["accent"] and lock.get("bg", "").startswith("#")
      and all(len(lock.get(k, "")) == 7 for k in ("bg", "surface", "text", "muted", "accent2", "crit", "ok", "warn")), str(lock))
css = open(os.path.join(home, ".config/waybar/style.css")).read()
check("bar uses the theme accent and transparency",
      themes.THEMES["nebula"]["accent"] in css and "rgba(22, 10, 36, 0.50)" in css, css[:200])
check("settings persist", themes.load_settings(home) == {"theme": "nebula", "opacity": 0.5, "glass": True, "icon_style": "colour", "glass_tint": "tinted"})
themes.apply(None, None, home=home, reload=False)
check("apply --saved reuses the saved theme", themes.load_settings(home)["theme"] == "nebula")
themes.apply("not-a-theme", 7, home=home, reload=False)
s = themes.load_settings(home)
check("unknown theme keeps the current one; opacity is clamped", s == {"theme": "nebula", "opacity": themes.MAX_OPACITY, "glass": True, "icon_style": "colour", "glass_tint": "tinted"}, str(s))
themes.apply(None, None, home=home, reload=False, glass=False)
check("glass windows can be turned off and stay off", themes.load_settings(home)["glass"] is False)
themes.apply("nebula", None, home=home, reload=False)
check("changing the theme keeps the glass choice", themes.load_settings(home)["glass"] is False)
pal = themes.app_palette(themes.THEMES["horizon"], 0.84)
check("glass: windows see-through once (inner backgrounds clear), text and fields solid",
      pal["glass"].endswith("0.84)") and pal["bg"].endswith(", 0.00)") and pal["text"].startswith("#")
      and pal["field"].startswith("#"), str(pal))
solid = themes.app_palette(themes.THEMES["horizon"])
check("no glass: window colour is the solid background", solid["glass"] == solid["bg"])
check("foot gets 8 regular colours and alpha", open(os.path.join(home, ".config/foot/foot.ini")).read().count("regular") == 8)
check("GTK follows the theme's dark/light mode",
      "prefer-dark-theme=true" in open(os.path.join(home, ".config/gtk-3.0/settings.ini")).read())
themes.apply("daybreak", 0.8, home=home, reload=False)
check("light theme -> GTK light mode",
      "prefer-dark-theme=false" in open(os.path.join(home, ".config/gtk-3.0/settings.ini")).read())
themes.apply("nebula", 1.0, home=home, reload=False)
for key, t in themes.THEMES.items():
    for fn in (lambda: themes.waybar_css(t, .7), lambda: themes.foot_ini(t, .7),
               lambda: themes.sway_conf(t), lambda: themes.panel_css(t, .7)):
        try: fn(); ok = True
        except Exception as e: ok = False
        if not ok: break
    check(f"theme '{key}' renders every target", ok)
    check(f"theme '{key}' has 8 terminal colours", len(t["term"]) == 8)
for key, t in themes.THEMES.items():
    pal = themes.app_palette(t)
    # The glass edge light is the one see-through colour (rgba); none without glass.
    ok = all(isinstance(v, str) and len(v) == 7 and v.startswith("#") for k, v in pal.items() if k != "edge") \
        and pal["edge"] == "rgba(255, 255, 255, 0.00)"
    check(f"theme '{key}' app palette is complete hex colours", ok and {"bg", "text", "accent", "on_accent"} <= set(pal), str(pal))
css = themes.app_palette_css(themes.THEMES["horizon"])
check("palette CSS defines @nexa_ colours", css.count("@define-color nexa_") == len(themes.app_palette(themes.THEMES["horizon"])))
import glob, re
used = set()
for f in glob.glob(os.path.join(os.path.dirname(os.path.dirname(HERE)), "*", "nexa_*", "*.py")) + \
         glob.glob(os.path.join(os.path.dirname(HERE), "nexa_common", "*.py")):
    used |= set(re.findall(r"@nexa_([a-z_]+)", open(f).read()))
missing = used - set(themes.app_palette(themes.THEMES["horizon"]))
check("every @nexa_ colour the apps use is defined", not missing, str(missing))
open(themes.settings_path(home), "w").write("{broken")
check("a corrupt settings file falls back to defaults", themes.load_settings(home)["theme"] == "horizon")

# ---- Hexa design tokens
from nexa_common import design
import math
pts = design.hex_points(10, 10, 5)
check("hexagon: 6 corners, all on the radius, top corner straight up",
      len(pts) == 6 and all(abs(math.hypot(x - 10, y - 10) - 5) < 1e-9 for x, y in pts) and abs(pts[0][0] - 10) < 1e-9 and pts[0][1] < 10)
css = design.global_css()
check("global CSS sets the Hexa typeface and themed text fields",
      f'"{design.FONT_UI}"' in css and "@nexa_field" in css and "160ms" in css)
check("reduce motion turns transitions off", "0ms" in design.global_css(reduce_motion=True))


# ---- desktop apps
d = tempfile.mkdtemp()
def entry(fname, body): open(os.path.join(d, fname), "w").write("[Desktop Entry]\n" + body)
entry("files.desktop", "Type=Application\nName=Files\nExec=/usr/local/bin/nexa-files %U\nComment=Browse your files\nKeywords=folder;explorer;\n")
entry("browser.desktop", "Type=Application\nName=Nexa Browser\nExec=nexa-browser %u\nComment=Browse the web\n")
entry("hidden.desktop", "Type=Application\nName=Secret\nExec=x\nNoDisplay=true\n")
entry("server.desktop", "Type=Application\nName=Foot Server\nExec=foot --server\n")
entry("link.desktop", "Type=Link\nName=A link\nURL=https://example.com\n")
apps = desktopapps.load_apps([d])
check("lists only launchable, visible apps", [a["name"] for a in apps] == ["Files", "Nexa Browser"], str([a["name"] for a in apps]))
check("search by name prefix ranks first", [a["name"] for a in desktopapps.search(apps, "nex")][:1] == ["Nexa Browser"])
check("search by keyword", [a["name"] for a in desktopapps.search(apps, "explorer")] == ["Files"])
check("search matches descriptions too", {a["name"] for a in desktopapps.search(apps, "browse")} == {"Files", "Nexa Browser"})
check("no match -> empty", desktopapps.search(apps, "zzz") == [])
check("empty query -> everything", len(desktopapps.search(apps, "  ")) == 2)
check("field codes are dropped from the command", desktopapps.command("/usr/local/bin/nexa-files %U") == ["/usr/local/bin/nexa-files"])

# ---- Nexus search
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(HERE)), "shell"))
from nexa_shell import search as nsearch
check("calculator: arithmetic", nsearch.calculate("2+2*3") == 8 and nsearch.calculate("10/4") == 2.5
      and nsearch.calculate("3 × 4") == 12)
check("calculator: never runs code", nsearch.calculate("__import__('os').system('id')") is None
      and nsearch.calculate("(1).__class__") is None)
check("calculator: refuses huge powers and division by zero", nsearch.calculate("9**9**9") is None and nsearch.calculate("1/0") is None)
check("calculator: plain words aren't sums", nsearch.calculate("hello") is None and nsearch.calculate("42") is None)
apps_ = [{"name": "Nexa Pulse", "comment": "Task manager", "keywords": "processes;", "icon": "nexa-pulse", "exec": "/usr/local/bin/nexa-pulse %U"}]
r = nsearch.search(apps_, "pulse")
check("search: app first, its command without field codes", r[0]["kind"] == "app" and r[0]["command"] == ["/usr/local/bin/nexa-pulse"], str(r))
r = nsearch.search(apps_, "wallpaper")
check("search: settings pages by what they contain", r[0] == {"kind": "setting", "title": "Personalization", "subtitle": "Settings",
                                                              "icon": "nexa-settings", "command": ["nexa-settings", "personalization"]}, str(r))
r = nsearch.search(apps_, "why is my laptop slow")
check("search: anything else goes to Nexa", r[-1]["kind"] == "ask" and r[-1]["command"] == ["nexa-ai-gui", "--ask", "why is my laptop slow"], str(r))
check("search: sums don't offer Nexa", [x["kind"] for x in nsearch.search(apps_, "7*6")] == ["calc"])

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
