"""Backup in a headless Sway (scripts/test-backup-app.sh), started the way an
installed app is: through nexa-run, confined to files.home and files.drives.
Real drives: tmpfs filesystems mounted under /media (one big, one too
small, one read-only), and a stand-in broker that opens a drive that isn't
open yet. Driven through BACKUP_TEST; screenshots go to $OUT."""
import json
import os
import shutil
import socket
import subprocess
import sys
import threading
import time

OUT = os.environ.get("OUT", "/tmp")
SMOKE = bool(os.environ.get("SMOKE"))
HOME = os.environ["HOME"]
T = "/tmp/bt"
STATE, CMD = os.path.join(T, "state.json"), os.path.join(T, "cmd")
DRIVES_JSON, BROKER = "/tmp/drives.json", "/tmp/broker.sock"
fails = 0
proc = None


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"), flush=True)
    fails += not ok


def state():
    try:
        return json.load(open(STATE))
    except (OSError, ValueError):
        return {}


def wait(cond, secs=10.0):
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
    time.sleep(0.2)


def shot(name):
    time.sleep(0.5)
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def put(rel, data):
    p = os.path.join(HOME, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "wb" if isinstance(data, bytes) else "w") as f:
        f.write(data)


def mount(name, size, ro=False):
    d = f"/media/{name}"
    os.makedirs(d, exist_ok=True)
    subprocess.run(["mount", "-t", "tmpfs", "-o", f"size={size}" + (",ro" if ro else ""), "tmpfs", d], check=True)


def drive_index(label):
    return next(i for i, d in enumerate(state()["drives"]) if d["label"] == label)


def done(s):
    return not s.get("busy") and s.get("result")


def backup(secs=60):
    """Back up and wait for that run to finish."""
    n = state().get("runs", 0)
    send("backup")
    return wait(lambda s: s["runs"] == n + 1 and not s["busy"], secs)


def put_and_wait(*cmds, secs=30):
    n = state().get("puts", 0)
    send(*cmds)
    return wait(lambda s: s["puts"] == n + 1 and not s["busy"], secs)


# ---- a stand-in broker: mount_drive opens the stick
def broker():
    srv = socket.socket(socket.AF_UNIX)
    if os.path.exists(BROKER):
        os.remove(BROKER)
    srv.bind(BROKER)
    os.chmod(BROKER, 0o666)
    srv.listen(4)
    while True:
        c, _ = srv.accept()
        req = json.loads(c.makefile().readline())
        if req.get("action") == "mount_drive" and req["args"]["name"] == "sdc1":
            mount("NEW_STICK", "64m")
            json.dump({"drives": [{"name": "sdc1", "label": "NEW STICK", "size_bytes": 1, "mountpoint": "/media/NEW_STICK", "fstype": "vfat"}]},
                      open(DRIVES_JSON, "w"))
            c.sendall(b'{"ok": true, "result": {"mountpoint": "/media/NEW_STICK"}}\n')
        else:
            c.sendall(b'{"ok": false, "error": "no"}\n')
        c.close()


threading.Thread(target=broker, daemon=True).start()
os.makedirs(T, exist_ok=True)
if not os.path.exists(CMD):
    os.mkfifo(CMD)

# ---- the drives and the home
mount("BACKUPS", "200m")
mount("TINY", "1m")
mount("LOCKED", "16m", ro=True)
json.dump({"drives": [{"name": "sdc1", "label": "NEW STICK", "size_bytes": 1, "mountpoint": None, "fstype": "vfat"}]}, open(DRIVES_JSON, "w"))
put("Documents/cv.txt", "my cv, version 1")
put("Documents/notes.md", "# notes\n")
put("Documents/School/essay.txt", "an essay")
put("Pictures/beach.jpg", os.urandom(300_000))
put("Music/song.flac", os.urandom(4_000_000))
put("Downloads/big.iso", os.urandom(3_000_000))
put(".config/someapp/settings.conf", "dark=yes")
put(".ssh/id_ed25519", "SECRET KEY DO NOT COPY")
put(".cache/junk.bin", os.urandom(100_000))
put("Desktop/it's \"odd\" & spaced.txt", "odd")


