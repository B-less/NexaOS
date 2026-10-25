"""An open Nexa app (Archive) follows a theme change live: dark Horizon to
light Daybreak, title bar and body. Run by scripts/test-theme-live.sh."""
import json, os, subprocess, sys, time
sys.path[:0] = ["/src/archive", "/src/common"]
import gi
gi.require_version("Gtk", "3.0"); gi.require_version("GdkPixbuf", "2.0")
from gi.repository import Gtk, GdkPixbuf
from nexa_archive.window import ArchiveWindow

def pump(s):
    end = time.time() + s
    while time.time() < end:
        while Gtk.events_pending(): Gtk.main_iteration_do(False)
        time.sleep(0.02)

def rect():
    stack = [json.loads(subprocess.run(["swaymsg", "-t", "get_tree"], capture_output=True, text=True).stdout)]
    while stack:
        n = stack.pop()
        if n.get("pid") == os.getpid(): return n["rect"]
        stack.extend(n.get("nodes", []) + n.get("floating_nodes", []))

def shot(name):
    r = rect(); path = os.path.join(os.environ.get("OUT", "/tmp"), name + ".png")
    subprocess.run(["grim", "-g", f"{r['x']},{r['y']} {r['width']}x{r['height']}", path])
    pb = GdkPixbuf.Pixbuf.new_from_file(path)
    px = pb.get_pixels(); rs = pb.get_rowstride(); nc = pb.get_n_channels()
    def at(x, y): o = y * rs + x * nc; return tuple(px[o:o+3])
    return {"titlebar": at(20, 20), "body": at(pb.get_width() // 2, pb.get_height() // 2)}

win = ArchiveWindow(None); win.connect("destroy", Gtk.main_quit); win.show_all(); pump(1.5)
before = shot("theme-before")
subprocess.run([sys.executable, "-c", "import sys; sys.path.insert(0, '/src/common'); from nexa_common import themes; themes.apply('daybreak', reload=False)"], check=True)
pump(2.5)
after = shot("theme-after")
print("before", before); print("after ", after)
print("PASS open app follows the theme" if before != after else "FAIL open app kept the old theme"); sys.exit(0 if before != after else 1)
