"""The PDF Viewer window, driven like a person would, against the real
engine (build/native/nexa-pdfdoc): pages in one scroll, zoom and fit,
contents and thumbnails, search, go to page, links, selecting and copying
text, turning, dark mode, presentation, a locked PDF, broken files, and
each document opening where it was left.

Needs a display: run it with scripts/test-pdf-app.sh."""
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time

HOME = tempfile.mkdtemp(prefix="pdf-home-")
os.environ["HOME"] = HOME
for v in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"):
    os.environ.pop(v, None)
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "pdf-viewer"), os.path.join(REPO, "common")]

import cairo  # noqa: E402
import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, Gtk  # noqa: E402

from nexa_pdf import window as W  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def pump(seconds=0.2, until=None):
    end = time.time() + seconds
    while time.time() < end or (until and not until() and time.time() < end + 15):
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        if until and until() and time.time() >= end:
            return
        time.sleep(0.01)


def make_pdf(path, pages=5):
    s = cairo.PDFSurface(path, 595, 842)
    s.set_metadata(cairo.PDF_METADATA_TITLE, "The Nexa Handbook")
    s.set_metadata(cairo.PDF_METADATA_AUTHOR, "Nexa Writers")
    cr = cairo.Context(s)
    cr.select_font_face("Sans")
    cr.set_font_size(24)
    top = s.add_outline(cairo.PDF_OUTLINE_ROOT, "Part One", "page=1", 0)
    for i in range(pages):
        cr.set_source_rgb(0, 0, 0)
        cr.move_to(72, 100)
        cr.show_text(f"Chapter {i + 1}: milk and eggs")
        cr.move_to(72, 400)
        cr.show_text("Milk again")
        if i < 3:
            s.add_outline(top, f"Chapter {i + 1}", f"page={i + 1}", 0)
        if i == 0:
            cr.tag_begin(cairo.TAG_LINK, "uri='https://nexaos.example/'")
            cr.move_to(72, 600)
            cr.show_text("A web link")
            cr.tag_end(cairo.TAG_LINK)
            cr.tag_begin(cairo.TAG_LINK, "page=3")
            cr.move_to(72, 700)
            cr.show_text("See chapter 3")
            cr.tag_end(cairo.TAG_LINK)
        s.show_page()
    s.finish()


PAD = bytes.fromhex("28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A")


def rc4(key, data):
    S, j, out = list(range(256)), 0, bytearray()
    for i in range(256):
        j = (j + S[i] + key[i % len(key)]) % 256
        S[i], S[j] = S[j], S[i]
    i = j = 0
    for c in data:
        i = (i + 1) % 256
        j = (j + S[i]) % 256
        S[i], S[j] = S[j], S[i]
        out.append(c ^ S[(S[i] + S[j]) % 256])
    return bytes(out)


def locked_pdf(path, password):
    """A one-page PDF locked with the standard security handler (RC4, 40-bit)."""
    pad = lambda p: (p.encode() + PAD)[:32]  # noqa: E731
    owner_key = hashlib.md5(pad("owner")).digest()[:5]
    O = rc4(owner_key, pad(password))
    P, doc_id = -4, b"0123456789abcdef"
    key = hashlib.md5(pad(password) + O + struct.pack("<i", P) + doc_id).digest()[:5]
    U = rc4(key, PAD)
    stream = b"BT /F1 24 Tf 72 700 Td (Secret milk) Tj ET"
    enc = rc4(hashlib.md5(key + struct.pack("<I", 4)[:3] + b"\0\0").digest()[:10], stream)
    objs = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
            b"<< /Length %d >>\nstream\n" % len(enc) + enc + b"\nendstream",
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            b"<< /Filter /Standard /V 1 /R 2 /O <" + O.hex().encode() + b"> /U <" + U.hex().encode() + b"> /P -4 >>"]
    out, offsets = bytearray(b"%PDF-1.4\n"), []
    for n, body in enumerate(objs, 1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % n + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1) + b"".join(b"%010d 00000 n \n" % o for o in offsets)
    out += b"trailer\n<< /Size %d /Root 1 0 R /Encrypt 6 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objs) + 1, doc_id.hex().encode(), doc_id.hex().encode(), xref)
    open(path, "wb").write(bytes(out))


D = os.path.join(HOME, "Documents")
os.makedirs(D)
book = os.path.join(D, "hand book.pdf")
make_pdf(book)
locked_pdf(os.path.join(D, "locked.pdf"), "nexa123")
open(os.path.join(D, "broken.pdf"), "wb").write(b"%PDF-1.4\nnot really a pdf")
open(os.path.join(D, "empty.pdf"), "wb").close()


def patch(win):
    win.errors = []
    win.error = lambda text, secondary=None: win.errors.append((text, secondary))


w0 = W.PdfWindow()
patch(w0)
w0.show_all()
pump(0.3)
check("opens with nothing: page and zoom controls are off", not w0.zoom_in_btn.get_sensitive() and not w0.find_btn.get_sensitive())
w0.destroy()

win = W.PdfWindow(book)
patch(win)
win.resize(1100, 800)
win.show_all()
pump(1.0, until=lambda: win.doc is not None and win.rects)
check("all 5 pages in one scroll", len(win.rects) == 5 and win.page_label.get_text() == "of 5", win.page_label.get_text())
check("the title comes from the document", win.get_title().startswith("The Nexa Handbook"), win.get_title())
check("fit width fills the view", win.mode == "width" and abs(win.rects[0][2] - (win.scroller.get_allocated_width() - 2 * W.MARGIN - 16)) < 3,
      f"{win.rects[0][2]} vs {win.scroller.get_allocated_width()}")
