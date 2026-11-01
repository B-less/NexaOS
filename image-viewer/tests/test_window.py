"""The Image Viewer window, driven like a person would, against the real
engines (build/native: nexa-imageinfo, nexa-fileops, nexa-thumb): photos the
right way up, zoom and pan, lossless turning with undo, turning on screen and
saving a copy, the Trash with undo, details, the strip, animations, slideshow,
broken files.

Needs a display: run it with scripts/test-viewer-app.sh."""
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time

HOME = tempfile.mkdtemp(prefix="viewer-home-")
os.environ["HOME"] = HOME
for v in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"):
    os.environ.pop(v, None)
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "image-viewer"), os.path.join(REPO, "common")]

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, GdkPixbuf, Gtk  # noqa: E402

from nexa_viewer import engine, window as W  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def pump(seconds=0.2, until=None):
    end = time.time() + seconds
    while time.time() < end or (until and not until() and time.time() < end + 10):
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        if until and until() and time.time() >= end:
            return
        time.sleep(0.01)


def tiff(orientation):
    """EXIF (little-endian TIFF): make, model, orientation, GPS, exposure..."""
    e = lambda tag, kind, count, val: struct.pack("<HHII", tag, kind, count, val)  # noqa: E731
    make, model = b"NexaCam\0", b"NexaCam Hex 1\0"
    ifd0 = 8
    data0 = ifd0 + 2 + 5 * 12 + 4
    sub = data0 + len(make) + len(model)
    rats = sub + 2 + 4 * 12 + 4
    date = b"2026:10:31 18:04:05\0"
    b = b"II*\0" + struct.pack("<I", ifd0) + struct.pack("<H", 5)
    b += e(0x010f, 2, len(make), data0) + e(0x0110, 2, len(model), data0 + len(make)) + e(0x0112, 3, 1, orientation)
    b += e(0x8769, 4, 1, sub) + e(0x8825, 4, 1, 0) + struct.pack("<I", 0) + make + model
    b += struct.pack("<H", 4) + e(0x829a, 5, 1, rats) + e(0x829d, 5, 1, rats + 8) + e(0x8827, 3, 1, 400)
    b += e(0x9003, 2, len(date), rats + 16) + struct.pack("<I", 0)
    b += struct.pack("<IIII", 1, 125, 18, 10) + date
    return b


