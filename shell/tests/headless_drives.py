"""USB drives on the desktop (nexa_shell drives) against a stand-in for the
broker's /run/nexa/drives.json: a stick appears, a second one, it goes,
none left (the layer hides), and a theme change redraws the tiles.
Run by scripts/test-desktop-drives.sh."""
import json, os, subprocess, sys, tempfile, time
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "shell"), os.path.join(REPO, "common")]
state_dir = tempfile.mkdtemp()
os.environ["NEXA_DRIVES_STATE"] = os.path.join(state_dir, "drives.json")
import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402
from nexa_shell import drives  # noqa: E402

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)

def pump(s=0.6):
    end = time.time() + s
    while time.time() < end:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        time.sleep(0.02)

def publish(ds):
    tmp = drives.STATE + ".tmp"
    with open(tmp, "w") as f:
        json.dump({"drives": ds}, f)
    os.replace(tmp, drives.STATE)
    pump(1.0)

def names(win):
    out = []
    for btn in win.box.get_children():
        def walk(w):
            if isinstance(w, Gtk.Label) and w.get_style_context().has_class("nx-drive-name"):
                out.append(w.get_text())
            if isinstance(w, Gtk.Container):
                w.foreach(walk)
        walk(btn)
    return out

stick = {"name": "sdb1", "label": "PHOTOS", "size_bytes": 32 * 1024**3, "mountpoint": "/media/PHOTOS", "fstype": "vfat"}
card = {"name": "mmcblk0p1", "label": "", "size_bytes": 64 * 1024**3, "mountpoint": None, "fstype": "exfat"}
win = drives.DesktopDrives()
pump()
check("nothing plugged in: nothing on the desktop", not win.get_visible())
publish([stick])
check("a stick plugged in shows on the desktop, named", win.get_visible() and names(win) == ["PHOTOS"], str(names(win)))
subprocess.run(["grim", os.path.join(os.environ.get("OUT", "/tmp"), "drives-one.png")])
publish([stick, card])
check("an SD card without a label is called SD card", names(win) == ["PHOTOS", "SD card"], str(names(win)))
subprocess.run(["grim", os.path.join(os.environ.get("OUT", "/tmp"), "drives-two.png")])
before = win.accent
subprocess.run([sys.executable, "-c", f"import sys; sys.path.insert(0, '{REPO}/common'); from nexa_common import themes; themes.apply('aurora', reload=False)"], check=True)
pump(1.5)
check("a theme change recolours the drive icons", win.accent != before, f"{before} -> {win.accent}")
subprocess.run(["grim", os.path.join(os.environ.get("OUT", "/tmp"), "drives-theme.png")])
publish([card])
check("the stick pulled out goes", names(win) == ["SD card"], str(names(win)))
publish([])
check("nothing left: the layer hides", not win.get_visible())
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
