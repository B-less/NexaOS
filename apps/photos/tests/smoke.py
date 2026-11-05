"""Photos run from its extracted package, through nexa-run: it starts, finds
the pictures, makes thumbnails and opens one (scripts/test-photos-app.sh)."""
import json
import os
import subprocess
import sys
import time

T, HOME = "/tmp/pt", os.environ["HOME"]
os.makedirs(T, exist_ok=True)
os.mkfifo(os.path.join(T, "cmd"))
subprocess.run([sys.executable, "/src/apps/photos/tests/make_photos.py", os.path.join(HOME, "Pictures")], check=True)


def state():
    try:
        return json.load(open(os.path.join(T, "state.json")))
    except (OSError, ValueError):
        return {}


def wait(cond, secs=20):
    end = time.time() + secs
    while time.time() < end:
        try:
            if cond(state()):
                return True
        except (KeyError, TypeError):
            pass
        time.sleep(0.05)
    return False


p = subprocess.Popen(["nexa-run", "photos"], env=dict(os.environ, PHOTOS_TEST=T))
ok = wait(lambda s: s["scanned"] and s["library"] == 28 and s["wanted"] == 0 and s["thumbs"] >= 20)
print(("PASS" if ok else "FAIL") + " from the package: 28 photos and their thumbnails")
with open(os.path.join(T, "cmd"), "w") as f:
    f.write("tile 1\n")
ok2 = wait(lambda s: (s.get("viewer") or {}).get("loaded"))
print(("PASS" if ok2 else "FAIL") + " a photo opens")
subprocess.run(["grim", "/out/photos-from-package.png"])
p.terminate()
sys.exit(0 if ok and ok2 else 1)
