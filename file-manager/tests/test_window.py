"""The Files window, driven like a person would: mouse clicks, keys, menus,
clipboard, Trash and undo, against the real engine (build/native/nexa-fileops).

Needs a display: run it with scripts/test-files-app.sh (headless Sway in
the nexa-apptest container)."""
import os
import shutil
import sys
import tempfile
import time

HOME = tempfile.mkdtemp(prefix="files-home-")
os.environ["HOME"] = HOME
os.environ.pop("XDG_DATA_HOME", None)
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "file-manager"), os.path.join(REPO, "common")]

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, Gtk  # noqa: E402

from nexa_files import window as W  # noqa: E402

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


def h(*parts):
    return os.path.join(HOME, *parts)


for d in ("Documents", "Music", "Pictures"):
    os.makedirs(h(d))
for f, data in (("notes.txt", "milk"), ("b-report.pdf", "x" * 2048), ("a-song.mp3", "y" * 10)):
    open(h(f), "w").write(data)
open(h(".hidden"), "w").write("")
from gi.repository import GdkPixbuf  # noqa: E402
for name, colour, size in (("beach.png", 0x3399ffff, (1600, 900)), ("forest.png", 0x22aa55ff, (800, 800)), ("dusk.png", 0xff8844ff, (900, 1200))):
    pix = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, False, 8, *size)
    pix.fill(colour)
    pix.savev(h("Pictures", name), "png", [], [])

win = W.FileManagerWindow(HOME)
errors, confirms, answers = [], [], []
win._error = lambda text, secondary=None: errors.append((text, secondary))
win._confirm = lambda text, secondary: confirms.append(text) or True
win._prompt = lambda title, message, default, select_stem=False: answers.pop(0) if answers else default
win.show_all()
pump(0.5)
win._set_view("list")  # the first part drives the list; the grid comes last
pump(0.3)


def names():
    return [r[W.COL_NAME] for r in win.store]


def selected():
    return sorted(i["name"] for i in win._selected())


def row(name):
    return next(r for r in win.store if r[W.COL_NAME] == name)


def wait():
    pump(0.1, until=lambda: not win.busy)


def key(keyval, ctrl=False, shift=False, alt=False):
    ev = Gdk.Event.new(Gdk.EventType.KEY_PRESS)
    ev.keyval = keyval
    ev.state = ((Gdk.ModifierType.CONTROL_MASK if ctrl else 0) | (Gdk.ModifierType.SHIFT_MASK if shift else 0)
                | (Gdk.ModifierType.MOD1_MASK if alt else 0))
    win.view.grab_focus()
    handled = win._on_key(win, ev)
    pump(0.05)
    return handled


CLOCK = [100000]


def mouse(kind, button, x, y, state=0):
    """A real event through GTK's main dispatch (so it is the "current"
    event GTK reads modifiers from): our handler, then the tree's own."""
    ev = Gdk.Event.new(kind)
    ev.button, ev.x, ev.y, ev.state = button, x, y, state
    ev.window = win.tree.get_bin_window()
    ev.set_device(Gdk.Display.get_default().get_default_seat().get_pointer())
    CLOCK[0] += 1000  # far apart: never a double-click by accident
    ev.time = CLOCK[0]
    Gtk.main_do_event(ev)
    pump(0.05)


def spot(name):
    pump(0.15)  # let a freshly loaded folder lay out first
    if name is None:
        return 40, win.tree.get_bin_window().get_height() - 5
    rect = win.tree.get_background_area(row(name).path, win.tree.get_column(0))
    return rect.x + 40, rect.y + rect.height / 2


def add(name):
    """Add to the selection, as Ctrl+click does."""
    win.tree.get_selection().select_iter(row(name).iter)
    pump(0.02)


def click(name=None, button=1, ctrl=False):
    """A mouse click on a row (or on the empty space below the rows)."""
    x, y = spot(name)
    state = Gdk.ModifierType.CONTROL_MASK if ctrl else 0
    mouse(Gdk.EventType.BUTTON_PRESS, button, x, y, state)
    mouse(Gdk.EventType.BUTTON_RELEASE, button, x, y, state)


