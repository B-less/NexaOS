"""Constellation in a headless Sway: Orbit hosts the sky, nexa-starmap finds
the projects, the session's Super+Tab opens it; real keys (wtype) and a real
pointer (nexa-vpointer). Run by scripts/test-constellation.sh; screenshots
go to $OUT."""
import json
import os
import socket
import subprocess
import sys
import threading
import time

import gi
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf

VP, OUT = "/src/build/test/nexa-vpointer", os.environ.get("OUT", "/tmp")
STAR = "/src/tools-c/nexa-constellation/tests/star.py"
STATE = "/tmp/sky.state"
W, H = 1280, 800
HOME = os.path.expanduser("~")
fails = 0


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"), flush=True)
    fails += not ok


def sky():
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
        except (KeyError, TypeError, IndexError, StopIteration):
            pass
        time.sleep(0.05)
    return False


def sway(*cmd):
    return subprocess.run(["swaymsg", *cmd], capture_output=True, text=True).stdout


def windows():
    tree, out, todo = json.loads(sway("-t", "get_tree")), [], []
    todo.append((tree, 0))
    while todo:
        n, ws = todo.pop()
        if n.get("type") == "workspace":
            ws = n.get("num")
        if n.get("app_id"):
            out.append(dict(n, ws=ws))
        todo.extend((c, ws) for c in n.get("nodes", []) + n.get("floating_nodes", []))
    return out


def by_title(t):
    return next((w for w in windows() if w["name"] == t), None)


def focused():
    return next((w for w in windows() if w.get("focused")), None)


def layer():
    return next(w["num"] for w in json.loads(sway("-t", "get_workspaces")) if w["focused"])


def keys(*args):
    """Real keys. Each wtype run is a new virtual keyboard with its own key
    map: runs closer than this can lose their first key while GTK switches
    maps (a test artefact: a real keyboard keeps its map)."""
    subprocess.run(["wtype", *args], check=True, timeout=10)
    time.sleep(0.35)


def super_tab():
    keys("-M", "logo", "-k", "Tab", "-m", "logo")


def star(title):
    return next(s for s in sky()["stars"] if s["title"] == title)


def sel():
    s = sky()
    return next((x for x in s["stars"] if x["id"] == s["selected"]), None)


def shot(name):
    time.sleep(0.4)
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def pixel(x, y, w=12, h=12):
    path = "/tmp/patch.png"
    subprocess.run(["grim", "-g", f"{x},{y} {w}x{h}", path], check=True)
    p = GdkPixbuf.Pixbuf.new_from_file(path)
    data, n, rs = p.get_pixels(), p.get_n_channels(), p.get_rowstride()
    tot = [0, 0, 0]
    for j in range(p.get_height()):
        for i in range(p.get_width()):
            for c in range(3):
                tot[c] += data[j * rs + i * n + c]
    return [t / (p.get_width() * p.get_height()) for t in tot]


def pss_kb(pid):
    for line in open(f"/proc/{pid}/smaps_rollup"):
        if line.startswith("Pss:"):
            return int(line.split()[1])
    return 0


def private_kb(pid):
    """The process's own memory (heap and anonymous mappings), not shared files."""
    kb, name = 0, None
    for line in open(f"/proc/{pid}/smaps"):
        f = line.split()
        if "-" in f[0] and not f[0].endswith(":"):
            name = f[5] if len(f) > 5 else "[anon]"
        elif f[0] == "Pss:" and name in ("[anon]", "[heap]"):
            kb += int(f[1])
    return kb


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


# Nexa Link, stood in for: the device list over its socket, and a nexa-linkd
# that writes down how it was asked to send.
LINK_DEVICES = []
def link_server():
    path = os.path.join(os.environ["XDG_RUNTIME_DIR"], "nexa-link.sock")
    srv = socket.socket(socket.AF_UNIX)
    srv.bind(path)
    srv.listen(4)
    while True:
        c, _ = srv.accept()
        req = c.makefile().readline()
        if '"devices"' in req:
            c.sendall((json.dumps({"ok": True, "devices": LINK_DEVICES}) + "\n").encode())
        c.close()
