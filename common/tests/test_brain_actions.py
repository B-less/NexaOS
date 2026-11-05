"""brain.run: the layer and Do not disturb actions ZeroSearch's command
model gives (nexa-strata, nexa-signalctl), with argument lists only."""
import os
import sys

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
print(f"{fails} failed")
sys.exit(1 if fails else 0)
