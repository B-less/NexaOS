"""brain.run: the layer, Do not disturb, windows and Mail actions
ZeroSearch's command model gives (nexa-strata, nexa-signalctl,
nexa-constellation, Mail), with argument lists only."""
import os
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from nexa_common import brain  # noqa: E402

ran, fails = [], 0
brain._spawn = lambda argv: ran.append(argv)


def check(ok, what):
    global fails
    print(("PASS " if ok else "FAIL ") + what)
    fails += not ok


brain.run({"type": "layer", "to": 3})
check(ran[-1] == ["nexa-strata", "go", "3"], "go to layer 3")
brain.run({"type": "layer", "step": 1})
brain.run({"type": "layer", "step": -1})
check(ran[-2:] == [["nexa-strata", "dive", "+1"], ["nexa-strata", "dive", "-1"]], "one layer deeper, one back")
try:
    brain.run({"type": "layer", "to": 9})
    check(False, "a layer that doesn't exist is refused")
except brain.BrainError:
    check(len(ran) == 3, "a layer that doesn't exist is refused")
said = brain.run({"type": "quiet", "on": True})
check(ran[-1] == ["nexa-signalctl", "mode", "-a", "do-not-disturb"] and "Signals card" in said, "Do not disturb on")
brain.run({"type": "quiet", "on": False})
check(ran[-1] == ["nexa-signalctl", "mode", "-r", "do-not-disturb"], "and off")
brain.run({"type": "windows"})
check(ran[-1] == ["nexa-constellation"], "show my windows: Constellation")

# Mail: its command from its menu entry; the words stay one argument
home = tempfile.mkdtemp()
apps = os.path.join(home, ".local/share/applications")
os.makedirs(apps)
brain.APPS_DIRS = (apps,)
try:
    brain.run({"type": "mail", "search": "kofi"})
    check(False, "without Mail: says so")
except brain.BrainError as e:
    check("Store" in str(e), "without Mail: says to get it from the Store")
prog = os.path.join(home, "mail")
with open(prog, "w") as f:
    f.write("#!/bin/sh\n")
os.chmod(prog, 0o755)
with open(os.path.join(apps, "nexa-mail.desktop"), "w") as f:
    f.write(f"[Desktop Entry]\nName=Mail\nExec={prog} %U\nMimeType=x-scheme-handler/mailto;\n")
brain.run({"type": "mail", "search": "kofi; rm -rf ~"})
check(ran[-1] == [prog, "--search", "kofi; rm -rf ~"], "find mail: one argument, never a shell")
brain.run({"type": "mail", "write": "my landlord"})
check(ran[-1] == [prog, "--write", "my landlord"], "write to someone")
brain.run({"type": "mail"})
check(ran[-1] == [prog], "open Mail")
with open(os.path.join(apps, "nexa-mail.desktop"), "w") as f:
    f.write("[Desktop Entry]\nName=Mail\nExec=sh -c evil\n")
try:
    brain.run({"type": "mail"})
    check(False, "a menu entry whose command isn't a program path is refused")
except brain.BrainError:
    check(ran[-1] == [prog], "a menu entry whose command isn't a program path is refused")
print(f"{fails} failed")
sys.exit(1 if fails else 0)