def menu_labels():
    return [c.get_label() for c in win._menu.get_children() if isinstance(c, Gtk.MenuItem) and c.get_label()]


# Listing and sorting.
check("folders first, then files by name, hidden files hidden",
      names() == ["Documents", "Music", "Pictures", "a-song.mp3", "b-report.pdf", "notes.txt"], str(names()))
win.store.set_sort_column_id(W.COL_BYTES, Gtk.SortType.DESCENDING)
check("sorting by size keeps folders first (equal sizes A to Z)", names() == ["Documents", "Music", "Pictures", "b-report.pdf", "a-song.mp3", "notes.txt"], str(names()))
win.store.set_sort_column_id(W.COL_NAME, Gtk.SortType.ASCENDING)
key(Gdk.KEY_h, ctrl=True)
check("Ctrl+H shows hidden files", ".hidden" in names())
key(Gdk.KEY_h, ctrl=True)

# Selection with the mouse and keys.
key(Gdk.KEY_a, ctrl=True)
check("Ctrl+A selects everything", len(selected()) == 6)
check("the status bar sums up the selection", win.status.get_text().startswith("6 items selected (3 folders,"), win.status.get_text())
click(None)
check("clicking empty space clears the selection", selected() == [])
click("notes.txt")
check("a click selects one item", selected() == ["notes.txt"])
# Ctrl/Shift+click are GTK's own; ours must let them through untouched.
x, y = spot("a-song.mp3")
ev = Gdk.Event.new(Gdk.EventType.BUTTON_PRESS)
ev.button, ev.x, ev.y, ev.state = 1, x, y, Gdk.ModifierType.CONTROL_MASK
check("Ctrl+click is left to GTK (multi-select)", win._on_button_press(win.tree, ev.button) is False and selected() == ["notes.txt"])
click("Music", button=3)
check("right-click on an unselected item selects just it", selected() == ["Music"])
check("...and shows the item menu", {"Open in New Window", "Cut", "Copy", "Rename…", "Move to Trash", "Properties"} <= set(menu_labels()), str(menu_labels()))
win._menu.popdown()
key(Gdk.KEY_a, ctrl=True)
click("notes.txt", button=3)
check("right-click inside a selection keeps it", len(selected()) == 6)
win._menu.popdown()
click(None, button=3)
check("right-click on empty space: nothing selected, the folder menu",
      selected() == [] and {"New Folder…", "Paste", "Select All", "Show Hidden Files", "Open Terminal Here"} <= set(menu_labels()), str(menu_labels()))
win._menu.popdown()
key(Gdk.KEY_a, ctrl=True)
x, y = spot("Music")
mouse(Gdk.EventType.BUTTON_PRESS, 1, x, y)
check("pressing on a selected item keeps the selection (so it can be dragged)", len(selected()) == 6, str(selected()))
mouse(Gdk.EventType.BUTTON_RELEASE, 1, x, y)
check("...and releasing without dragging selects just that item", selected() == ["Music"], str(selected()))
key(Gdk.KEY_Escape)
check("Escape clears the selection", selected() == [])

# Zoom with Ctrl+scroll.
before = win.icon_size_index
sc = Gdk.Event.new(Gdk.EventType.SCROLL)
sc.direction, sc.state = Gdk.ScrollDirection.UP, Gdk.ModifierType.CONTROL_MASK
win._on_scroll(win.tree, sc)
check("Ctrl+scroll zooms", win.icon_size_index == before + 1)
key(Gdk.KEY_0, ctrl=True)

# Copy and paste, then undo.
click("notes.txt")
key(Gdk.KEY_c, ctrl=True)
check("Ctrl+C copies", win.clipboard == {"paths": [h("notes.txt")], "cut": False})
win._navigate(h("Documents"))
pump(0.1)
key(Gdk.KEY_v, ctrl=True)
wait()
check("Ctrl+V pastes a copy (through the engine) and selects it",
      os.path.exists(h("Documents", "notes.txt")) and selected() == ["notes.txt"], str((selected(), errors)))
