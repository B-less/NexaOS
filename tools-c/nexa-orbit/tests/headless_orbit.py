"""Orbit (nexa-orbit) in a headless Sway with a real pointer (nexa-vpointer).
Run by scripts/test-orbit.sh; screenshots go to $OUT."""
import json
import os
import subprocess
import sys
import time

ORBIT, VP, OUT = "/tmp/nexa-orbit", "/src/build/test/nexa-vpointer", os.environ.get("OUT", "/tmp")
STATE = "/tmp/orbit.state"
W, H = 1280, 800
HERE = os.path.dirname(os.path.abspath(__file__))
CONF = os.path.expanduser("~/.config/nexa/orbit.conf")
fails = 0


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"))
    fails += not ok


def state():
    try:
        lines = open(STATE).read().splitlines()
        head = json.loads(lines[0])
        return head, {d["id"]: d for d in map(json.loads, lines[1:])}, [json.loads(l)["id"] for l in lines[1:]]
    except (OSError, ValueError, IndexError):
        return {}, {}, []


def wait(cond, secs=6.0):
    end = time.time() + secs
    while time.time() < end:
        try:
            if cond():
                return True
        except (KeyError, TypeError):
            pass
        time.sleep(0.1)
    return False


def screen_y(item_id):
    head, items, _ = state()
    return (H - head["surface"][1]) // 2 + items[item_id]["y"]


def pointer(*args):
    subprocess.run([VP, str(W), str(H), *map(str, args)], check=True, timeout=20)


def shot(name):
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def sway(*cmd):
    return subprocess.run(["swaymsg", *cmd], capture_output=True, text=True).stdout


def windows():
    tree, out, stack = json.loads(sway("-t", "get_tree")), [], []
    stack.append(tree)
    while stack:
        n = stack.pop()
        if n.get("app_id"):
            out.append(n)
        stack.extend(n.get("nodes", []) + n.get("floating_nodes", []))
    return out


# Stand-in apps: their desktop files start fakeapp.py with the right app id.
apps = os.path.expanduser("~/.local/share/applications")
os.makedirs(apps, exist_ok=True)
for app_id, name, icon, extra in (("nexa-files", "Files", "folder", ""), ("nexa-browser", "Browser", "web-browser", " busy"),
                                  ("nexa-notes", "Notes", "accessories-text-editor", "")):
    with open(os.path.join(apps, app_id + ".desktop"), "w") as f:
        f.write(f"[Desktop Entry]\nType=Application\nName={name}\nIcon={icon}\n"
                f"Exec=python3 {HERE}/fakeapp.py {app_id}{extra} %U\n")
os.makedirs(os.path.dirname(CONF), exist_ok=True)
with open(CONF, "w") as f:
    f.write("pinned=nexa-files,nexa-browser,nexa-notes,not-installed\nfold=no\n")

env = dict(os.environ, NEXA_ORBIT_STATE=STATE)
# Started as "nexa-orbit", as the session does (BusyBox pkill -x matches argv[0]).
orbit = subprocess.Popen(["nexa-orbit"], executable=ORBIT, env=env, stderr=open("/tmp/orbit.err", "w"))

check(wait(lambda: state()[2] == ["nexus", "nexa-files", "nexa-browser", "nexa-notes"]),
      "the rail shows the mark and the pinned apps (one not installed is left out)", str(state()[2]))
head, items, _ = state()
check(head.get("exclusive") == 72, "it keeps a strip of the left edge for itself", str(head.get("exclusive")))
check(all(items[i]["icon"] for i in ("nexa-files", "nexa-browser", "nexa-notes")), "every app has an icon")
check(head.get("accent") == "#4da3ff", "Horizon's accent", str(head.get("accent")))
time.sleep(0.5)
shot("orbit-1-rest.png")

# Click Files: it starts, shows "starting", then runs.
pointer("move", 38, screen_y("nexa-files"), "wait", 150, "click")
check(wait(lambda: state()[1]["nexa-files"]["starting"] or state()[1]["nexa-files"]["running"], 3), "a click starts Files")
check(wait(lambda: state()[1]["nexa-files"]["running"] and not state()[1]["nexa-files"]["starting"], 10),
      "Files' window arrives: running, no longer starting")
files_win = next((w for w in windows() if w["app_id"] == "nexa-files"), None)
check(files_win is not None and files_win["rect"]["x"] >= 72, "the new window opens beside Orbit, not under it",
      str(files_win and files_win["rect"]))

# Middle click on Browser: a busy app; its ring traces its CPU use.
pointer("move", 38, screen_y("nexa-browser"), "wait", 100, "mclick")
check(wait(lambda: state()[1]["nexa-browser"]["running"], 10), "a middle click starts Browser")
check(wait(lambda: state()[1]["nexa-browser"]["cpu"] > 50, 9), "Browser's activity is measured (one core busy)",
      str(state()[1].get("nexa-browser", {}).get("cpu")))
check(state()[1]["nexa-files"]["cpu"] < 20, "Files, idle, is not shown as busy", str(state()[1]["nexa-files"]["cpu"]))

