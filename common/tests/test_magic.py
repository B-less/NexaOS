"""nexa_common.magic: defaults, a damaged file, checked changes, reset."""
import json
import os
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from nexa_common import magic  # noqa: E402

failures = 0


def check(what, ok):
    global failures
    print(("PASS " if ok else "FAIL ") + what)
    failures += not ok


home = tempfile.mkdtemp()
os.environ.pop("XDG_CONFIG_HOME", None)
os.environ["XDG_RUNTIME_DIR"] = home  # notify() finds no renderer: fine
cfg = magic.load(home)
check("defaults without a file", cfg["living"]["quality"] == "auto" and cfg["living"]["apps"]["nexa-files"] == "circuit")
check("learning features start off", not cfg["features"]["clipboard"]["enabled"] and cfg["features"]["why"]["enabled"])
check("ten styles, as the renderer has", len(magic.STYLES) == 10)
os.makedirs(os.path.join(home, ".config", "nexa"))
with open(magic.path(home), "w") as f:
    f.write("{not json")
check("a damaged file reads as defaults", magic.load(home)["living"]["enabled"] is True)
with open(magic.path(home), "w") as f:
    json.dump({"living": {"speed": "fast", "brightness": 1.2, "apps": {"foot": "nebula"}}}, f)
lv = magic.load(home)["living"]
check("a value of the wrong kind is ignored, good ones kept", lv["speed"] == 1.0 and lv["brightness"] == 1.2 and lv["apps"]["foot"] == "nebula"
      and lv["apps"]["nexa-files"] == "circuit")
magic.set_living(home, quality="low", speed=1.5)
check("changes saved", magic.load(home)["living"]["quality"] == "low")
for bad in ({"quality": "ultra"}, {"desktop": "fireworks"}, {"nonsense": 1}):
    try:
        magic.set_living(home, **bad)
        check(f"refused: {bad}", False)
    except ValueError:
        check(f"refused: {bad}", True)
magic.set_app_style(["nexa-files", "nexa-calc"], "rain", home)
check("one style for several apps", magic.app_style("nexa-calc", home) == "rain" and magic.app_style("nexa-files", home) == "rain")
magic.set_app_style("nexa-calc", "off", home)
check("off: no style, no glass forced", magic.app_style("nexa-calc", home) is None and magic.window_alpha("nexa-calc", home) is None)
check("a styled app gets see-through glass", 0.55 <= magic.window_alpha("nexa-files", home) <= 0.95)
magic.set_living(home, quality="off")
check("quality off: nobody styled", magic.app_style("nexa-files", home) is None)
magic.reset("living", home)
check("reset", magic.load(home)["living"]["quality"] == "auto" and magic.app_style("nexa-calc", home) == "geometry")
check("send without a renderer is harmless", magic.notify() is False)
sys.exit(1 if failures else 0)
