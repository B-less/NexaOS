"""Several selected files dragged onto a folder with a real mouse (the
virtual pointer, common/tests/vpointer.c, through the compositor): all of
them must move, in the grid and in the list. Pressing on one of the
selected files used to narrow the selection to that one, so only it moved.
Run with scripts/test-files-drag.sh (headless Sway, nexa-apptest)."""
import json
import os
import subprocess
import sys
import tempfile
import time

HOME = tempfile.mkdtemp(prefix="files-drag-")
os.environ["HOME"] = HOME
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "file-manager"), os.path.join(REPO, "common")]

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

from nexa_files import window as W  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def pump(seconds=0.2):
    end = time.time() + seconds
    while time.time() < end:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        time.sleep(0.01)


_vp = subprocess.Popen([os.environ.get("VPOINTER", "/tmp/vpointer")], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
_vp.stdout.readline()


def mouse(cmd):
    _vp.stdin.write(cmd + "\n")
    _vp.stdin.flush()
    end = time.time() + 5
    while time.time() < end:
        pump(0.02)
        if _vp.stdout.readline():
            return


def node():
    stack = [json.loads(subprocess.run(["swaymsg", "-t", "get_tree"], capture_output=True, text=True).stdout)]
    while stack:
        n = stack.pop()
        if n.get("pid") == os.getpid():
            return n
        stack.extend(n.get("nodes", []) + n.get("floating_nodes", []))
    return None


def screen_xy(win, view, x, y):
    """Widget coordinates in `view` to screen coordinates."""
    rect = node()["rect"]
    vx, vy = view.translate_coordinates(win, int(x), int(y))
    bx, by = win.get_titlebar().translate_coordinates(win, 0, 0)
    return rect["x"] + vx - bx, rect["y"] + vy - by


def spot(win, name):
    row = next(r for r in win.store if os.path.basename(r[W.COL_PATH]) == name)
    if win.is_grid:
        _ok, rect = win.grid.get_cell_rect(row.path, None)
        return screen_xy(win, win.grid, rect.x + rect.width / 2, rect.y + rect.height / 3)
    rect = win.tree.get_cell_area(row.path, win.tree.get_column(0))
    bx, by = win.tree.convert_bin_window_to_widget_coords(rect.x + 30, rect.y + rect.height / 2)
    return screen_xy(win, win.tree, bx, by)


def drag(win, files, folder):
    """Select `files`, press on the first, move in steps onto `folder`, release."""
    win._unselect_all()
    for f in files:
        row = next(r for r in win.store if os.path.basename(r[W.COL_PATH]) == f)
        if win.is_grid:
            win.grid.select_path(row.path)
        else:
            win.tree.get_selection().select_path(row.path)
    pump(0.3)
    (x0, y0), (x1, y1) = spot(win, files[0]), spot(win, folder)
    mouse(f"move {int(x0)} {int(y0)}")
    mouse("press")
    if os.environ.get("DRAG_DEBUG"):
        print("DEBUG", win.is_grid, (x0, y0), (x1, y1), "selected after press:", len(win._selected()),
              "dragging:", win._dragging, file=sys.stderr)
    for i in range(1, 13):
        mouse(f"move {int(x0 + (x1 - x0) * i / 12)} {int(y0 + (y1 - y0) * i / 12)}")
        pump(0.05)
    pump(0.3)
    if os.environ.get("DRAG_DEBUG"):
        print("DEBUG before release: dragging", win._dragging, "selected", len(win._selected()), file=sys.stderr)
    mouse("release")
    for _ in range(60):
        pump(0.1)
        if all(os.path.exists(os.path.join(HOME, folder, f)) for f in files):
            break


if os.environ.get("DRAG_DEBUG"):
    def trace(name):
        orig = getattr(W.FileManagerWindow, name)
        def wrapped(self, *a):
            r = orig(self, *a)
            print("DEBUG", name, [x for x in a if isinstance(x, (int, float, str, list))], "->", r, file=sys.stderr)
            return r
        setattr(W.FileManagerWindow, name, wrapped)
    for n in ("_on_drag_drop", "_on_drag_data_received", "_transfer"):
        trace(n)
    _orig_df = W.FileManagerWindow._drop_folder
    def df(self, x, y):
        r = _orig_df(self, x, y)
        if self.is_grid:
            target = next(row for row in self.store if row[W.COL_PATH].endswith("Target"))
            _ok, rect = self.grid.get_cell_rect(target.path, None)
            print("DEBUG drop", x, y, "dest:", self.grid.get_dest_item_at_pos(x, y), "path_at:", self.grid.get_path_at_pos(x, y),
                  "Target rect:", rect.x, rect.y, rect.width, rect.height, "->", r[1][-12:], file=sys.stderr)
        return r
    W.FileManagerWindow._drop_folder = df

for view in ("grid", "list"):
    base = os.path.join(HOME, view)
    os.makedirs(os.path.join(base, "Target"))
    for f in ("a.txt", "b.txt", "c.txt"):
        open(os.path.join(base, f), "w").write(f)
win = W.FileManagerWindow(os.path.join(HOME, "grid"))
win._error = lambda *a, **k: None
win.show_all()
pump(1.0)
check("the Files window is on screen", node() is not None)

for view in ("grid", "list"):
    win._set_view(view)
    win._navigate(os.path.join(HOME, view))
    pump(0.8)
    global_home = HOME
    HOME = os.path.join(global_home, view)
    drag(win, ["a.txt", "b.txt", "c.txt"], "Target")
    moved = sorted(os.listdir(os.path.join(HOME, "Target")))
    left = sorted(f for f in os.listdir(HOME) if f.endswith(".txt"))
    check(f"{view}: dragging one of three selected files moves all three", moved == ["a.txt", "b.txt", "c.txt"] and not left,
          f"moved {moved}, left {left}")
    HOME = global_home

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