# Clicking a running app brings it forward.
pointer("move", 38, screen_y("nexa-files"), "wait", 100, "click")
check(wait(lambda: state()[1]["nexa-files"]["focused"], 4), "clicking running Files brings it in front")
pointer("move", 38, screen_y("nexa-browser"), "wait", 100, "click")
check(wait(lambda: state()[1]["nexa-browser"]["focused"] and not state()[1]["nexa-files"]["focused"], 4),
      "then Browser, and Files is no longer in front")

# An app that wants attention.
sway("[app_id=nexa-files] urgent enable")
check(wait(lambda: state()[1]["nexa-files"]["urgent"], 4), "an app asking for attention is marked")

# A window from an app with no desktop file: shown after the pinned ones.
mystery = subprocess.Popen(["python3", f"{HERE}/fakeapp.py", "mystery-app"])
check(wait(lambda: state()[2][-1] == "mystery-app" and state()[1]["mystery-app"]["running"], 8),
      "a running app that isn't pinned appears after the pinned ones", str(state()[2]))
# The pretend mouse leaves when nexa-vpointer exits: keep it there for the shot.
held = subprocess.Popen([VP, str(W), str(H), "move", "38", str(screen_y("nexa-browser")), "wait", "3000"])
check(wait(lambda: state()[0]["hover"] == 2, 3), "pointing at Browser shows its name", str(state()[0].get("hover")))
time.sleep(0.8)
shot("orbit-2-running.png")
held.wait()

# Right click: the app's menu.
held = subprocess.Popen([VP, str(W), str(H), "move", "38", str(screen_y("nexa-files")), "wait", "100", "rclick", "wait", "2500",
                         "move", "700", "400", "click"])  # then away: closes the menu
time.sleep(1.2)
shot("orbit-3-menu.png")
held.wait()

# Pinned order follows the file, live.
with open(CONF, "w") as f:
    f.write("pinned=nexa-notes,nexa-files\nfold=no\n")
check(wait(lambda: state()[2][:3] == ["nexus", "nexa-notes", "nexa-files"] and "nexa-browser" in state()[2][3:], 4),
      "a change to orbit.conf is followed live (Browser, unpinned, moves down with the running ones)", str(state()[2]))

# A theme change recolours it live.
subprocess.run([sys.executable, "-c", "import sys; sys.path.insert(0, '/src/common'); from nexa_common import themes; "
                "themes.apply('nebula', reload=False)"], check=True)
check(wait(lambda: state()[0]["accent"] == "#c77dff", 4), "a theme change recolours Orbit live", str(state()[0].get("accent")))
time.sleep(0.6)
shot("orbit-4-nebula.png")

# Super+O: the keyboard.
subprocess.run(["pkill", "-USR1", "-x", "nexa-orbit"])
check(wait(lambda: state()[0]["keyboard"] == 1 and state()[0]["kfocus"] == 1, 3), "Super+O gives Orbit the keyboard")
time.sleep(0.4)
shot("orbit-5-keyboard.png")
subprocess.run(["pkill", "-USR1", "-x", "nexa-orbit"])
check(wait(lambda: state()[0]["keyboard"] == 0, 3), "and pressing it again gives it back")

# Fold: no strip kept, a line of light until the pointer touches the edge.
pointer("move", 700, 400)
with open(CONF, "w") as f:
    f.write("pinned=nexa-notes,nexa-files\nfold=yes\n")
check(wait(lambda: state()[0]["folded"] == 1 and state()[0]["exclusive"] == 0, 4), "fold=yes: folded, keeps no strip",
      str(state()[0]))
time.sleep(1.0)
shot("orbit-6-folded.png")
held = subprocess.Popen([VP, str(W), str(H), "move", "1", "400", "wait", "2500", "move", "700", "400", "wait", "2500"])
check(wait(lambda: state()[0]["folded"] == 0, 2.4), "touching the left edge unfolds it")
check(wait(lambda: state()[0]["folded"] == 1, 4), "and it folds again when the pointer leaves")
held.wait()

# Close all windows from the menu's action (the compositor's kill).
check(sum(1 for w in windows() if w["app_id"] == "mystery-app") == 1, "the unknown app's window is there before closing")
mystery.terminate()
check(wait(lambda: "mystery-app" not in state()[2], 6), "an unpinned app leaves the rail when its window closes")

# Memory (PSS), for the plan's measurements.
try:
    pss = next(int(l.split()[1]) for l in open(f"/proc/{orbit.pid}/smaps_rollup") if l.startswith("Pss:"))
    print(f"INFO nexa-orbit PSS {pss / 1024:.1f} MB, RSS "
          f"{next(int(l.split()[1]) for l in open(f'/proc/{orbit.pid}/status') if l.startswith('VmRSS')) / 1024:.1f} MB")
except (OSError, StopIteration):
    pass
check(orbit.poll() is None, "Orbit still running at the end")
orbit.terminate()
err = open("/tmp/orbit.err").read()
check("CRITICAL" not in err and "Segmentation" not in err, "no GTK criticals", err[-400:])
print(("PASS" if not fails else "FAIL") + f": {fails} failed")
sys.exit(1 if fails else 0)
