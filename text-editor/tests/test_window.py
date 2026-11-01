"""The Editor window, driven like a person would, against the real engine
(build/native/nexa-textio): encodings and line endings kept, safe saves,
other programs' changes, find and replace, go to line, tabs, drafts after a
crash, the open tabs coming back, hostile files.

Needs a display: run it with scripts/test-editor-app.sh (headless Sway in
the nexa-apptest container)."""
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import time

HOME = tempfile.mkdtemp(prefix="edit-home-")
os.environ["HOME"] = HOME
for v in ("XDG_DATA_HOME", "XDG_CONFIG_HOME"):
    os.environ.pop(v, None)
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "text-editor"), os.path.join(REPO, "common")]

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, Gtk  # noqa: E402

from nexa_edit import engine, window as W  # noqa: E402

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


def h(*parts):
    return os.path.join(HOME, *parts)


def key(win, name, ctrl=False, shift=False):
    ev = Gdk.Event.new(Gdk.EventType.KEY_PRESS)
    ev.window = win.get_window()
    ev.keyval = Gdk.keyval_from_name(name)
    ev.state = (Gdk.ModifierType.CONTROL_MASK if ctrl else 0) | (Gdk.ModifierType.SHIFT_MASK if shift else 0)
    keymap = Gdk.Keymap.get_for_display(Gdk.Display.get_default())
    ok, keys = keymap.get_entries_for_keyval(ev.keyval)
    if ok and keys:
        ev.hardware_keycode = keys[0].keycode
    ev.set_device(Gdk.Display.get_default().get_default_seat().get_keyboard())
    Gtk.main_do_event(ev)
    pump(0.1)


def patch(win, answers):
    win.errors = []
    win.error = lambda text, secondary=None: win.errors.append((text, secondary))
    win.asked = []
    win.choose = lambda text, secondary, buttons: (win.asked.append(text), answers.pop(0) if answers else "cancel")[1]


# Files as people have them.
os.makedirs(h("Documents"))
open(h("Documents", "old.txt"), "wb").write(b"caf\xe9 line one\r\nsecond line\r\nthird line\r\n")
os.chmod(h("Documents", "old.txt"), 0o640)
open(h("Documents", "notes.md"), "w").write("# Notes\n\nmilk\neggs\nmilk again\n")
open(h("Documents", "my file 'quoted'.txt"), "w").write("spaces and quotes\n")
open(h("Documents", "empty.txt"), "w").close()
shutil.copy("/bin/busybox" if os.path.exists("/bin/busybox") else sys.executable, h("Documents", "program.txt"))
with open(h("Documents", "huge.txt"), "wb") as f:
    f.truncate(65 << 20)
    f.seek(0)
    f.write(b"x")

answers = []
win = W.EditorWindow([h("Documents", "old.txt")])
patch(win, answers)
win.show_all()
pump(0.5)
doc = win.doc()
check("opens an old Windows file: Windows-1252, CRLF", (doc.encoding, doc.line_ending) == ("windows-1252", "crlf")
      and win.enc_btn.get_label() == "Windows-1252" and win.eol_btn.get_label() == "CRLF", f"{doc.encoding} {doc.line_ending}")
check("its text reads right", doc.text().startswith("café line one\nsecond"), repr(doc.text()[:20]))
check("one tab: no tab bar", not win.notebook.get_show_tabs())

# Edit and save: same encoding, line endings and permissions.
doc.buffer.insert(doc.buffer.get_end_iter(), "fourth line, €5\n")
pump(0.1)
check("unsaved changes show in the tab and title", doc.tab_label.get_text().startswith("●") and win.get_title().startswith("●"))
key(win, "s", ctrl=True)
raw = open(h("Documents", "old.txt"), "rb").read()
check("Ctrl+S saves in Windows-1252 with CRLF", raw == b"caf\xe9 line one\r\nsecond line\r\nthird line\r\nfourth line, \x805\r\n", repr(raw))
check("the file keeps its permissions", stat.S_IMODE(os.stat(h("Documents", "old.txt")).st_mode) == 0o640)
check("no temporary files left", sorted(os.listdir(h("Documents"))) == sorted(["old.txt", "notes.md", "my file 'quoted'.txt", "empty.txt", "program.txt", "huge.txt"]))
check("saved: no dot", not doc.modified and not doc.tab_label.get_text().startswith("●"))