threading.Thread(target=link_server, daemon=True).start()
FAKE_LINKD = "/tmp/fake-linkd"
with open(FAKE_LINKD, "w") as f:
    f.write("#!/bin/sh\necho \"$@\" > /tmp/link-args\nsleep \"$(cat /tmp/link-slow 2>/dev/null || echo 0)\"\n"
            "echo '{\"event\": \"sent\", \"to\": \"x\"}'\n")
os.chmod(FAKE_LINKD, 0o755)


def link_args():
    try:
        return open("/tmp/link-args").read().split()
    except OSError:
        return []


def open_sky():
    super_tab()
    return wait(lambda: sky()["open"] == 1, 3)


def close_sky():
    if sky().get("open"):
        if sky().get("query"):
            keys("-k", "Escape")
        keys("-k", "Escape")
    wait(lambda: sky()["open"] == 0, 2)


check(open("/tmp/sway-validate.rc").read().strip() == "0", "Sway accepts the session's Super+Tab and Alt+Tab bindings",
      open("/tmp/sway-validate.log").read()[-300:])

# ---- a home with two projects
os.makedirs(f"{HOME}/code/NexaOS/.git", exist_ok=True)
os.makedirs(f"{HOME}/code/NexaOS/tools", exist_ok=True)
write(f"{HOME}/code/NexaOS/tools/main.rs", "fn main() {}\n")
write(f"{HOME}/Documents/Thesis/ch1.odt", "x")
write(f"{HOME}/Documents/Thesis/fig1.png", "x")
write(f"{HOME}/notes.txt", "x")
os.makedirs(f"{HOME}/Downloads", exist_ok=True)
write(f"{HOME}/.local/share/recently-used.xbel",
      '<?xml version="1.0"?>\n<xbel version="1.0">\n'
      f'<bookmark href="file://{HOME}/code/NexaOS/tools/main.rs"></bookmark>\n'
      f'<bookmark href="file://{HOME}/Documents/Thesis/ch1.odt"></bookmark>\n</xbel>\n')
apps = f"{HOME}/.local/share/applications"
for app, name, icon in (("test-editor", "Editor", "accessories-text-editor"), ("test-term", "Terminal", "utilities-terminal"),
                        ("test-viewer", "Viewer", "image-x-generic"), ("test-files", "Files", "folder")):
    write(f"{apps}/{app}.desktop", f"[Desktop Entry]\nType=Application\nName={name}\nIcon={icon}\nExec=true\n")

orbit = subprocess.Popen(["nexa-orbit"], env=dict(os.environ, NEXA_CONSTELLATION_STATE=STATE, NEXA_LINKD=FAKE_LINKD),
                         stderr=open("/tmp/orbit.err", "w"))
time.sleep(1)
procs = []
def launch(app, title, *extra):
    procs.append(subprocess.Popen(["python3", STAR, app, title, *extra], cwd=HOME, stderr=subprocess.DEVNULL))
    wait(lambda: by_title(title), 8)

launch("test-files", "~/Downloads — Files")
launch("test-viewer", "fig1.png", "--hold", f"{HOME}/Documents/Thesis/fig1.png")
launch("test-editor", "ch1.odt — Editor")
launch("test-term", "ama@nexa: ~", "--child", f"{HOME}/code/NexaOS/tools")
launch("test-editor", "main.rs — Editor")
launch("test-editor", "notes.txt — Editor")
sway(f'[title="notes.txt — Editor"]', "move", "to", "workspace", "number", "3")
for t in ("~/Downloads — Files", "fig1.png", "ch1.odt — Editor", "ama@nexa: ~", "main.rs — Editor"):
    sway(f'[title="{t}"]', "focus")
    time.sleep(0.2)
check(len(windows()) == 6 and focused()["name"] == "main.rs — Editor", "six windows, main.rs in front", [w["name"] for w in windows()])
before = pss_kb(orbit.pid)

# ---- opening
check(open_sky(), "Super+Tab opens Constellation (the session's binding, through nexa-constellation)")
s = sky()
check(s["size"] == [W, H], "over the whole screen", s.get("size"))
check(len(s["stars"]) == 6, "a star for every window, the one on layer 3 too", len(s["stars"]))
check(sel()["title"] == "ama@nexa: ~", "it starts on the window used before the one in front (Enter swaps back)", sel())
eds = [x for x in s["stars"] if x["app"] == "Editor"]
check(len(eds) == 3 and len({x["group"] for x in eds}) == 1 and len({x["group"] for x in s["stars"]}) == 4,
      "an app's windows are one figure: 3 editors, 4 apps", [(x["title"], x["group"]) for x in s["stars"]])
