"""Strata in a headless Sway: nexa-strata moves, Orbit shows the stack,
Living tints the wallpaper. Run by scripts/test-strata.sh; screenshots go
to $OUT."""
import json
import os
import subprocess
import sys
import time

import gi
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf

VP, OUT = "/src/build/test/nexa-vpointer", os.environ.get("OUT", "/tmp")
FAKEAPP = "/src/tools-c/nexa-orbit/tests/fakeapp.py"
STATE, ORBIT_STATE = "/tmp/strata.state", "/tmp/orbit.state"
W, H = 1280, 800
CFG = os.path.expanduser("~/.config/nexa")
STRATA_CONF = os.path.join(CFG, "strata.conf")
STACK_W, STACK_H, STACK_MARGIN = 300, 240, 90
SPOT = (1180, 40, 20, 20)   # bare desktop: no window, no stack, no Orbit
fails = 0


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"))
    fails += not ok


def stack():
    try:
        return json.load(open(STATE))
    except (OSError, ValueError):
        return {}


def wait(cond, secs=5.0):
    end = time.time() + secs
    while time.time() < end:
        try:
            if cond():
                return True
        except (KeyError, TypeError, IndexError):
            pass
        time.sleep(0.05)
    return False


def sway(*cmd):
    return subprocess.run(["swaymsg", *cmd], capture_output=True, text=True).stdout


def layer():
    return next(w["num"] for w in json.loads(sway("-t", "get_workspaces")) if w["focused"])


def strata(*args):
    return subprocess.run(["nexa-strata", *args], capture_output=True, text=True, timeout=5)


def go(n):
    sway("workspace", "number", str(n))
    wait(lambda: stack().get("current") == n, 3)


def settle_gap():
    time.sleep(0.2)  # past nexa-strata's 150 ms touchpad guard


def shot(name):
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def pixel(x=SPOT[0], y=SPOT[1], w=SPOT[2], h=SPOT[3]):
    """The average colour of a patch of the screen, 0..255."""
    path = "/tmp/patch.png"
    subprocess.run(["grim", "-g", f"{x},{y} {w}x{h}", path], check=True)
    p = GdkPixbuf.Pixbuf.new_from_file(path)
    data, n, rs = p.get_pixels(), p.get_n_channels(), p.get_rowstride()
    tot = [0, 0, 0]
    for j in range(p.get_height()):
        for i in range(p.get_width()):
            o = j * rs + i * n
            for c in range(3):
                tot[c] += data[o + c]
    k = p.get_width() * p.get_height()
    return [t / k for t in tot]


def jumps(old, new, secs=2.0):
    """Watch the bare desktop until it leaves `old`: (did the first colour
    after it already equal `new`, i.e. no fade on the way; detail). The
    test's Living may run at a low frame rate, so how soon isn't asked."""
    seen, end = [], time.time() + secs
    while time.time() < end:
        seen.append(pixel())
        if not close(seen[-1], old, 2.0):
            break
    first = next((p for p in seen if not close(p, old, 2.0)), None)
    return first is not None and close(first, new), f"saw {seen[-4:]}, from {old} to {new}"


def hexrgb(s):
    return [int(s[i:i + 2], 16) for i in (1, 3, 5)]


def blend(base, tint, a):
    return [b * (1 - a) + t * a for b, t in zip(base, tint)]


def close(a, b, tol=4.0):
    return max(abs(x - y) for x, y in zip(a, b)) <= tol


def windows():
    tree, out, stack_ = json.loads(sway("-t", "get_tree")), [], []
    stack_.append((tree, 0))
    while stack_:
        n, ws = stack_.pop()
        if n.get("type") == "workspace":
            ws = n.get("num")
        if n.get("app_id"):
            out.append(dict(n, ws=ws))
        stack_.extend((c, ws) for c in n.get("nodes", []) + n.get("floating_nodes", []))
    return out