key(Gdk.KEY_v, ctrl=True)
wait()
check("pasting again never overwrites: 'notes (copy).txt'", os.path.exists(h("Documents", "notes (copy).txt")))
key(Gdk.KEY_z, ctrl=True)
wait()
check("Ctrl+Z undoes the last paste", not os.path.exists(h("Documents", "notes (copy).txt"))
      and os.path.exists(h("Documents", "notes.txt")), win.status.get_text())

# Back, forward, up.
key(Gdk.KEY_Left, alt=True)
check("Alt+Left goes back", win.path == HOME)
key(Gdk.KEY_Right, alt=True)
check("Alt+Right goes forward", win.path == h("Documents"))
key(Gdk.KEY_BackSpace)
check("Backspace goes up and keeps your place", win.path == HOME and selected() == ["Documents"], str(selected()))
mouse(Gdk.EventType.BUTTON_PRESS, 8, 10, 10)
check("the mouse's back button goes back", win.path == h("Documents"), f"{win.path} {win.back_stack}")
win._navigate(HOME)
pump(0.1)

# Cut and paste moves; cut items are dimmed until pasted.
click("a-song.mp3")
key(Gdk.KEY_x, ctrl=True)
check("Ctrl+X dims the cut item", row("a-song.mp3")[W.COL_ACTIVE] is False)
win._navigate(h("Music"))
key(Gdk.KEY_v, ctrl=True)
wait()
check("pasting a cut moves it", os.path.exists(h("Music", "a-song.mp3")) and not os.path.exists(h("a-song.mp3")))
check("a cut is pasted once", win.clipboard is None)

# Rename (F2), new folder (Ctrl+Shift+N), duplicate.
answers.append("song.mp3")
click("a-song.mp3")
key(Gdk.KEY_F2)
check("F2 renames", os.path.exists(h("Music", "song.mp3")) and selected() == ["song.mp3"], str(errors))
answers.append("Albums")
key(Gdk.KEY_n, ctrl=True, shift=True)
check("Ctrl+Shift+N makes a folder and selects it", os.path.isdir(h("Music", "Albums")) and selected() == ["Albums"])
click("song.mp3")
win._on_duplicate(None)
wait()
check("Duplicate makes 'song (copy).mp3'", os.path.exists(h("Music", "song (copy).mp3")))

# The Trash: Delete, the Trash view, restore, undo, empty.
click("song (copy).mp3")
add("Albums")
key(Gdk.KEY_Delete)
wait()
check("Delete moves the selection to the Trash", not os.path.exists(h("Music", "Albums"))
      and os.path.exists(h(".local/share/Trash/info/Albums.trashinfo")), str(errors))
check("...and says how to undo", "Ctrl+Z" in win.status.get_text(), win.status.get_text())
win._on_show_trash(None)
pump(0.1)
check("the Trash view lists them with where they came from",
      sorted(names()) == ["Albums", "notes (copy).txt", "song (copy).mp3"] and row("Albums")[W.COL_TIP] == f"From {h('Music')}", str(names()))
check("...with Restore and Empty Trash", win.actions.get_visible_child_name() == "trash")
click("Albums")
win._on_restore(None)
wait()
check("Restore puts it back", os.path.isdir(h("Music", "Albums")) and sorted(names()) == ["notes (copy).txt", "song (copy).mp3"])
win._on_empty_trash(None)
wait()
check("Empty Trash (after asking)", names() == [] and confirms[-1] == "Empty the Trash?")
win._navigate(h("Music"))
click("song.mp3")
key(Gdk.KEY_Delete, shift=True)
wait()
check("Shift+Delete deletes for good after asking", not os.path.exists(h("Music", "song.mp3"))
      and confirms[-1].startswith("Delete “song.mp3” for good"))

# Properties measures folders with the engine.
win._navigate(HOME)
click("Documents")
shown = {}
real_run = Gtk.Dialog.run


