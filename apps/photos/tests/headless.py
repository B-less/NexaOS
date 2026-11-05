"""Photos in a headless Sway (scripts/test-photos-app.sh), started the way an
installed app is: through nexa-run, confined to files.pictures. Driven
through PHOTOS_TEST (state out, commands in). Screenshots go to $OUT."""
import json
import os
import shutil
import subprocess
import sys
import time

OUT = os.environ.get("OUT", "/tmp")
HOME = os.environ["HOME"]
PICS = os.path.join(HOME, "Pictures")
T = "/tmp/pt"
STATE, CMD = os.path.join(T, "state.json"), os.path.join(T, "cmd")
fails = 0
proc = None


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"), flush=True)
    fails += not ok


def state():
    try:
        with open(STATE) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def wait(cond, secs=8.0):
    end = time.time() + secs
    while time.time() < end:
        try:
            if cond(state()):
                return True
        except (KeyError, TypeError, IndexError):
            pass
        time.sleep(0.05)
    return False


def send(*lines):
    with open(CMD, "w") as f:
        for line in lines:
            f.write(line + "\n")
    time.sleep(0.15)


def shot(name):
    time.sleep(0.4)
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def start(*args, env=None):
    global proc
    if os.path.exists(STATE):
        os.remove(STATE)
    e = dict(os.environ, PHOTOS_TEST=T, **(env or {}))
    proc = subprocess.Popen(["nexa-run", "photos", *args], env=e, stderr=open("/tmp/photos.err", "a"))
    return wait(lambda s: s.get("scanned"), 20)


def stop():
    proc.terminate()
    proc.wait(5)


def viewer(s):
    return s.get("viewer") or {}


os.makedirs(T, exist_ok=True)
if not os.path.exists(CMD):
    os.mkfifo(CMD)

# ---- installed and confined
explain = subprocess.run(["nexa-run", "--explain", "photos"], capture_output=True, text=True).stdout
check("files.pictures" in explain and "no internet" in explain, "nexa-run: Photos may use Pictures, no internet", explain)

# ---- the empty state
os.makedirs(PICS, exist_ok=True)
check(start(), "starts, with nothing in Pictures")
s = state()
check("No photos yet" in s.get("empty", "") and s.get("library") == 0, "the empty state says what to do", s.get("empty"))
shot("photos-empty.png")
stop()

# ---- a library
subprocess.run([sys.executable, "/src/apps/photos/tests/make_photos.py", PICS], check=True)
os.makedirs(os.path.join(HOME, "Documents"), exist_ok=True)
shutil.copy(os.path.join(PICS, "old-1.jpg"), os.path.join(HOME, "Documents", "secret.jpg"))
t0 = time.time()
check(start(), "starts with a library")
first_scan = time.time() - t0
s = state()
check(s.get("library") == 28, "28 pictures found (not the empty, text or hidden ones)", s.get("library"))
check(s.get("months") == ["November 2026", "October 2026", "August 2026", "March 2025", "January 2024"],
      "by month, newest first (a picture without a date inside: its file's date)", s.get("months"))
check(s.get("sub") == "28 photos in ~/Pictures", "the heading counts them", s.get("sub"))
check("secret.jpg" not in s.get("names", []), "nothing from outside Pictures")
check(wait(lambda s: s["wanted"] == 0 and s["thumbs"] >= 20, 20), "thumbnails made", f"{state().get('thumbs')} made")
print(f"INFO first start: library read in {first_scan:.2f} s")
shot("photos-library.png")
stop()

t0 = time.time()
start()
check(wait(lambda s: s["wanted"] == 0 and s["thumbs"] >= 20, 10), "next start: thumbnails from the cache")
print(f"INFO next start: library and thumbnails in {time.time() - t0:.2f} s")
check(os.path.exists(os.path.join(HOME, ".cache/photos/index")) and os.listdir(os.path.join(HOME, ".cache/photos/thumbs")),
      "index and thumbnails kept in ~/.cache/photos")

# ---- the viewer
names = state()["names"]
beach = names.index("beach-01.jpg")
send(f"tile {beach}")
check(wait(lambda s: s["page"] == "viewer" and viewer(s).get("loaded")), "a click opens the viewer")
v = viewer(state())
check(v.get("name") == "beach-01.jpg" and v.get("title") == "beach-01.jpg" and v.get("pic") == [1200, 800], "the photo, full size", v)
send("key i")
check(wait(lambda s: viewer(s).get("info")), "I: details")
time.sleep(0.3)
check(900 <= viewer(state())["area"][0] <= 960, "the details panel keeps to its width (about 320 px)", viewer(state()).get("area"))
info = viewer(state()).get("info_text", "")
check("Camera: NexaCam Hex 1" in info and "Settings: f/2.8 · 1/250 s · ISO 200 · 35 mm" in info, "camera and settings", info)
check("Place: 5.6037° N, 0.1870° W" in info and "Taken: Tuesday 20 October 2026, 10:00" in info and
      "Folder: Pictures/Holiday" in info, "when, where, which folder", info)
