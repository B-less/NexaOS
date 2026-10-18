"""nexa-thumb (C) and thumbs.py: real pictures, a PDF, a video, folders, a
broken file, and the Landlock confinement. Needs GTK's libraries: run with
scripts/test-files-app.sh (it runs this too) or any system with them."""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "file-manager"), os.path.join(REPO, "common")]
# Not under /tmp: nexa-thumb may write there (glycin), so a test of what it
# can read must use other folders.
HOME = tempfile.mkdtemp(prefix="thumb-home-", dir="/var/tmp")
os.environ["HOME"] = HOME
os.environ.pop("XDG_CACHE_HOME", None)

import gi  # noqa: E402
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf  # noqa: E402

from nexa_files import thumbs  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


pics = os.path.join(HOME, "Pictures")
os.makedirs(pics)
big = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, False, 8, 1600, 900)
big.fill(0x3366ccff)
big.savev(os.path.join(pics, "holiday.png"), "png", [], [])
for i, colour in enumerate((0xcc3333ff, 0x33cc66ff)):
    p = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, False, 8, 300, 200)
    p.fill(colour)
    p.savev(os.path.join(pics, f"photo{i}.jpg"), "jpeg", [], [])
open(os.path.join(pics, "broken.png"), "wb").write(b"\x89PNG\r\n\x1a\nthis is not really a picture")
docs = os.path.join(HOME, "Documents")
os.makedirs(docs)
# A one-page PDF, written by hand.
objs = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 4 0 R >>"]
stream = b"0 0 1 rg 20 20 260 360 re f"
objs.append(b"<< /Length %d >>stream\n" % len(stream) + stream + b"\nendstream")
pdf, offsets = b"%PDF-1.4\n", []
for i, o in enumerate(objs, 1):
    offsets.append(len(pdf))
    pdf += b"%d 0 obj\n" % i + o + b"\nendobj\n"
xref = len(pdf)
pdf += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1) + b"".join(b"%010d 00000 n \n" % o for o in offsets)
pdf += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, xref)
open(os.path.join(docs, "report.pdf"), "wb").write(pdf)

holiday = os.path.join(pics, "holiday.png")
uri_md5 = hashlib.md5(("file://" + holiday).encode()).hexdigest()
got = thumbs.run([holiday, os.path.join(pics, "broken.png"), os.path.join(docs, "report.pdf"), os.path.join(docs, "missing.txt")], 128)
expect = os.path.join(HOME, ".cache/thumbnails/normal", uri_md5 + ".png")
check("a picture: freedesktop thumbnail named after its URI", got.get(holiday) == expect, str(got))
t = GdkPixbuf.Pixbuf.new_from_file(expect)
check("...at most 128 px, aspect kept", (t.get_width(), t.get_height()) == (128, 72), f"{t.get_width()}x{t.get_height()}")
check("...carrying Thumb::URI and Thumb::MTime", t.get_option("tEXt::Thumb::URI") == "file://" + holiday
      and t.get_option("tEXt::Thumb::MTime") == str(int(os.stat(holiday).st_mtime)))
check("a PDF: its first page", os.path.join(docs, "report.pdf") in got)
check("a broken file: no thumbnail, recorded as failed", os.path.join(pics, "broken.png") not in got
      and len(os.listdir(os.path.join(HOME, ".cache/thumbnails/fail/nexa-thumb"))) == 1)
before = os.stat(expect).st_mtime_ns
time.sleep(0.05)
thumbs.run([holiday], 128)
check("an unchanged file reuses its thumbnail", os.stat(expect).st_mtime_ns == before)
os.utime(holiday, (time.time() + 5, time.time() + 5))
thumbs.run([holiday], 128)
check("a changed file gets a new one", os.stat(expect).st_mtime_ns != before)
check("large (256) thumbnails go in large/", thumbs.run([holiday], 256)[holiday].endswith(f"large/{uri_md5}.png"))

# A video, if this system can make one to test with.
if shutil.which("gst-launch-1.0"):
    video = os.path.join(HOME, "clip.ogv")
    subprocess.run(["gst-launch-1.0", "-q", "videotestsrc", "num-buffers=50", "!", "video/x-raw,width=320,height=240",
                    "!", "theoraenc", "!", "oggmux", "!", "filesink", f"location={video}"], capture_output=True)
    if os.path.exists(video) and os.path.getsize(video) > 0:
        v = thumbs.run([video], 128).get(video)
        check("a video: a frame, with film-strip edges", bool(v) and GdkPixbuf.Pixbuf.new_from_file(v).get_width() == 128, str(v))
else:
    print("note: no gst-launch-1.0 here; video thumbnails not exercised")

# Folders: the pictures inside, or nothing.
f = thumbs.run([pics, docs], 128, "5b8cff", folder=True)
check("a folder with pictures gets a picture-folder thumbnail", pics in f and "/nexa-folders/normal/" in f[pics], str(f))
check("a folder without pictures keeps the normal icon", docs not in f)
fp = GdkPixbuf.Pixbuf.new_from_file(f[pics])
check("...128 px square, transparent around the folder", (fp.get_width(), fp.get_height()) == (128, 128) and fp.get_has_alpha())

# Confinement: a "picture" that is really a link to a file outside the
# folders it may read. Landlock checks the target, so it can't be read.
outside = tempfile.mkdtemp(prefix="secret-", dir="/var/tmp")
p = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, False, 8, 64, 64)
p.fill(0xffffffff)
p.savev(os.path.join(outside, "secret.png"), "png", [], [])
link = os.path.join(pics, "innocent.png")
os.symlink(os.path.join(outside, "secret.png"), link)
check("Landlock: a link out of the folder can't be read", link not in thumbs.run([link], 128))
check("...while the real file in its own folder can", os.path.join(outside, "secret.png") in thumbs.run([os.path.join(outside, "secret.png")], 128))

shutil.rmtree(HOME, ignore_errors=True)
shutil.rmtree(outside, ignore_errors=True)
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
