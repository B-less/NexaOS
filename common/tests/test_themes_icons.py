"""Which icon set goes with a theme: NexaHexa-<theme> when installed, else
NexaHexa, else Adwaita; settings.ini says so. Run: python3 common/tests/test_themes_icons.py"""
import os, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from nexa_common import themes
failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)
home = tempfile.mkdtemp()
themes.ICON_DIRS = (tempfile.mkdtemp(),)
check("nothing installed: Adwaita", themes.icon_theme_for("aurora", home) == "Adwaita")
icons = os.path.join(home, ".local", "share", "icons")
os.makedirs(os.path.join(icons, "NexaHexa")); open(os.path.join(icons, "NexaHexa", "index.theme"), "w").close()
check("only the base set: NexaHexa", themes.icon_theme_for("aurora", home) == "NexaHexa")
os.makedirs(os.path.join(icons, "NexaHexa-aurora")); open(os.path.join(icons, "NexaHexa-aurora", "index.theme"), "w").close()
check("the theme's own variant when there is one", themes.icon_theme_for("aurora", home) == "NexaHexa-aurora")
check("another theme without a variant: the base set", themes.icon_theme_for("nebula", home) == "NexaHexa")
themes.apply("aurora", home=home, reload=False)
ini = open(os.path.join(home, ".config", "gtk-3.0", "settings.ini")).read()
check("applying the theme writes its icon set to settings.ini", "gtk-icon-theme-name=NexaHexa-aurora" in ini, ini)
print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