check(star("main.rs — Editor")["group"] == 0 and abs(sum(x["x"] for x in eds) / 3 - W / 2) < 120,
      "the app used last in the middle", [(x["x"], x["y"]) for x in eds])
check(star("notes.txt — Editor")["layer"] == 3 and s["current"] == 1, "the window on layer 3 knows its layer")
check(wait(lambda: sky()["projects_known"] == 1, 4), "nexa-starmap answered")
s = sky()
proj = {x["title"]: x["project"] for x in s["stars"]}
check(proj["ama@nexa: ~"] == "NexaOS", "a terminal's shell folder: NexaOS (the .git above it)", proj)
check(proj["main.rs — Editor"] == "NexaOS", "an editor by its title and the recent files: NexaOS", proj)
check(proj["ch1.odt — Editor"] == "Thesis" and proj["fig1.png"] == "Thesis", "a recent file and a file held open: Thesis", proj)
check(proj["~/Downloads — Files"] == "" and proj["notes.txt — Editor"] == "",
      "no project for Downloads or a note in home", proj)
ids = {x["title"]: x["id"] for x in s["stars"]}
links = {frozenset(l) for l in s["links"]}
check(links == {frozenset((ids["ama@nexa: ~"], ids["main.rs — Editor"])), frozenset((ids["ch1.odt — Editor"], ids["fig1.png"]))},
      "a line between the windows of each project, no others", s["links"])
apart = all(abs(a["x"] - b["x"]) > 30 or abs(a["y"] - b["y"]) > 30 for a in s["stars"] for b in s["stars"] if a is not b)
check(apart and all(40 <= x["x"] <= W - 40 and 60 <= x["y"] <= H - 100 for x in s["stars"]), "stars apart and on screen",
      [(x["x"], x["y"]) for x in s["stars"]])
time.sleep(0.3)
showing = pss_kb(orbit.pid)
shot("constellation.png")

# ---- keys
keys("-k", "Tab")
check(sel()["title"] == "ch1.odt — Editor", "Tab: the next by last use", sel())
keys("-M", "shift", "-k", "Tab", "-m", "shift")
check(sel()["title"] == "ama@nexa: ~", "Shift+Tab: back", sel())
keys("-k", "Home")
check(sel()["title"] == "main.rs — Editor", "Home: the window in front", sel())
x0 = sel()
for k in ("Right", "Left", "Up", "Down"):
    keys("-k", k)
    x1 = sel()
    moved = {"Right": x1["x"] > x0["x"], "Left": x1["x"] < x0["x"], "Up": x1["y"] < x0["y"], "Down": x1["y"] > x0["y"]}[k]
    check(x1["id"] == x0["id"] or moved, f"{k}: to a star that way (or stays)", (x0["title"], x1["title"]))
    x0 = x1
keys("ch1")
s = sky()
check(s["query"] == "ch1" and [x["title"] for x in s["stars"] if x["on"]] == ["ch1.odt — Editor"] and sel()["title"] == "ch1.odt — Editor",
      "typing finds: “ch1”", (s["query"], [x["title"] for x in s["stars"] if x["on"]]))
shot("constellation-search.png")
keys("-k", "BackSpace", "-k", "BackSpace", "-k", "BackSpace")
check(sky()["query"] == "" and all(x["on"] for x in sky()["stars"]), "Backspace: everything again")
keys("thesis")
check([x["title"] for x in sky()["stars"] if x["on"]] == [] and sky()["open"], "a project's name isn't a title: nothing (and it stays open)")
keys("-k", "Escape")
check(sky()["open"] and sky()["query"] == "", "Escape: the search cleared first")
keys("ch1", "-k", "Return")
check(wait(lambda: sky()["open"] == 0, 2) and wait(lambda: focused()["name"] == "ch1.odt — Editor", 2),
      "Enter goes to the window", focused() and focused()["name"])

check(open_sky(), "open again")
keys("notes", "-k", "Return")
check(wait(lambda: layer() == 3 and focused()["name"] == "notes.txt — Editor", 3), "a window on layer 3: Enter goes to that layer",
      (layer(), focused() and focused()["name"]))
sway("workspace", "number", "1")
time.sleep(0.3)