def fake_run(dialog):
    labels = []

    def collect(w):
        if isinstance(w, Gtk.Label):
            labels.append(w)
        elif isinstance(w, Gtk.Container):
            w.foreach(collect)
    collect(dialog)
    pump(0.1, until=lambda: not any(l.get_text() == "Calculating…" for l in labels))
    shown["text"] = [l.get_text() for l in labels]
    return Gtk.ResponseType.CLOSE


Gtk.Dialog.run = fake_run
key(Gdk.KEY_Return, alt=True)
Gtk.Dialog.run = real_run
check("Alt+Enter shows Properties with the folder's measured size",
      "Folder" in shown.get("text", []) and any("1 file" in t for t in shown.get("text", [])), str(shown))

# Opening a folder in a new window (middle click).
click("Pictures", button=2)
check("middle-click opens a folder in a new window", W.FileManagerWindow.open_windows == 2)
for other in [w for w in Gtk.Window.list_toplevels() if isinstance(w, W.FileManagerWindow) and w is not win]:
    other.destroy()
check("closing it leaves this one open", W.FileManagerWindow.open_windows == 1)
win._on_toggle_hidden(None)
check("the View menu shows the hidden-files state", win.hidden_item.get_active() and ".hidden" in names())
win._on_toggle_hidden(None)

# A screenshot with a selection and the menu open.
click("Documents")
add("Pictures")
click("Pictures", button=3)
pump(0.5)
shot = os.environ.get("SHOT")
if shot:
    os.system(f"grim {shot}")
win._menu.popdown()


# ---- The grid: big icons and thumbnails, arranged in rows ----
import json as _json  # noqa: E402
win._navigate(HOME)
pump(0.2)
click("notes.txt")
key(Gdk.KEY_2, ctrl=True)
pump(0.3)
check("Ctrl+2 switches to the grid", win.is_grid and win.views.get_visible_child_name() == "grid")
check("...keeping the selection", selected() == ["notes.txt"], str(selected()))
check("...and is remembered", _json.load(open(W.CONFIG_PATH))["view"] == "grid")
pump(0.3, until=lambda: (h("Pictures"), row("Pictures")[W.COL_TIME], 128) in win._thumb_cache)
folder_icon = row("Pictures")[W.COL_ICON]
check("a folder with pictures shows them (folder thumbnail)", (h("Pictures"), row("Pictures")[W.COL_TIME], 128) in win._thumb_cache
      and folder_icon.get_width() == 128 and folder_icon.get_has_alpha())
win._navigate(h("Pictures"))
pump(0.3, until=lambda: all((h("Pictures", n), row(n)[W.COL_TIME], 128) in win._thumb_cache for n in ("beach.png", "dusk.png")))
beach = row("beach.png")[W.COL_ICON]
dusk = row("dusk.png")[W.COL_ICON]
check("pictures show as thumbnails, shape kept", (beach.get_width(), beach.get_height()) == (128, 72)
      and (dusk.get_width(), dusk.get_height()) == (96, 128), f"{beach.get_width()}x{beach.get_height()} {dusk.get_width()}x{dusk.get_height()}")
gwin = win.grid.get_window().get_children()[0] if win.grid.get_window().get_children() else win.grid.get_bin_window()


def gspot(name):
    pump(0.15)
    ok, rect = win.grid.get_cell_rect(row(name).path, None)
    return rect.x + rect.width / 2, rect.y + rect.height / 3


def gmouse(kind, button, x, y, state=0):
    ev = Gdk.Event.new(kind)
    ev.button, ev.x, ev.y, ev.state = button, x, y, state
    ev.window = gwin
    ev.set_device(Gdk.Display.get_default().get_default_seat().get_pointer())
    CLOCK[0] += 1000
    ev.time = CLOCK[0]
    Gtk.main_do_event(ev)
    pump(0.05)


x, y = gspot("forest.png")
gmouse(Gdk.EventType.BUTTON_PRESS, 3, x, y)
check("right-click in the grid selects the item and shows its menu",
      selected() == ["forest.png"] and "Rename…" in menu_labels(), str((selected(), menu_labels())))