def start():
    global proc
    if os.path.exists(STATE):
        os.remove(STATE)
    proc = subprocess.Popen(["nexa-run", "backup"], env=dict(os.environ, BACKUP_TEST=T, BACKUP_DRIVES_JSON=DRIVES_JSON,
                                                             NEXA_BROKER_SOCKET=BROKER), stderr=open("/tmp/backup.err", "a"))
    return wait(lambda s: s.get("drives"), 15)


def stop():
    proc.terminate()
    proc.wait(5)


if SMOKE:
    check(start(), "from the package: it starts and finds the drives")
    send(f"drive {drive_index('BACKUPS')}")
    check(backup(30) and state()["snaps"] == 1, "and backs up", state().get("result"))
    subprocess.run(["grim", os.path.join(OUT, "backup-from-package.png")])
    stop()
    print(f"{fails} failed")
    sys.exit(1 if fails else 0)

explain = subprocess.run(["nexa-run", "--explain", "backup"], capture_output=True, text=True).stdout
check("files.home" in explain and "files.drives" in explain and "no internet" in explain,
      "nexa-run: Backup may use the home folder and USB drives, no internet", explain)
check(start(), "starts, confined")
s = state()
labels = [d["label"] for d in s["drives"]]
check({"BACKUPS", "TINY", "LOCKED", "NEW STICK"} <= set(labels), "every drive under /media, and one not open yet", labels)
check(next(d for d in s["drives"] if d["label"] == "LOCKED")["ro"] and not next(d for d in s["drives"] if d["label"] == "NEW STICK")["mounted"],
      "the read-only one and the closed one are told apart")
send(f"drive {drive_index('BACKUPS')}")
check(wait(lambda s: s["head"] == "No backup on BACKUPS yet" and s["can_back_up"]), "BACKUPS chosen: no backup yet", state().get("head"))
shot("backup-start.png")

# ---- what to back up
send("page 1")
time.sleep(1.5)  # the sizes are measured meanwhile
send("off Downloads")
check(wait(lambda s: s["off"] == ["Downloads"]), "What to Back Up: Downloads switched off")
shot("backup-choice.png")
send("page 0")

# ---- the first backup
check(backup(60), "Back Up Now: it runs and finishes", state().get("result"))
s = state()
check(s["result"].startswith("Backed up.") and s["snaps"] == 1 and s["status"] == 1 and s["head"].startswith("Last backup: today"),
      "backed up: today, with the hexagon's tick", f"{s['result']} / {s['head']}")
root = "/media/BACKUPS/NexaOS Backup/root"
snaps = os.listdir(os.path.join(root, "snapshots"))
listing = open(os.path.join(root, "snapshots", snaps[0])).read()
check("Documents/cv.txt" in listing and ".config/someapp/settings.conf" in listing and "Desktop/it" in listing,
      "files, settings and odd names in the backup")
check(".ssh" not in listing and ".cache" not in listing and "Downloads" not in listing,
      "no secret key, no cache, not the switched-off Downloads")
secret_copied = subprocess.run(["grep", "-rl", "SECRET KEY DO NOT COPY", "/media/BACKUPS"], capture_output=True).stdout
check(not secret_copied, "the secret key's contents are nowhere on the drive")
shot("backup-done.png")

# ---- the next one copies only what changed
time.sleep(1.1)
put("Documents/cv.txt", "my cv, version 2!")
put("Documents/new.txt", "new")
check(backup(30) and state()["snaps"] == 2, "a second backup")
check("Copied 2 new or changed files" in state()["result"], "copies only the 2 changed files", state()["result"])

# ---- stopping halfway
put("Videos/long.bin", os.urandom(60_000_000))
n = state()["runs"]
send("backup")
wait(lambda s: s["busy"], 3)
time.sleep(0.3)
send("stop")
check(wait(lambda s: s["runs"] == n + 1, 30) and state()["result"].startswith("Stopped") and state()["snaps"] == 2,
      "Stop: no half backup, the others stay", state().get("result"))