check(open_sky(), "Super+Tab on layer 1 again")
keys("Downloads", "-k", "Delete")
check(wait(lambda: not by_title("~/Downloads — Files"), 3), "Delete closes the window")
check(wait(lambda: len(sky()["stars"]) == 5, 3), "and its star goes, while open", len(sky()["stars"]))
keys("-k", "Escape")
super_tab()
check(wait(lambda: sky()["open"] == 0, 2), "Super+Tab again closes it")

# ---- the pointer
check(open_sky(), "open for the pointer")
v = star("fig1.png")
subprocess.run([VP, str(W), str(H), "move", str(int(v["x"])), str(int(v["y"])), "wait", "150"], timeout=10)
check(wait(lambda: sel()["title"] == "fig1.png", 2), "pointing picks a star", sel())
subprocess.run([VP, str(W), str(H), "move", str(int(v["x"])), str(int(v["y"])), "wait", "60", "click"], timeout=10)
check(wait(lambda: sky()["open"] == 0, 2) and wait(lambda: focused()["name"] == "fig1.png", 2), "a click goes to it")
check(open_sky(), "open again")
stars = sky()["stars"]
spot = max(((x, y) for x in range(30, W - 30, 50) for y in range(80, H - 120, 50)),
           key=lambda p: min((p[0] - s["x"]) ** 2 + (p[1] - s["y"]) ** 2 for s in stars))
subprocess.run([VP, str(W), str(H), "move", str(spot[0]), str(spot[1]), "wait", "60", "click"], timeout=10)
check(wait(lambda: sky()["open"] == 0, 2), "a click on the sky leaves", spot)

# ---- Nexa Link: Ctrl+S sends the picked window's file
check(open_sky(), "open for Nexa Link")
files = {x["title"]: x["file"] for x in sky()["stars"]}
check(files["fig1.png"] == f"{HOME}/Documents/Thesis/fig1.png" and files["ch1.odt — Editor"] == f"{HOME}/Documents/Thesis/ch1.odt"
      and files["main.rs — Editor"] == f"{HOME}/code/NexaOS/tools/main.rs" and files["ama@nexa: ~"] == "",
      "each window's file: held open, a recent file's name; a terminal has none", files)
def ctrl_s():
    """Ctrl+S, pressed once more if the sky shows no change at all: a new
    wtype keyboard can lose its first key while GTK takes its key map (see
    keys()); a real keyboard doesn't."""
    before = (sky().get("link"), sky().get("choosing"))
    keys("-M", "ctrl", "-k", "s", "-m", "ctrl")
    if not wait(lambda: (sky().get("link"), sky().get("choosing")) != before, 1):
        keys("-M", "ctrl", "-k", "s", "-m", "ctrl")
keys("fig1")
ctrl_s()
check(wait(lambda: "No paired device is nearby" in sky()["link"], 2), "no device nearby: says where to pair one", sky().get("link"))
LINK_DEVICES[:] = [{"id": "dev-1", "name": "Ama's phone", "files": "ask", "online": True},
                   {"id": "dev-9", "name": "Old tablet", "files": "ask", "online": False}]
ctrl_s()
check(wait(lambda: sky()["link"] == "Sent fig1.png to Ama's phone.", 4), "one device nearby: sent to it", sky().get("link"))
check(link_args() == ["send", "dev-1", f"{HOME}/Documents/Thesis/fig1.png", "--json"], "nexa-linkd send DEVICE FILE (an offline one isn't asked)",
      link_args())
LINK_DEVICES.append({"id": "dev-2", "name": "Office PC", "files": "always", "online": True})
ctrl_s()
check(wait(lambda: sky()["choosing"] == 2 and "1 Ama's phone" in sky()["link"] and "2 Office PC" in sky()["link"], 2),
      "two nearby: which one, by number", sky().get("link"))
keys("2")
check(wait(lambda: link_args()[:2] == ["send", "dev-2"] and sky()["link"].startswith("Sent fig1.png to Office PC"), 4), "2: the Office PC",
      (link_args(), sky().get("link")))
keys("-k", "Escape")
keys("ama")
ctrl_s()
check(wait(lambda: "can't tell which file" in sky()["link"], 2), "a window with no file: says so", sky().get("link"))
shot("constellation-link.png")
keys("-k", "Escape")
# a send still going when the sky closes: its result comes as a Signal
with open("/tmp/link-slow", "w") as f:
    f.write("2")