win._menu.popdown()
key(Gdk.KEY_a, ctrl=True)
check("Ctrl+A in the grid", len(selected()) == 3)
key(Gdk.KEY_Escape)
check("Escape clears it", selected() == [])
before = win.grid_size_index
sc = Gdk.Event.new(Gdk.EventType.SCROLL)
sc.direction, sc.state = Gdk.ScrollDirection.UP, Gdk.ModifierType.CONTROL_MASK
win._on_scroll(win.grid, sc)
pump(0.5, until=lambda: (h("Pictures", "beach.png"), row("beach.png")[W.COL_TIME], W.GRID_SIZES[before + 1]) in win._thumb_cache)
check("Ctrl+scroll makes the grid bigger, thumbnails sharp at the new size",
      win.grid_size_index == before + 1 and row("beach.png")[W.COL_ICON].get_width() == W.GRID_SIZES[before + 1])
win._set_sort(W.COL_BYTES, True)
check("View > Sort by size, reversed, in the grid", [r[W.COL_NAME] for r in win.store][0] in ("beach.png", "dusk.png"))
win._set_sort(W.COL_NAME, False)
key(Gdk.KEY_0, ctrl=True)
pump(0.6)
shot = os.environ.get("SHOT")
if shot:
    os.system(f"grim {shot.replace('.png', '-grid.png')}")
win._navigate(HOME)
pump(0.8)
if shot:
    os.system(f"grim {shot.replace('.png', '-grid-home.png')}")
key(Gdk.KEY_1, ctrl=True)
check("Ctrl+1 back to the list", not win.is_grid and win.views.get_visible_child_name() == "list")
# FileTrail: what this test did through the engine is on the trail.
check("the Trail button is there (FileTrail on by default)", win.trail_btn.get_visible())
win.trail_btn.clicked()
pump(1.0, until=lambda: getattr(win, "_trail_win", None) is not None and win._trail_win.items)
tw = getattr(win, "_trail_win", None)
items = tw.items if tw else []
check("Trail lists what Files did in this test", len(items) >= 3, f"{len(items)} {[i['kind'] for i in items[:6]]}")
kinds = {i["kind"] for i in items}
check("...moves or renames, the Trash, new things", kinds & {"moved", "renamed"} and "trashed" in kinds, str(kinds))
pump(0.4)
if shot:
    os.system(f"grim {shot.replace('.png', '-trail.png')}")
here = [i for i in items if i["status"] == "here" and os.path.dirname(i["now"]) != win.path]
if here:
    tw.show_cb(here[0]["now"])
    pump(0.5)
    sel = [i["path"] for i in win._selected()]
    check("Show opens its folder with it selected", win.path == os.path.dirname(here[0]["now"]) and here[0]["now"] in sel,
          f"{win.path} {sel}")
tw.search.set_text("zzz-no-such-name")
pump(0.6, until=lambda: not tw.items)
check("searching by name narrows it", tw.items == [] and "Nothing by that name" in tw.status.get_text())
tw.destroy()

# ---- The sidebar: places, the Trash, pinned folders ----
win._navigate(HOME)
pump(0.3)


def side_rows():
    return [r for r in win.side.get_children() if r.kind != "head"]


labels = [r.get_child().get_children()[1].get_text() for r in side_rows()]
check("the sidebar lists Home, the standard folders and the Trash",
      {"Home", "Documents", "Music", "Pictures", "Trash"} <= set(labels), str(labels))
check("...with Home marked while it is open", win.side.get_selected_row() is next(r for r in side_rows() if r.target == HOME))
docs = next(r for r in side_rows() if r.target == h("Documents"))
win._on_side_row(win.side, docs)
pump(0.3)
check("clicking Documents opens it and marks it", win.path == h("Documents") and win.side.get_selected_row() is docs, win.path)
trash_row = next(r for r in side_rows() if r.kind == "trash")
win._on_side_row(win.side, trash_row)
pump(0.3)
check("clicking Trash opens the Trash", win.in_trash)
win._navigate(HOME)
pump(0.3)
win._pin(h("Pictures"))
pump(0.2)
check("Pin to Sidebar adds the folder under Pinned", any(r.kind == "pin" and r.target == h("Pictures") for r in side_rows()))
check("...and remembers it", h("Pictures") in W.FileManagerWindow._load_prefs(win)["pins"])
win._unpin(h("Pictures"))
pump(0.2)
check("Remove from Sidebar takes it away", not any(r.kind == "pin" for r in side_rows()))
key(Gdk.KEY_F9)
check("F9 hides the sidebar", not win.side_scroll.get_visible())
key(Gdk.KEY_F9)
check("...and F9 again shows it", win.side_scroll.get_visible())