def win(app):
    return next((w for w in windows() if w["app_id"] == app), None)


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path + ".tmp", "w") as f:
        f.write(text)
    os.rename(path + ".tmp", path)


def pss_kb(pid):
    try:
        return sum(int(l.split()[1]) for l in open(f"/proc/{pid}/smaps_rollup") if l.startswith("Pss:"))
    except OSError:
        return 0


check(open("/tmp/sway-validate.rc").read().strip() == "0", "Sway accepts the session's Strata bindings",
      open("/tmp/sway-validate.log").read()[-300:])
bindings = sway("-t", "get_config")
check(bindings.count("nexa-strata") == 8, "all eight bindings are in the running config", str(bindings.count("nexa-strata")))

# Desktop files so Orbit (and so the stack) knows the apps' icons.
apps = os.path.expanduser("~/.local/share/applications")
os.makedirs(apps, exist_ok=True)
for app_id, name, icon in (("nexa-files", "Files", "folder"), ("nexa-notes", "Notes", "accessories-text-editor")):
    write(os.path.join(apps, app_id + ".desktop"),
          f"[Desktop Entry]\nType=Application\nName={name}\nIcon={icon}\nExec=python3 {FAKEAPP} {app_id}\n")
write(os.path.join(CFG, "orbit.conf"), "pinned=nexa-files\nfold=no\n")
# Living with no desktop style: only the wallpaper and the tint, so the
# colour on screen can be worked out. No tint to start with.
write(os.path.join(CFG, "magic.json"), json.dumps({"living": {"desktop": "none", "wallpaper": "/src/assets/wallpaper.png"}}))
write(STRATA_CONF, "tint=0\n")

living = subprocess.Popen(["nexa-living"], stderr=open("/tmp/living.err", "w"))
orbit = subprocess.Popen(["nexa-orbit"], env=dict(os.environ, NEXA_STRATA_STATE=STATE, NEXA_ORBIT_STATE=ORBIT_STATE),
                         stderr=open("/tmp/orbit.err", "w"))
check(wait(lambda: stack().get("current") == 1), "Orbit notes the layer it starts on", str(stack()))
check(not stack().get("shown"), "and shows no stack for it")
r = strata("current")
check(r.returncode == 0 and r.stdout.strip() == "1", "nexa-strata current: layer 1", r.stdout + r.stderr)
t0 = time.time()
for _ in range(20):
    strata("current")
per_call = (time.time() - t0) / 20 * 1000
check(per_call < 30, f"nexa-strata answers fast ({per_call:.1f} ms a call)")

files = subprocess.Popen(["python3", FAKEAPP, "nexa-files"])
check(wait(lambda: win("nexa-files") is not None, 8), "Files opens on layer 1")
sway("[app_id=nexa-files]", "move", "position", "820", "120")
check(wait(lambda: stack()["layers"][0]["windows"] == 1 and stack()["layers"][0]["icons"] == 1),
      "the stack knows layer 1 has Files, with its icon", str(stack().get("layers")))
time.sleep(1.0)
p0 = pixel()

# The tint: strength 0.14 (the default) once strata.conf says nothing.
write(STRATA_CONF, "# Strata\nshow=yes\n")
check(wait(lambda: stack().get("tint") == 0.14, 3), "strata.conf is followed live (default tint 0.14)", str(stack().get("tint")))
time.sleep(0.8)
p1 = pixel()
tints = [hexrgb(l["tint"]) for l in stack()["layers"]]
check(stack()["layers"][0]["tint"] == "#4da3ff", "layer 1's colour is Horizon's accent", stack()["layers"][0]["tint"])
check(len({l["tint"] for l in stack()["layers"]}) == 5, "every layer has its own colour", str([l["tint"] for l in stack()["layers"]]))
check(close(p1, blend(p0, tints[0], 0.14)), "Living tints the wallpaper with layer 1's colour",
      f"{p1} vs {blend(p0, tints[0], 0.14)}")
shot("strata-1-layer1.png")

