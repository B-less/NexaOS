"""The Video Player (C) follows a theme change live. Run by scripts/test-theme-live.sh."""
import json, os, subprocess, sys, time
import gi; gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf
vp = subprocess.Popen(["/src/apps/video-player/build/video-player"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
def rect():
    for _ in range(100):
        stack = [json.loads(subprocess.run(["swaymsg", "-t", "get_tree"], capture_output=True, text=True).stdout)]
        while stack:
            n = stack.pop()
            if n.get("pid") == vp.pid: return n["rect"]
            stack.extend(n.get("nodes", []) + n.get("floating_nodes", []))
        time.sleep(0.2)
def shot(name):
    r = rect(); path = os.path.join(os.environ.get("OUT", "/tmp"), name + ".png")
    subprocess.run(["grim", "-g", f"{r['x']},{r['y']} {r['width']}x{r['height']}", path])
    pb = GdkPixbuf.Pixbuf.new_from_file(path); px = pb.get_pixels(); rs = pb.get_rowstride(); nc = pb.get_n_channels()
    at = lambda x, y: tuple(px[y * rs + x * nc: y * rs + x * nc + 3])
    return {"titlebar": at(20, 18), "body": at(pb.get_width() // 2, pb.get_height() - 30)}
time.sleep(2); before = shot("vp-before")
subprocess.run([sys.executable, "-c", "import sys; sys.path.insert(0, '/src/common'); from nexa_common import themes; themes.apply('daybreak', reload=False)"], check=True)
time.sleep(2.5); after = shot("vp-after")
print("before", before); print("after ", after); ok = before != after; print("PASS Video Player follows the theme" if ok else "FAIL Video Player kept the old theme")
vp.kill(); sys.exit(0 if ok else 1)