os.remove(os.path.join(HOME, "Videos/long.bin"))

# ---- a drive too small, a read-only one, one not open yet
send(f"drive {drive_index('TINY')}")
check(backup(30) and "There isn't room on the drive" in state()["result"], "a drive too small: said so before starting",
      state().get("result"))
shot("backup-full.png")
send(f"drive {drive_index('LOCKED')}")
time.sleep(0.5)
check(not state()["can_back_up"], "a read-only drive can't be backed up to")
send(f"drive {drive_index('NEW STICK')}")
check(wait(lambda s: any(d["label"] == "NEW_STICK" and d["mounted"] for d in s["drives"]), 8),
      "a drive not open yet: opened through the broker", [d["label"] for d in state()["drives"]])

# ---- putting back
send(f"drive {drive_index('BACKUPS')}", "page 2")
check(wait(lambda s: s["page"] == "put" and s["snaps"] == 2), "Put Back: the two backups")
send("snap 1")  # the older one
check(wait(lambda s: s["snap"] and "Documents" in s["entries"]), "its folders", state().get("entries"))
send("folder Documents")
check(wait(lambda s: "Documents/cv.txt" in s["entries"] and "Documents/School" in s["entries"]), "inside a folder", state().get("entries"))
send("tick Documents/cv.txt")
check(wait(lambda s: s["put_btn"] == "Put Back Ticked"), "ticking a file: Put Back Ticked")
check(put_and_wait("where 0", "put") and state()["put_msg"].startswith("Put back 1 item where"), "put back where it was", state().get("put_msg"))
check(open(os.path.join(HOME, "Documents/cv.txt")).read() == "my cv, version 2!" and
      open(os.path.join(HOME, "Documents/cv (restored).txt")).read() == "my cv, version 1",
      "the changed one kept, the old one beside it")
shot("backup-putback.png")
os.remove(os.path.join(HOME, "Documents/School/essay.txt"))
check(put_and_wait("snap 0", "where 1", "put") and state()["put_msg"].startswith("Put back") and "into" in state()["put_msg"], "everything into a new folder", state().get("put_msg"))
restored = [d for d in os.listdir(HOME) if d.startswith("Restored ")]
check(restored and open(os.path.join(HOME, restored[0], "Documents/School/essay.txt")).read() == "an essay"
      and open(os.path.join(HOME, restored[0], "Music/song.flac"), "rb").read() == open(os.path.join(HOME, "Music/song.flac"), "rb").read(),
      "it's all there, byte for byte", restored)
send("search essay")
check(wait(lambda s: s["entries"] == ["Documents/School/essay.txt"]), "find a file in a backup", state().get("entries"))
send("search ")

# ---- checking, and damage
check(put_and_wait("check") and "every file in this backup reads back exactly" in state()["put_msg"], "Check: all fine", state().get("put_msg"))
objs = os.path.join(root, "objects")
newest = max(os.listdir(os.path.join(root, "snapshots")))
h = next(l.split("\t")[4] for l in open(os.path.join(root, "snapshots", newest)) if l.startswith("F\t") and "Documents/notes.md" in l)
victim = os.path.join(objs, h[:2], h)
with open(victim, "r+b") as f:
    f.write(b"XX")
check(put_and_wait("check") and "damaged or missing" in state()["put_msg"], "Check finds a damaged copy", state().get("put_msg"))

# ---- deleting a backup
send("snap 1", "delete")
check(wait(lambda s: s["snaps"] == 1 and "deleted" in s["put_msg"]), "Delete This Backup", state().get("put_msg"))
check(proc.poll() is None, "still running after all of it")
stop()

err = open("/tmp/backup.err").read()
bad = [l for l in err.splitlines() if ("CRITICAL" in l or "WARNING" in l) and "Glycin" not in l]
check(not bad, "no GTK warnings or criticals", "\n".join(bad[:5]))
print(f"{fails} failed")
sys.exit(1 if fails else 0)