# Dive: the stack appears, then goes; the tint follows.
r = strata("dive", "+1")
check(r.returncode == 0 and layer() == 2, "dive +1: on layer 2", r.stderr)
check(wait(lambda: stack()["shown"] and stack()["current"] == 2, 1), "the stack shows, at layer 2", str(stack()))
s = stack()
check(s.get("surface") == [STACK_W, STACK_H], "the stack's size", str(s.get("surface")))
time.sleep(0.3)
shot("strata-2-dive.png")
check(wait(lambda: not stack()["shown"], 2.5), "the stack goes after about a second")
time.sleep(0.3)
p2 = pixel()
check(close(p2, blend(p0, tints[1], 0.14)), "layer 2 tints the wallpaper with its own colour", f"{p2} vs {blend(p0, tints[1], 0.14)}")
check(max(abs(a - b) for a, b in zip(p1, p2)) > 2, "the two layers look different", f"{p1} {p2}")

# Ends: nothing above 1 or below 5.
go(5)
check(stack()["shown"], "a jump straight to a layer (Super+5) shows the stack too")
settle_gap()
r = strata("dive", "+1")
check(r.returncode == 0 and layer() == 5, "dive +1 at layer 5 stays there (no wrapping)", f"{r.returncode} {layer()}")
go(1)
settle_gap()
r = strata("dive", "-1")
check(r.returncode == 0 and layer() == 1, "dive -1 at layer 1 stays there")

# A touchpad's stream of scroll steps: one move per 150 ms.
settle_gap()
strata("dive", "+1")
strata("dive", "+1")
strata("dive", "+1")
check(layer() == 2, "three steps at once move one layer", str(layer()))
settle_gap()
strata("dive", "+1")
check(layer() == 3, "the next step after a pause moves again", str(layer()))
check(wait(lambda: stack()["current"] == 3 and stack()["shown"], 1), "the stack follows to layer 3")
time.sleep(0.3)
shot("strata-3-layer3.png")
check(strata("dive", "x").returncode == 2 and strata("sideways", "+1").returncode == 2, "bad arguments refused")

# Carry: the window in front comes along.
go(1)
sway("[app_id=nexa-files]", "focus")
settle_gap()
r = strata("carry", "+1")
check(r.returncode == 0 and layer() == 2 and (win("nexa-files") or {}).get("ws") == 2, "carry +1 takes Files to layer 2",
      f"{r.stderr} {layer()} {(win('nexa-files') or {}).get('ws')}")
check(wait(lambda: stack()["layers"][1]["windows"] == 1 and stack()["layers"][0]["windows"] == 0, 2),
      "the stack shows Files on layer 2 now", str(stack().get("layers")))
go(4)
settle_gap()
r = strata("carry", "+1")
check(r.returncode == 0 and layer() == 5, "carry from an empty layer just moves", r.stderr)