LINK_DEVICES[:] = LINK_DEVICES[:1]
keys("fig1")
ctrl_s()
check(wait(lambda: "Sending fig1.png" in sky()["link"], 2), "sending...", sky().get("link"))
close_sky()
def link_signal():
    r = subprocess.run(["nexa-signalctl", "history"], capture_output=True, text=True).stdout
    try:
        return [i for i in json.loads(r)["data"][0] if i["app-name"]["data"] == "Nexa Link"]
    except (ValueError, KeyError, IndexError):
        return []
check(wait(lambda: link_signal() and link_signal()[0]["body"]["data"] == "Sent fig1.png to Ama's phone.", 6),
      "finished after the sky closed: a Signal says so", link_signal())

# ---- needs you; the theme, live
sway('[title="main.rs — Editor"]', "urgent", "enable")
check(open_sky(), "open")
check(wait(lambda: star("main.rs — Editor")["urgent"] == 1, 2), "a window that needs you is marked")
far = sorted(((x, y) for x in range(30, W - 60, 40) for y in range(80, H - 140, 40)),
             key=lambda p: -min((p[0] - s["x"]) ** 2 + (p[1] - s["y"]) ** 2 for s in sky()["stars"]))[0]
dark = pixel(*far)
subprocess.run(["python3", "-c", "import sys; sys.path.insert(0, '/src/common'); from nexa_common import themes; themes.apply('daybreak', reload=False)"],
               capture_output=True)
check(wait(lambda: sum(pixel(*far)) / 3 > 170, 4), "a light theme lights the sky, while open", (dark, pixel(*far)))
shot("constellation-daybreak.png")
close_sky()
after = pss_kb(orbit.pid)
def rounds(n):
    for _ in range(n):
        open_sky()
        time.sleep(0.3)
        close_sky()
    time.sleep(1)
    return private_kb(orbit.pid)
own5 = rounds(5)
own15 = rounds(10)
again = pss_kb(orbit.pid)
check(own15 - own5 < 1024, "opened and closed 10 more times: its own memory stays level (no leak)", f"{own5} kB then {own15} kB")

# ---- the graph program on its own
lines = "".join(f"{w['id']}\t{w['pid']}\t{w['app_id']}\t{w['name']}\n" for w in windows())
t0 = time.time()
for _ in range(5):
    r = subprocess.run(["nexa-starmap"], input=lines, capture_output=True, text=True, timeout=5)
ms = (time.time() - t0) / 5 * 1000
check(r.returncode == 0 and len(json.loads(r.stdout)["windows"]) == 5, "nexa-starmap on its own", r.stderr[-200:])
print(f"INFO Orbit PSS {before / 1024:.1f} MB closed, {showing / 1024:.1f} MB with the sky open, {after / 1024:.1f} MB after, {again / 1024:.1f} MB after 15 more "
      f"(its own memory {own5 / 1024:.1f} MB, then {own15 / 1024:.1f} MB); "
      f"nexa-starmap {ms:.1f} ms for {len(windows())} windows", flush=True)

# ---- reduced motion (GTK's setting, read by Orbit at start)
orbit.terminate()
orbit.wait(5)
write(os.path.expanduser("~/.config/glib-2.0/settings/keyfile"), "[org/gnome/desktop/interface]\nenable-animations=false\n")
os.remove(STATE)
orbit = subprocess.Popen(["nexa-orbit"], env=dict(os.environ, NEXA_CONSTELLATION_STATE=STATE), stderr=open("/tmp/orbit.err", "a"))
time.sleep(1)
check(open_sky() and sky()["animations"] == 0, "with reduced motion: no flight (GTK's setting)")
close_sky()

orbit.terminate()
orbit.wait(5)
r = subprocess.run(["nexa-constellation"], capture_output=True, text=True)
check(r.returncode == 1 and "Orbit isn't running" in r.stderr, "without Orbit: says so", r.stderr)
for p in procs:
    p.terminate()
err = open("/tmp/orbit.err").read()
bad = [l for l in err.splitlines() if ("CRITICAL" in l or "WARNING" in l) and "Glycin" not in l]
check(not bad, "no GTK warnings or criticals", "\n".join(bad[:5]))
print(f"{fails} failed")
sys.exit(1 if fails else 0)
