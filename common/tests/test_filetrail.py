"""FileTrail through Files' engine and the real nexa-fileops: moves,
renames, copies and the Trash are written down, followed to where things
are now (through a later folder move), put back, forgotten; the switch in
Settings > Magic Layer stops it."""
import os
import sys
import tempfile
import time

here = os.path.dirname(__file__)
sys.path.insert(0, os.path.join(here, ".."))
sys.path.insert(0, os.path.join(here, "..", "..", "file-manager"))
from nexa_common import magic  # noqa: E402
from nexa_files import engine  # noqa: E402
from nexa_files.trail import describe, place  # noqa: E402

failures = 0


def check(what, ok, detail=""):
    global failures
    print(("PASS " if ok else "FAIL ") + what + (f"  [{detail}]" if not ok and detail else ""))
    failures += not ok


home = os.path.realpath(tempfile.mkdtemp())
os.environ["HOME"] = home
for k in ("XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"):
    os.environ.pop(k, None)
os.environ["XDG_RUNTIME_DIR"] = home
H = lambda *p: os.path.join(home, *p)  # noqa: E731
for d in ("Downloads", "Documents", "Pictures", "USB"):
    os.makedirs(H(d))
SECRET = "inside-the-file-7f3a9"
for f in ("Downloads/report.pdf", "Downloads/old.txt", "Pictures/photo.jpg", "notes.txt"):
    with open(H(f), "w") as fh:
        fh.write(SECRET)

engine.move([H("Downloads/report.pdf")], H("Documents"))
engine.rename(H("Downloads/old.txt"), "new.txt")
engine.copy([H("Pictures/photo.jpg")], H("USB"))
engine.trash([H("notes.txt")])
os.makedirs(H("Work"))
engine.move([H("Documents")], H("Work"))  # the folder, report.pdf inside

t = engine.trail("report")
check("a move is written down", len(t) == 1 and t[0]["kind"] == "moved", t)
check("...and followed through the later folder move", t and t[0]["now"] == H("Work/Documents/report.pdf"), t and t[0]["now"])
what, now = describe(t[0], home)
check("said in words", what.startswith("Moved from Downloads to Documents") and now == "Now in Work/Documents", (what, now))
all_items = {i["name"]: i for i in engine.trail()}
check("a rename", all_items.get("new.txt", {}).get("kind") == "renamed")
check("a copy, with where it came from", all_items.get("photo.jpg", {}).get("from") == H("Pictures/photo.jpg"))
check("the Trash, and that it's still there", all_items.get("notes.txt", {}).get("status") == "in-trash")

# Put it back: report.pdf goes from Work/Documents back to Downloads.
engine.undo([t[0]["put_back"]])
check("Put it back moves it home", os.path.exists(H("Downloads/report.pdf")) and not os.path.exists(H("Work/Documents/report.pdf")))
t = engine.trail("report")
check("...and that is on the trail too", t[0]["kind"] == "put back" and t[0]["now"] == H("Downloads/report.pdf"), t[0])
engine.undo([all_items["notes.txt"]["put_back"]])
check("a trashed file comes back from the Trash", os.path.exists(H("notes.txt")))

# Moved by something other than Files: said honestly.
os.rename(H("USB/photo.jpg"), H("photo-elsewhere.jpg"))
p = [i for i in engine.trail("photo") if i["kind"] == "copied"][0]
check("moved behind Files' back: 'not there any more', not a guess", p["status"] == "gone" and "not there" in describe(p, home)[1].lower())

# Private and bounded (nexa_sys::filetrail tests the bound).
st = os.stat(H(".local/state/nexa/filetrail.jsonl"))
check("only you can read it", st.st_mode & 0o077 == 0)
with open(H(".local/state/nexa/filetrail.jsonl")) as fh:
    text = fh.read()
check("names and places only, never contents", SECRET not in text and "report.pdf" in text)

# The switch.
magic.set_feature("filetrail", home, enabled=False)
before = len(engine.trail())
engine.rename(H("Downloads/new.txt"), "newer.txt")
check("switched off: nothing more is written", len(engine.trail()) == before)
magic.set_feature("filetrail", home, enabled=True)

engine.trail_forget()
check("Forget all of it", engine.trail() == [])
check("place() speaks like a person", place(H("a.txt"), home) == "your home folder" and place("/media/STICK/a", home) == "/media/STICK")
start = time.time()
for _ in range(5):
    engine.trail("x")
print(f"MEASURE trail question: {(time.time() - start) / 5 * 1000:.1f} ms each (start nexa-fileops, read, follow)")
sys.exit(1 if failures else 0)