# Clicks go through the stack: Notes lies under it on layer 2.
go(2)
notes = subprocess.Popen(["python3", FAKEAPP, "nexa-notes"])
check(wait(lambda: win("nexa-notes") is not None, 8), "Notes opens on layer 2")
sx, sy = (W - STACK_W) // 2, H - STACK_MARGIN - STACK_H
sway("[app_id=nexa-notes]", "move", "position", str(sx), str(sy + 20))
go(1)
settle_gap()
strata("dive", "+1")
check(wait(lambda: stack()["shown"], 1), "on layer 2 again, the stack is over Notes")
subprocess.run([VP, str(W), str(H), "move", str(sx + STACK_W // 2), str(sy + STACK_H - 40), "wait", "60", "click"], timeout=10)
shown_at_click = stack()["shown"]
check(wait(lambda: (win("nexa-notes") or {}).get("focused"), 2) and shown_at_click,
      "a click on the stack reaches the window under it", f"shown={shown_at_click}")

# show=no: no stack, the tint still changes.
write(STRATA_CONF, "show=no\n")
check(wait(lambda: stack().get("show") == 0, 3), "show=no is followed live")
settle_gap()
strata("dive", "+1")
time.sleep(0.5)
check(layer() == 3 and not stack()["shown"], "with show=no the layer changes without the stack")
time.sleep(0.3)
check(close(pixel(), blend(p0, tints[2], 0.14)), "but the tint still follows (layer 3)")
write(STRATA_CONF, "show=yes\ntint=0.25\n")
check(wait(lambda: stack().get("tint") == 0.25, 3), "a stronger tint read")
ok, why = jumps(blend(p0, tints[2], 0.14), blend(p0, tints[2], 0.25))  # (it may be drawn already)
check(ok, "and drawn at once, without a fade", why)

# A theme change: new colours for every layer, in the stack and the wallpaper.
subprocess.run([sys.executable, "-c", "import sys; sys.path.insert(0, '/src/common'); from nexa_common import themes; "
                "themes.apply('nebula', reload=False)"], check=True)
check(wait(lambda: stack()["layers"][0]["tint"] != "#4da3ff", 3), "Nebula: the layers take its accent",
      stack()["layers"][0]["tint"])
time.sleep(0.6)
nebula = [hexrgb(l["tint"]) for l in stack()["layers"]]
settle_gap()
strata("dive", "+1")
time.sleep(0.3)
shot("strata-4-nebula.png")
time.sleep(0.4)
check(close(pixel(), blend(p0, nebula[3], 0.25)), "Living follows the theme too (layer 4 in Nebula)")

# Memory: the stack lives in Orbit; what does it add while shown?
wait(lambda: not stack()["shown"], 3)
time.sleep(0.3)
idle = pss_kb(orbit.pid)
go(1)
time.sleep(0.2)
showing = pss_kb(orbit.pid)
print(f"INFO Orbit PSS {idle / 1024:.1f} MB, {showing / 1024:.1f} MB with the stack shown; "
      f"Living {pss_kb(living.pid) / 1024:.1f} MB")

# Reduced motion: Living's tint changes at once; the stack doesn't glide.
write(os.path.join(CFG, "interface", "global.json"), json.dumps({"motion": "none"}))
time.sleep(0.5)
wait(lambda: close(pixel(), blend(p0, nebula[0], 0.25)), 2)  # settled on layer 1
settle_gap()
strata("dive", "+1")
ok, why = jumps(blend(p0, nebula[0], 0.25), blend(p0, nebula[1], 0.25))
check(ok, "with reduced motion the tint changes at once", why)
orbit.terminate()
orbit.wait(5)
# As in the session: GSETTINGS_BACKEND=keyfile (on Wayland GTK takes the
# animation setting from GSettings, not settings.ini).
write(os.path.expanduser("~/.config/glib-2.0/settings/keyfile"),
      "[org/gnome/desktop/interface]\nenable-animations=false\n")
os.remove(STATE)
orbit = subprocess.Popen(["nexa-orbit"], env=dict(os.environ, NEXA_STRATA_STATE=STATE), stderr=open("/tmp/orbit.err", "a"))
check(wait(lambda: stack().get("animations") == 0, 4), "the stack reads GTK's animation setting")
settle_gap()
strata("dive", "+1")
check(wait(lambda: stack()["shown"] and stack()["current"] == 3 and stack()["pos"] == 3.0, 1),
      "with reduced motion it shows the new layer straight away", str(stack()))
check(wait(lambda: not stack()["shown"], 2.5), "and still goes after about a second")

for p in (files, notes, orbit, living):
    p.terminate()
err = open("/tmp/orbit.err").read() + open("/tmp/living.err").read()
bad = [l for l in err.splitlines() if "CRITICAL" in l or "WARNING" in l and "Glycin" not in l]
check(not bad, "no GTK warnings or criticals", "\n".join(bad[:5]))
print(f"{fails} failed")
sys.exit(1 if fails else 0)