toc = [(r[0], r[1]) for r in win.toc_store]
check("contents (outline) listed", toc == [("Part One", 1)] and len(list(win.toc_store[0].iterchildren())) == 3, str(toc))
pump(4.0, until=lambda: all(getattr(r, "done", False) for r in win.thumb_rows))
check("page thumbnails made by the engine", all(getattr(r, "done", False) for r in win.thumb_rows))

# Moving around.
win.go_to(3)
pump(0.3)
check("go to page 4", win.current == 3 and win.page_entry.get_text() == "4")
win.page_entry.set_text("2")
win._on_page_entry()
pump(0.3)
check("typing a page number goes there", win.current == 1)
win.zoom_by(W.ZOOM_STEP)
pump(0.3)
check("zoom in", win.mode == "custom" and win.zoom_label.get_text().endswith("%"))
check("zooming keeps the same page in view", win.current == 1, str(win.current))
win.set_mode("page")
pump(0.3)
check("fit page shows a whole page", win.rects[0][3] <= win.scroller.get_allocated_height())
win.set_mode("width")
pump(0.3)

# Contents and links.
win.go_to(0)
pump(0.2)
links = win._links_of(0)
uris = [a.uri.uri for _r, a in links if a.type == W.Poppler.ActionType.URI]
check("web links found", uris == ["https://nexaos.example/"], str(uris))
inner = [a for _r, a in links if a.type == W.Poppler.ActionType.GOTO_DEST]
win._follow(inner[0])
pump(0.3)
check("a link inside the document goes to its page", win.current == 2, str(win.current))

# Search.
win.show_find()
win.find_entry.set_text("milk")
win.search()
pump(1.0, until=lambda: win.find_count.get_text().endswith("of 10"))
check("search finds every match in the document", len(win.matches) == 10 and win.find_count.get_text().endswith("of 10"), win.find_count.get_text())
check("it starts at the page being read", win.matches[win.match_index][0] == 2, str(win.match_index))
i0 = win.match_index
win.find_next(1)
check("next match", win.match_index == (i0 + 1) % 10)
win.case_btn.set_active(True)
win.find_entry.set_text("Milk")
win.search()
pump(1.0, until=lambda: "of" in win.find_count.get_text() or win.find_count.get_text() == "Not found")
check("upper and lower case", len(win.matches) == 5, str(len(win.matches)))
win.case_btn.set_active(False)
win.find_entry.set_text("zebra")
win.search()
pump(1.0, until=lambda: win.find_count.get_text() == "Not found")
check("nothing found is said", win.find_count.get_text() == "Not found")
win.find_entry.set_text("milk")
win.search()
pump(1.0, until=lambda: win.find_count.get_text().endswith("of 10"))

# Selecting text.
win._select(0, (60, 70), (480, 110))
check("selecting text on a page", "Chapter 1: milk and eggs" in win.sel_text, repr(win.sel_text))
win.copy()
pump(0.2)
check("Ctrl+C copies it", Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).wait_for_text() == win.sel_text)

# Turning: page and screen positions still match.
win.turn()
pump(0.3)
ok = all(max(abs(a - b) for a, b in zip(win.to_page(1, *win.to_screen(1, x, y)), (x, y))) < 0.01 for x, y in ((10, 20), (500, 700), (300, 11)))
check("turned: pages lie on their side, positions still right", win.rotation == 90 and win.rects[0][2] > win.rects[0][3] and ok)
win.turn(); win.turn(); win.turn()
pump(0.2)

# Dark mode.
win.set_dark(True)
surf = win._render(0)
data = surf.get_data()
check("dark reading mode: the white page turns dark", data[0] < 40 and data[1] < 40, str(bytes(data[:4])))
win.set_dark(False)

# Presentation.
win.start_presentation()
pump(0.4)
check("F5 presents one page full screen", win.present and not win.toolbar.get_visible())
p0 = win.present_page
win._on_key(win, type("E", (), {"keyval": Gdk.KEY_space, "state": 0})())
check("space goes to the next slide", win.present_page == p0 + 1)
win._on_key(win, type("E", (), {"keyval": Gdk.KEY_Escape, "state": 0})())
pump(0.4)
check("Esc ends the presentation", not win.present and win.toolbar.get_visible())

# It remembers where you were.
win.go_to(3)
pump(0.3)
win._remember()
win.destroy()
win = W.PdfWindow(book)
patch(win)
win.show_all()
pump(1.0, until=lambda: win.doc is not None and win.current == 3)
check("reopened where it was left", win.current == 3, str(win.current))

# A locked PDF.
asked = []
answers = ["wrong", "nexa123"]
win.ask_password = lambda path, wrong: (asked.append(wrong), answers.pop(0))[1]
check("a locked PDF asks for its password", win.open_path(os.path.join(D, "locked.pdf")) and asked == [False, True], str(asked))
win.find_entry.set_text("secret")
win.search()
pump(1.0, until=lambda: win.find_count.get_text().endswith("of 1"))
check("and once open, search works on it too", len(win.matches) == 1, win.find_count.get_text())

# Broken files.
for name, want in (("broken.pdf", "damaged"), ("empty.pdf", "damaged"), ("missing.pdf", "isn't there")):
    win.errors = []
    win.open_path(os.path.join(D, name))
    check(f"{name}: a plain message, the open document stays", win.errors and want in (win.errors[-1][1] or "") and win.path.endswith("locked.pdf"),
          str(win.errors))

win.open_path(book)
win.side_btn.set_active(True)
win.show_find()
win.find_entry.set_text("milk")
win.search()
pump(1.5, until=lambda: win.find_count.get_text().endswith("of 10"))
pump(1.0)
subprocess.run(["grim", os.environ.get("SHOT", "/tmp/pdf.png")], check=False)
shutil.rmtree(HOME, ignore_errors=True)
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
