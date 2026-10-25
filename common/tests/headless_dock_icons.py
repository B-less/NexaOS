"""The Dock's icons follow the App icons style (Colour -> Tinted) live: it
draws its own icons from pictures it keeps, so they must be loaded again.
Run by scripts/test-theme-live.sh (NexaHexa sets installed there)."""
import os, subprocess, sys, time
sys.path[:0] = ["/src/shell", "/src/common"]
import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk
from nexa_shell import dock

def pump(s):
    end = time.time() + s
    while time.time() < end:
        while Gtk.events_pending(): Gtk.main_iteration_do(False)
        time.sleep(0.02)

app = Gtk.Application(application_id="org.nexaos.test.dockicons")
app.register(None)
d = dock.Dock(app)
d.show_all()
pump(1.5)
files = next((it for it in d.items if it["id"] == "nexa-files"), None)
if files is None or files["pixbuf"] is None:
    print("FAIL the Dock has a Files icon"); sys.exit(1)
def centre(pb):
    px, rs, nc = pb.get_pixels(), pb.get_rowstride(), pb.get_n_channels()
    x, y = pb.get_width() // 2, pb.get_height() // 4  # the tile's colour, above the glyph
    return tuple(px[y * rs + x * nc: y * rs + x * nc + 3])
before = centre(files["pixbuf"])
subprocess.run([sys.executable, "-c", "import sys; sys.path.insert(0, '/src/common'); from nexa_common import themes; themes.apply(icon_style='tinted', reload=False)"], check=True)
pump(2.5)
after = centre(files["pixbuf"])
ok = before != after and sum(after) < sum(before)
print(("PASS" if ok else "FAIL") + f" the Dock's icons switch to Tinted live: {before} -> {after}")
sys.exit(0 if ok else 1)