# ---- Search (the engine's "search") ----
os.makedirs(h("Documents", "Taxes", "2025"))
open(h("Documents", "Taxes", "2025", "Tax-Return.pdf"), "w").write("x")
open(h("Documents", "returned-mail.txt"), "w").write("x")
win.search_entry.set_text("return TAX")
pump(0.4, until=lambda: win.search_query and win._search_proc is None and len(win.store) > 0)
check("search finds a file two folders down, words in any order and case", names() == ["Tax-Return.pdf"], str(names()))
check("...and says which folder it is in", "Documents/Taxes/2025" in (win.store[0][W.COL_TIP] or ""), win.store[0][W.COL_TIP])
check("...and the status line says how many", "1 found" in win.status.get_text(), win.status.get_text())
win.search_entry.set_text("return")
pump(0.4, until=lambda: win._search_proc is None and len(win.store) >= 2)
check("typing more or less searches again", sorted(names()) == ["Tax-Return.pdf", "returned-mail.txt"], str(names()))
win.search_entry.grab_focus()
pump(0.1)
ev = Gdk.Event.new(Gdk.EventType.KEY_PRESS)
ev.keyval = Gdk.KEY_Escape
win._on_key(win, ev)
pump(0.3)
check("Escape leaves the search and shows the folder again", win.search_query is None and "Documents" in names(), str(names()))
win.search_entry.set_text("zzz-nothing")
pump(0.4, until=lambda: win._search_proc is None and "Nothing found" in win.status.get_text())
check("no match says so", "Nothing found" in win.status.get_text(), win.status.get_text())
win._navigate(h("Documents"))
pump(0.3)
check("opening another folder ends the search", win.search_query is None and win.search_entry.get_text() == "")

# ---- Compress and Extract (nexa-archiver) ----
os.makedirs(h("Documents", "Project", "src"))
open(h("Documents", "Project", "src", "main.c"), "w").write("int main(void) { return 0; }\n")
open(h("Documents", "Project", "README"), "w").write("hello\n")
win._refresh(select=[h("Documents", "Project")])
pump(0.2)
labels = [e[0] for e in win._item_menu() if e]
check("the item menu offers Compress… (and no Extract for a folder)", "Compress…" in labels
      and not dict((e[0], e[2] if len(e) > 2 else True) for e in win._item_menu() if e)["Extract Here"], str(labels))
win._compress_fmt = "zip"
real_dialog_run = Gtk.Dialog.run
Gtk.Dialog.run = lambda self: Gtk.ResponseType.OK  # accept the suggested name
win._on_compress(None)
Gtk.Dialog.run = real_dialog_run
wait()
check("Compress makes Project.zip next to it and selects it", os.path.isfile(h("Documents", "Project.zip"))
      and selected() == ["Project.zip"], f"{names()} {selected()} {errors}")
menu = dict((e[0], e[2] if len(e) > 2 else True) for e in win._item_menu() if e)
check("an archive gets Extract Here", menu.get("Extract Here") is True)
win._on_extract(None)
wait()
check("Extract Here never replaces: it makes Project (2) with everything inside",
      open(h("Documents", "Project (2)", "src", "main.c")).read().startswith("int main") and selected() == ["Project (2)"],
      f"{names()} {selected()} {errors}")
key(Gdk.KEY_z, ctrl=True)
wait()
check("Ctrl+Z takes the extracted folder away again (to the Trash)", not os.path.exists(h("Documents", "Project (2)")))
check("no error dialogs along the way", not errors, str(errors))
shutil.rmtree(HOME, ignore_errors=True)
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
