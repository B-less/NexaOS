"""strataconf: Settings > Layers' file, read the same way as layers.c."""
import os
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from nexa_common import strataconf  # noqa: E402

fails = 0


def check(ok, what):
    global fails
    print(("PASS " if ok else "FAIL ") + what)
    fails += not ok


c = strataconf.parse("")
check(c["show"] and c["tint"] == 0.14 and all(not m["name"] and not m["quiet"] for m in c["layers"]), "no file: the defaults")
t = ("show=no\ntint=9\nlayer.3.name=Work\nlayer.3.signals=quiet\nlayer.3.mute=Chat, Mail,,\nlayer.5.dim=0.45\n"
     "layer.5.power=powersave\nlayer.5.style=none\nlayer.1.power=turbo\nlayer.2.style=../x\nlayer.9.name=x\nlayer.4.dim=abc\n"
     "layer.2.name=\x01Pl\x7fay\n")
c = strataconf.parse(t)
check(not c["show"] and c["tint"] == strataconf.TINT_MAX, "show and a capped tint")
w = c["layers"][2]
check(w["name"] == "Work" and w["quiet"] and w["mute"] == ["Chat", "Mail"], "a layer's name, quiet and muted apps")
n = c["layers"][4]
check(n["dim"] == 0.45 and n["power"] == "powersave" and n["style"] == "none", "dim, power and style")
check(c["layers"][0]["power"] == "" and c["layers"][1]["style"] == "" and c["layers"][3]["dim"] == 0.0, "bad values ignored")
check(c["layers"][1]["name"] == "Play", "control characters dropped")
with tempfile.TemporaryDirectory() as home:
    c["layers"][0] = dict(strataconf.PRESETS["night"], mute=["A,B"])
    back = strataconf.save(c, home)
    text = open(strataconf.path(home)).read()
    check(back == strataconf.parse(text) and back["layers"][0]["name"] == "Night" and back["layers"][0]["dim"] == 0.45,
          "saved and read back the same")
    check("layer.1.mute=A B" in text, "a comma inside an app name can't split it")
    check(strataconf.load(home)["layers"][2]["name"] == "Work", "load")
print(f"{fails} failed")
sys.exit(1 if fails else 0)