# A character Windows-1252 can't hold: refused, says what to do.
doc.buffer.insert(doc.buffer.get_end_iter(), "snow ☃\n")
win.save(doc)
check("a character the encoding can't hold: not saved, told to use UTF-8", win.errors and "UTF-8" in (win.errors[-1][1] or ""), str(win.errors))
win._set_encoding("utf-8")
check("encoding switched in the status bar", win.enc_btn.get_label() == "UTF-8")
win.save(doc)
check("saved as UTF-8 after switching", open(h("Documents", "old.txt"), "rb").read().endswith("snow ☃\r\n".encode()))

# Tabs.
win.open_path(h("Documents", "notes.md"))
pump(0.2)
check("a second file opens in a new tab", win.notebook.get_n_pages() == 2 and win.notebook.get_show_tabs())
check("Markdown recognised", (win.doc().buffer.get_language() or None) is not None and win.lang_btn.get_label() == "Markdown", win.lang_btn.get_label())
win.open_path(h("Documents", "old.txt"))
check("opening an open file goes to its tab", win.notebook.get_n_pages() == 2 and win.doc().path.endswith("old.txt"))
key(win, "Tab", ctrl=True)
check("Ctrl+Tab goes to the next tab", win.doc().path.endswith("notes.md"))

# Find and replace.
key(win, "f", ctrl=True)
win.find_entry.set_text("milk")
pump(0.4, until=lambda: win.find_count.get_text() not in ("", "…"))
check("Find counts matches", win.find_count.get_text() == "2 found", win.find_count.get_text())
win.find_step(True)
pump(0.2)
check("Next selects a match", win.find_count.get_text() == "1 of 2", win.find_count.get_text())
win.regex_btn.set_active(True)
win.find_entry.set_text("e+g{2}s")
pump(0.4, until=lambda: win.find_count.get_text() not in ("", "…"))
check("regular expressions", win.find_count.get_text() == "1 found", win.find_count.get_text())
win.find_entry.set_text("(")
pump(0.3)
check("a broken pattern is said plainly", win.find_count.get_text() == "That pattern isn't valid", win.find_count.get_text())
win.regex_btn.set_active(False)
win.word_btn.set_active(True)
win.find_entry.set_text("milk")
key(win, "h", ctrl=True)
pump(0.3)
check("Ctrl+H shows the Replace row", win.replace_entry.get_mapped())
win.replace_entry.set_text("bread")
n = win.replace_all()
check("Replace All replaces every whole word and counts", n == 2 and win.doc().text().count("bread") == 2, str(n))
win.doc().buffer.undo()
check("one undo puts them all back", win.doc().text().count("milk") == 2)
key(win, "Escape")
check("Esc closes the find bar", not win.find_revealer.get_reveal_child())

# Go to line.
key(win, "l", ctrl=True)
win.goto_entry.set_text("4")
win.goto_line()
it = win.doc().buffer.get_iter_at_mark(win.doc().buffer.get_insert())
check("Go to line 4", it.get_line() == 3 and win.status_pos.get_text().startswith("Ln 4"), win.status_pos.get_text())