shot("photos-viewer.png")
send("key i")
send("key Right")
check(wait(lambda s: viewer(s).get("index") == beach + 1 and viewer(s).get("loaded")), "Right: the next photo")
send("key End")
check(wait(lambda s: viewer(s).get("index") == 27), "End: the last")
send("key Home")
check(wait(lambda s: viewer(s).get("index") == 0), "Home: the first")
send(f"key Right", f"key Right")
wait(lambda s: viewer(s).get("loaded"))
send("key 1")
check(wait(lambda s: abs(viewer(s)["scale"] * viewer(s)["ratio"] - 1) < 0.01), "1: actual size",
      viewer(state()))
send("key plus")
check(wait(lambda s: viewer(s)["scale"] * viewer(s)["ratio"] > 1.2), "+: zoom in")
send("key 0")
check(wait(lambda s: viewer(s)["fit"]), "0: back to fit")

# Stored on its side: shown upright; turning it is remembered, the file untouched.
send("key Escape")
wait(lambda s: s["page"] == "grid")
turned = state()["names"].index("turned.jpg")
before = open(os.path.join(PICS, "turned.jpg"), "rb").read()
send(f"tile {turned}")
check(wait(lambda s: viewer(s).get("loaded") and viewer(s)["pic"] == [800, 1200]), "a photo stored on its side is shown upright",
      viewer(state()))
shot("photos-upright.png")
send("key r")
check(wait(lambda s: viewer(s).get("loaded") and viewer(s)["pic"] == [1200, 800] and viewer(s)["orientation"] == 1),
      "R: turned right", viewer(state()))
check(open(os.path.join(PICS, "turned.jpg"), "rb").read() == before, "the file isn't changed")

# Favourite
send("key f")
check(wait(lambda s: viewer(s).get("fav") and "turned.jpg" in s["favourites"]), "F: a favourite")
send("key Escape")
wait(lambda s: s["page"] == "grid")
stop()
start()
send(f"tile {state()['names'].index('turned.jpg')}")
check(wait(lambda s: viewer(s).get("loaded") and viewer(s)["pic"] == [1200, 800] and viewer(s)["fav"]),
      "the turn and the favourite are still there next time")
send("key Escape")
wait(lambda s: s["page"] == "grid")

# ---- selecting, albums
names = state()["names"]
g1 = names.index("garden-01.jpg")
send(f"ctrltile {g1}", f"ctrltile {g1 + 1}", f"shifttile {g1 + 4}")
check(wait(lambda s: s["selected"] == 5), "Ctrl+click and Shift+click select", state().get("selected"))
shot("photos-selected.png")
send("addto Garden days")
check(wait(lambda s: s["albums"] == [{"name": "Garden days", "count": 5}] and "Added 5 photos to Garden days." in s["toast"]),
      "Add to Album: a new album with them", state().get("albums"))
check(state()["selected"] == 0, "and the selection is done")
send(f"ctrltile {names.index('beach-03.jpg')}", "addto Garden days")
check(wait(lambda s: s["albums"][0]["count"] == 6), "more added to it")
send("page albums")
check(wait(lambda s: s["page"] == "albums"), "Albums page")
wait(lambda s: not s["toast"], 9)
shot("photos-albums.png")
send("page library", "addto  ")
check(wait(lambda s: "Type a name" in s["toast"]), "an album needs a name", state().get("toast"))
send("album Garden days")
check(wait(lambda s: s["page"] == "grid" and s["heading"] == "Garden days" and s["shown"] == 6), "an album lists its photos")
send("page favourites")
check(wait(lambda s: s["heading"] == "Favourites" and s["names"] == ["turned.jpg"]), "Favourites")
send("page library")

# ---- search
send("search holiday")
check(wait(lambda s: s["shown"] == 11 and s["heading"] == "Results for “holiday”"), "search: a folder", state().get("shown"))
send("search october 2026")
check(wait(lambda s: s["shown"] == 12), "a month", state().get("shown"))
send("search garden days")
check(wait(lambda s: s["shown"] == 6), "an album's name", state().get("shown"))
send("search nothing-like-this")
check(wait(lambda s: s["shown"] == 0 and "No photos match" in s["empty"]), "nothing found: said so")
shot("photos-search-empty.png")
send("search ")
check(wait(lambda s: s["shown"] == 28 and s["heading"] == "Library"), "cleared: the library again")

