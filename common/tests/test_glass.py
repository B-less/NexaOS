"""Glass: clear or tinted, with an edge light. Run: python3 common/tests/test_glass.py"""
import os, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from nexa_common import themes
failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)
home = tempfile.mkdtemp()
check("tinted glass by default", themes.load_settings(home)["glass_tint"] == "tinted")
for c in ("hyprland", "sway"):
    check(f"clear glass lets more through than tinted ({c})", themes.glass_alpha(c, "clear") < themes.glass_alpha(c, "tinted"))
check("Sway (no blur) keeps glass more solid than Hyprland", themes.glass_alpha("sway", "clear") > themes.glass_alpha("hyprland", "clear"))
t = themes.THEMES["horizon"]
off, tinted, clear = themes.app_palette(t), themes.app_palette(t, 0.84, "tinted"), themes.app_palette(t, 0.62, "clear")
check("no edge light without glass", off["edge"].endswith("0.00)"), off["edge"])
check("an edge light on glass, brighter on clear", tinted["edge"] == "rgba(255, 255, 255, 0.13)" and clear["edge"] == "rgba(255, 255, 255, 0.24)", f"{tinted['edge']} {clear['edge']}")
check("clear glass is frosted (lighter), tinted keeps the theme colour", clear["glass"] != tinted["glass"] and clear["glass"].endswith("0.62)"), f"{clear['glass']} / {tinted['glass']}")
applied = themes.apply(glass_tint="clear", home=home, reload=False)
cfg = os.path.join(home, ".config")
check("choosing Clear is saved", applied["glass_tint"] == "clear" and themes.load_settings(home)["glass_tint"] == "clear")
hypr = open(os.path.join(cfg, "hypr", "nexa-glass.conf")).read()
check("Hyprland: clear blur and an edge-lit border", "brightness = 1.0" in hypr and "rgba(ffffff88)" in hypr, hypr)
gtk = open(os.path.join(cfg, "gtk-3.0", "gtk.css")).read()
check("every GTK app gets the edge light (gtk.css)", "@define-color nexa_edge rgba(255, 255, 255, 0.24);" in gtk)
themes.apply(glass=False, home=home, reload=False)
hypr = open(os.path.join(cfg, "hypr", "nexa-glass.conf")).read()
gtk = open(os.path.join(cfg, "gtk-3.0", "gtk.css")).read()
check("glass off: no glass settings, no edge light", "glass windows are off" in hypr and "nexa_edge rgba(255, 255, 255, 0.00)" in gtk, hypr)
check("...and the choice of look is kept for later", themes.load_settings(home)["glass_tint"] == "clear")
print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