# Another program changes a file.
notes = win.doc()
time.sleep(0.05)
open(h("Documents", "notes.md"), "w").write("changed elsewhere\n")
pump(1.0, until=lambda: notes.text() == "changed elsewhere\n")
check("unchanged here, changed there: reloaded", notes.text() == "changed elsewhere\n", repr(notes.text()))
notes.buffer.insert(notes.buffer.get_end_iter(), "mine\n")
time.sleep(0.05)
open(h("Documents", "notes.md"), "w").write("theirs again\n")
pump(1.0, until=lambda: notes.infobar.get_visible())
check("changed in both: asked, not lost", notes.infobar.get_visible() and "Another program changed" in notes.info_label.get_text())
notes.stamp = ("1", 1)  # as if the notice was dismissed and the file kept changing
answers[:] = ["force"]
check("saving over their version asks first", win.save(notes) and win.asked and "changed" in win.asked[-1])
check("then saves yours", open(h("Documents", "notes.md")).read() == "changed elsewhere\nmine\n")

# Hostile files.
for name, want in (("program.txt", "isn't a text file"), ("huge.txt", "too big")):
    win.errors = []
    win.open_path(h("Documents", name))
    check(f"{name}: refused with a plain message", win.errors and want in (win.errors[-1][1] or ""), str(win.errors))
for name in ("my file 'quoted'.txt", "empty.txt"):
    check(f"opens {name}", win.open_path(h("Documents", name)))

# A new file, Save As.
win.new_doc()
d = win.doc()
d.buffer.set_text("brand new\n")
check("new tab is Untitled", d.title().startswith("Untitled"))
win.save(d, h("Documents", "new.txt"), force=True)
check("Save As writes it and names the tab", open(h("Documents", "new.txt")).read() == "brand new\n" and d.title() == "new.txt")
check("recent files remembered", win.settings["recent"][0] == h("Documents", "new.txt"))

# View options and zoom.
win._set_option("wrap", True)
win.zoom(2)
check("wrap and zoom apply", d.view.get_wrap_mode() != Gtk.WrapMode.NONE and win.settings["size"] == 13)
win.zoom(0)

# Drafts: unsaved changes are kept while typing.
d.buffer.insert(d.buffer.get_end_iter(), "typed, not saved\n")
drafts = h(".local/share/nexa-edit/drafts")
pump(2.5, until=lambda: os.path.isdir(drafts) and os.listdir(drafts))
check("a draft of unsaved changes is kept", os.path.isdir(drafts) and len(os.listdir(drafts)) == 1)
check("drafts are private (0700)", stat.S_IMODE(os.stat(drafts).st_mode) == 0o700)

# Closing: asked, then the session is remembered.
answers[:] = ["discard"]
win.close()
pump(0.3)
check("closing with unsaved changes asks", win.asked and "unsaved" in win.asked[-1] or "Save the changes" in win.asked[-1], str(win.asked[-1:]))
check("Don't Save throws the drafts away", not os.listdir(drafts))
check("the open files are remembered", h("Documents", "new.txt") in W.load_settings()["session"])

# A crash: a draft left by an Editor that is gone (pid 999999).
engine.run("draft_put", id="999999-crashed", path=h("Documents", "notes.md"), title="notes.md", text="lost work\n",
           encoding="utf-8", line_ending="lf")
win2 = W.EditorWindow([])
patch(win2, [])
win2.show_all()
pump(0.5)
rec = [x for x in win2.docs() if x.text() == "lost work\n"]
check("after a crash the unsaved changes come back", len(rec) == 1 and rec[0].modified and rec[0].infobar.get_visible())
check("the files open before come back too", any(x.path == h("Documents", "new.txt") for x in win2.docs()))
pump(2.0)
check("the recovered draft now belongs to this Editor", all(not f.startswith("999999") for f in os.listdir(drafts)), str(os.listdir(drafts)))
win2.notebook.set_current_page(win2.notebook.page_num(rec[0].page))
win2.show_find(True)
win2.find_entry.set_text("lost")
pump(0.5)
subprocess.run(["grim", os.environ.get("SHOT", "/tmp/editor.png")], check=False)
check("no error dialogs in the second window", not win2.errors, str(win2.errors))
shutil.rmtree(HOME, ignore_errors=True)
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