# ---- deleting, Undo, Deleted
names = state()["names"]
o1, o2 = names.index("old-1.jpg"), names.index("old-2.jpg")
send(f"ctrltile {o1}", f"ctrltile {o2}", "key Delete")
check(wait(lambda s: s["library"] == 26 and "Moved 2 photos to Deleted." in s["toast"] and s["undo"]), "Delete: 2 moved to Deleted")
check(not os.path.exists(os.path.join(PICS, "old-1.jpg")) and os.listdir(os.path.join(HOME, ".local/share/photos/deleted")),
      "out of Pictures, into Photos' own Deleted folder")
send("key z ctrl")
check(wait(lambda s: s["library"] == 28 and s["deleted"] == 0, 10), "Ctrl+Z: put back", state().get("library"))
check(os.path.exists(os.path.join(PICS, "old-1.jpg")), "back in Pictures")
wait(lambda s: s["page"] == "grid" and s["shown"] == 28)
send(f"tile {state()['names'].index('old-3.jpg')}")
wait(lambda s: viewer(s).get("loaded"))
send("key Delete")
check(wait(lambda s: s["deleted"] == 1 and viewer(s).get("name") != "old-3.jpg"), "Delete in the viewer: the next photo shows")
send("key Escape", "page deleted")
check(wait(lambda s: s["page"] == "deleted" and s["deleted"] == 1), "Deleted lists it")
shot("photos-deleted.png")
send("restore")
check(wait(lambda s: s["deleted"] == 0 and s["library"] == 28, 10), "Put Back")
send("page library")
send(f"ctrltile {state()['names'].index('old-4.jpg')}", "key Delete")
wait(lambda s: s["deleted"] == 1)
send("empty")
check(wait(lambda s: s["deleted"] == 0) and not os.listdir(os.path.join(HOME, ".local/share/photos/deleted")),
      "Empty Deleted: gone for good")
send("page library")
wait(lambda s: s["library"] == 27)

# ---- slideshow
send("slideshow")
check(wait(lambda s: viewer(s).get("slideshow") and s["fullscreen"]), "Slideshow: full screen")
i0 = viewer(state()).get("index")
check(wait(lambda s: viewer(s).get("index") == i0 + 1, 6), "the next photo after 4 seconds")
shot("photos-slideshow.png")
send("key space")
check(wait(lambda s: viewer(s).get("paused")), "Space pauses")
send("key Escape")
check(wait(lambda s: not viewer(s).get("slideshow") and not s["fullscreen"]), "Esc ends it")
send("key Escape")

# ---- hostile pictures
names = state()["names"]
send(f"tile {names.index('huge.jpg')}")
check(wait(lambda s: "too big" in (viewer(s).get("why") or "")), "a picture claiming to be 60000 x 60000: refused, in words",
      viewer(state()))
v = viewer(state())
check(v["area"][0] == 1280 and 0 < v["msg_at"][0] < 640, "the viewer fills the window, the message in its middle", v)
shot("photos-error.png")
send("key Escape")
wait(lambda s: s["page"] == "grid")
send(f"tile {state()['names'].index('cut-short.jpg')}")
check(wait(lambda s: viewer(s).get("loaded") or viewer(s).get("why")), "a cut-short JPEG: shown or explained, no crash")
send("key Escape")
wait(lambda s: s["page"] == "grid")
send(f"tile {state()['names'].index('really-a-png.jpg')}")
check(wait(lambda s: viewer(s).get("loaded") and viewer(s)["pic"] == [640, 480]), "a PNG named .jpg is shown")
send("key Escape")
wait(lambda s: s["page"] == "grid")
q = "it's \"quoted\" & spaced.jpg"
send(f"tile {state()['names'].index(q)}")
check(wait(lambda s: viewer(s).get("loaded") and viewer(s)["name"] == q), "a name with quotes and spaces")
send("key Escape")
check(proc.poll() is None, "still running after all of them")

# ---- new photos appear by themselves
shutil.copy(os.path.join(PICS, "Holiday", "beach-02.jpg"), os.path.join(PICS, "new one.jpg"))
check(wait(lambda s: s["library"] == 28 and "new one.jpg" in s["names"], 8), "a photo copied in shows up by itself")
stop()

# ---- a file from outside Pictures, opened with Photos
start(os.path.join(HOME, "Documents", "secret.jpg"))
check(wait(lambda s: viewer(s).get("loaded") and viewer(s)["name"] == "secret.jpg" and viewer(s)["count"] == 1),
      "a file opened with Photos from elsewhere is shown (only that one)")
check(state()["library"] == 28, "but isn't added to the library")
stop()

# ---- confined: another folder stays shut
start()
probe = os.path.join(HOME, "Documents", "secret.jpg")
send("search secret")
check(wait(lambda s: s["shown"] == 0), "Documents stays out of reach")
stop()

err = open("/tmp/photos.err").read()
bad = [l for l in err.splitlines() if ("CRITICAL" in l or "WARNING" in l) and "Glycin" not in l]
check(not bad, "no GTK warnings or criticals", "\n".join(bad[:5]))
print(f"{fails} failed")
sys.exit(1 if fails else 0)