def picture(path, w, h, left, right, kind="png"):
    pix = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, False, 8, w, h)
    pix.fill(right)
    pix.new_subpixbuf(0, 0, w // 2, h).fill(left)
    pix.savev(path, kind, [], [])


P = os.path.join(HOME, "Pictures")
os.makedirs(P)
# A phone photo: stored 400x200 (red left, blue right), EXIF says "turn right".
picture(os.path.join(P, "photo.jpg"), 400, 200, 0xff0000ff, 0x0000ffff, "jpeg")
raw = open(os.path.join(P, "photo.jpg"), "rb").read()
t = tiff(6)
app1 = b"\xff\xe1" + struct.pack(">H", len(t) + 8) + b"Exif\0\0" + t
open(os.path.join(P, "photo.jpg"), "wb").write(raw[:2] + app1 + raw[2:])
for n in ("img2.png", "img10.png", "screen.png"):
    picture(os.path.join(P, n), 300, 150, 0x22aa55ff, 0xffffffff)
picture(os.path.join(P, "really a png.jpg"), 120, 80, 0xffaa00ff, 0x000000ff)
gif_frame = b"\x21\xf9\x04\x00\x0a\x00\x00\x00" + b"\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00" + b"\x02\x02\x44\x01\x00"
open(os.path.join(P, "moving.gif"), "wb").write(b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff" + gif_frame * 2 + b"\x3b")
open(os.path.join(P, "broken.png"), "wb").write(b"\x89PNG\r\n\x1a\nnot really")
open(os.path.join(P, "empty.jpg"), "wb").close()

# Empty state.
w0 = W.ViewerWindow()
w0.show_all()
pump(0.3)
check("opens with nothing: buttons that need a picture are off", not w0.zoom_in_btn.get_sensitive() and not w0.trash_btn.get_sensitive())
w0.destroy()

win = W.ViewerWindow(os.path.join(P, "photo.jpg"))
win.show_all()
pump(0.5, until=lambda: win.pix is not None)
check("files listed in natural order", [os.path.basename(p) for p in win.files][:4] == ["broken.png", "empty.jpg", "img2.png", "img10.png"],
      str([os.path.basename(p) for p in win.files]))
check("the photo is shown the right way up (EXIF says turned)", (win.pix.get_width(), win.pix.get_height()) == (200, 400),
      f"{win.pix.get_width()}x{win.pix.get_height()}")
win.info_btn.set_active(True)
pump(0.3)
texts = [c.get_text() for c in win.info_box.get_children() if isinstance(c, Gtk.Label)]
check("details: camera, settings, date", "NexaCam Hex 1" in texts and "1/125 s, f/1.8, ISO 400" in texts and "31 October 2026, 18:04" in texts, str(texts))
check("details warn that the location is saved in the photo", any("where it was taken" in t for t in texts))

# Zoom and pan.
win.set_scale(2.0)
pump(0.2)
check("zoom to 200%", win.zoom_label.get_text() == "200%", win.zoom_label.get_text())
cx0 = win.cx
win.drag = (100, 100, win.cx, win.cy)
ev = Gdk.Event.new(Gdk.EventType.MOTION_NOTIFY)
ev.x, ev.y = 60.0, 100.0
win._on_motion(win.canvas, ev)
check("drag pans the picture", win.cx > cx0, f"{cx0} -> {win.cx}")
win.drag = None
s0 = win._view()[0]
win.zoom_by(W.ZOOM_STEPS, (10, 10))
check("the wheel zooms in", win._view()[0] > s0)
win.set_fit()
pump(0.1)

# Lossless turn of the JPEG, and undo.
before = open(os.path.join(P, "photo.jpg"), "rb").read()
win.turn("right")
after = open(os.path.join(P, "photo.jpg"), "rb").read()
changed = sum(1 for a, b in zip(before, after) if a != b)
check("turning a JPEG photo saves it by changing only its orientation", changed == 1 and len(before) == len(after)
      and engine.info(os.path.join(P, "photo.jpg"))["exif"]["orientation"] == 3, f"{changed} bytes")
check("and shows it turned", (win.pix.get_width(), win.pix.get_height()) == (400, 200))
check("says it lost no quality, offers Undo", "without losing" in win.toast_label.get_text() and win.toast_btn.get_visible())
win._toast_action()
check("Undo turns it back", engine.info(os.path.join(P, "photo.jpg"))["exif"]["orientation"] == 6)

# A PNG: turned on screen, saved as a copy.
win.show(win.files.index(os.path.join(P, "img2.png")))
pump(0.5, until=lambda: win.path.endswith("img2.png") and win.pix is not None and win.pix.get_width() == 300)
win.turn("left")
check("a PNG turns on screen only, and says so", (win.pix.get_width(), win.pix.get_height()) == (150, 300)
      and "on screen only" in win.toast_label.get_text())
win.save_turned_copy()
copy = os.path.join(P, "img2 (turned).png")
ok = os.path.exists(copy) and GdkPixbuf.Pixbuf.get_file_info(copy)[1:] == (150, 300)
check("Save a Turned Copy keeps the original, writes a new file", ok and os.path.exists(os.path.join(P, "img2.png")))

# Next and previous, animations, broken files.
win.show(win.files.index(os.path.join(P, "moving.gif")))
pump(0.5, until=lambda: win.path.endswith("moving.gif") and win.pix is not None)
check("an animated GIF plays", win.anim_iter is not None and (win.meta or {}).get("animated"))
win.show(win.files.index(os.path.join(P, "broken.png")))
pump(0.5, until=lambda: win.problem is not None)
check("a broken file: a plain message, no crash", win.problem and "broken.png" in win.problem[0], str(win.problem))
win.show(win.files.index(os.path.join(P, "empty.jpg")))
pump(0.5, until=lambda: win.problem is not None and "empty" in win.problem[0])
check("an empty file: a plain message", win.problem is not None and "empty.jpg" in win.problem[0])
win.show(win.files.index(os.path.join(P, "really a png.jpg")))
pump(0.5, until=lambda: win.pix is not None and win.path.endswith("png.jpg"))
check("a PNG named .jpg opens anyway", win.pix is not None and win.meta and win.meta["format"] == "PNG")
i = win.index
win._on_key(win, type("E", (), {"keyval": Gdk.KEY_Right, "state": 0})())
check("Right goes to the next picture", win.index == (i + 1) % len(win.files))

# Trash and undo.
target = os.path.join(P, "screen.png")
win.show(win.files.index(target))
pump(0.5, until=lambda: win.pix is not None and win.path == target)
win.trash()
pump(0.2)
in_trash = os.path.exists(os.path.join(HOME, ".local/share/Trash/files/screen.png"))
check("Delete moves it to the Trash and shows the next one", not os.path.exists(target) and in_trash and target not in win.files and win.path != target)
win._toast_action()
pump(0.5, until=lambda: win.pix is not None and win.path == target)
check("Undo brings it back", os.path.exists(target) and win.path == target)

# Copy, wallpaper.
win.copy()
pump(0.2)
img = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).wait_for_image()
check("Copy puts the picture on the clipboard", img is not None and img.get_width() == 300)
set_to = []
W.compositor.apply_wallpaper = lambda p: set_to.append(p)
win.set_wallpaper()
check("Set as Wallpaper uses this picture", set_to == [target])

# Strip of thumbnails.
pump(3.0, until=lambda: len(win.thumbs) >= 3)
check("the strip shows thumbnails (nexa-thumb)", len(win.thumbs) >= 3 and len(win.strip_buttons) == len(win.files), f"{len(win.thumbs)}")

# Slideshow.
win.start_slideshow()
pump(0.3)
check("F5 slideshow: full screen, moving on", win.slide_timer and win._presenting())
win._on_key(win, type("E", (), {"keyval": Gdk.KEY_Escape, "state": 0})())
check("Esc leaves the slideshow", not win.slide_timer and not win._presenting())

win.show(win.files.index(os.path.join(P, "photo.jpg")))
pump(1.0, until=lambda: win.pix is not None and win.path.endswith("photo.jpg"))
win.set_scale(0.6)
pump(1.0)
subprocess.run(["grim", os.environ.get("SHOT", "/tmp/viewer.png")], check=False)
shutil.rmtree(HOME, ignore_errors=True)
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
