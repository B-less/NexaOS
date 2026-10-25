"""Files: browse, select and manage files.

Selection works like other desktops: click, Ctrl+click, Shift+click, drag a
box from empty space, Ctrl+A. Selected items can be dragged onto a folder
(move; hold Ctrl to copy) or into another app, cut/copied/pasted, renamed,
duplicated, sent to the Trash and brought back. Every change runs in
nexa-fileops (Rust, see engine.py): nothing is ever overwritten and each
change can be undone with Ctrl+Z."""
import os
import pwd
import stat as statmod
import subprocess
import sys
import threading
import time
import json

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, Gio, GLib, GdkPixbuf, GObject

from . import engine, fileops, thumbs
from nexa_common import removable, themes
from nexa_common.titlebar import attach_titlebar

ICON_SIZES = [16, 20, 24, 32, 48, 64]
GRID_SIZES = [64, 96, 128, 160, 200, 256]
DEFAULT_GRID_SIZE_INDEX = 2  # 128px
DEFAULT_ICON_SIZE_INDEX = 2  # 24px, matches the size the old fixed icon-name rendering used
CONFIG_PATH = os.path.expanduser("~/.config/nexa-files/config.json")
TRASH = "trash:///"
TERMINALS = ("foot", "alacritty", "kitty", "xterm")

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#pathentry { background-image: none; background-color: @nexa_field; color: @nexa_text_strong; border-radius: 6px; padding: 6px 10px; border: 1px solid @nexa_border; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#toolbtn:disabled { color: @nexa_disabled; }
#toolbtn.nx-active { background-color: @nexa_accent; color: @nexa_selected_text; }
treeview { background-image: none; background-color: @nexa_bg; color: @nexa_text; }
treeview:selected { background-image: none; background-color: @nexa_selected; color: @nexa_selected_text; }
treeview.rubberband, .rubberband { background-color: alpha(@nexa_accent, 0.18); border: 1px solid @nexa_accent; }
treeview header button { background-image: none; background-color: @nexa_surface; color: @nexa_muted; }
iconview { background-image: none; background-color: @nexa_bg; color: @nexa_text; }
iconview:selected { background-image: none; background-color: @nexa_selected; color: @nexa_selected_text; border-radius: 10px; }
iconview .rubberband, iconview.rubberband { background-color: alpha(@nexa_accent, 0.18); border: 1px solid @nexa_accent; }
#statusbar { background-image: none; background-color: @nexa_surface; color: @nexa_muted; font-size: 9pt; }
#statusrow { background-image: none; background-color: @nexa_surface; }
progressbar trough { min-height: 6px; border-radius: 3px; background-color: @nexa_hover; }
progressbar progress { min-height: 6px; border-radius: 3px; background-color: @nexa_accent; }
"""

(COL_ICON, COL_NAME, COL_SIZE, COL_MTIME, COL_PATH, COL_IS_DIR,
 COL_BYTES, COL_TIME, COL_ACTIVE, COL_TIP) = range(10)


def _uri_targets():
    return [Gtk.TargetEntry.new("text/uri-list", 0, 0)]


def _mods(state):
    return (bool(state & Gdk.ModifierType.CONTROL_MASK), bool(state & Gdk.ModifierType.SHIFT_MASK),
            bool(state & Gdk.ModifierType.MOD1_MASK))


def _items(n):
    return "1 item" if n == 1 else f"{n} items"


class FileManagerWindow(Gtk.Window):
    open_windows = 0

    def __init__(self, start_dir=None):
        super().__init__(title="Files")
        self.set_icon_name("nexa-files")
        attach_titlebar(self, "Files")
        self.set_default_size(880, 580)
        self.path = os.path.abspath(start_dir or os.environ.get("HOME", "/root"))
        self.show_hidden = False
        self.clipboard = None          # {"paths": [...], "cut": bool}
        self.undo_stack = []           # [(what was done, steps)]
        self.back_stack, self.forward_stack = [], []
        prefs = self._load_prefs()
        self.icon_size_index = prefs["icon_size_index"]
        self.grid_size_index = prefs["grid_size_index"]
        self.view_mode = prefs["view"]
        self.thumbnailer = thumbs.Thumbnailer()
        self._thumb_cache = {}  # (path, mtime, size) -> pixbuf
        self.busy = False
        self._proc = None
        self._pending_click = None
        self._dragging = False
        self._drop_action = Gdk.DragAction.MOVE
        self._monitor = None
        self._refresh_soon_id = 0
        self._summary = ""

        FileManagerWindow.open_windows += 1
        self.connect("destroy", self._on_destroy)
        self._build_ui()
        for child in self.views.get_children():
            child.show_all()
        self._set_view(self.view_mode)
        self._navigate(self.path, remember=False)

    # ---- Settings ----

    def _load_prefs(self):
        prefs = {"icon_size_index": DEFAULT_ICON_SIZE_INDEX, "grid_size_index": DEFAULT_GRID_SIZE_INDEX, "view": "grid"}
        try:
            with open(CONFIG_PATH) as f:
                saved = json.load(f)
        except (OSError, ValueError):
            saved = {}
        if isinstance(saved, dict):
            for key, sizes in (("icon_size_index", ICON_SIZES), ("grid_size_index", GRID_SIZES)):
                if isinstance(saved.get(key), int) and 0 <= saved[key] < len(sizes):
                    prefs[key] = saved[key]
            if saved.get("view") in ("grid", "list"):
                prefs["view"] = saved["view"]
        return prefs

    def _save_prefs(self):
        try:
            os.makedirs(os.path.dirname(CONFIG_PATH), exist_ok=True)
            with open(CONFIG_PATH, "w") as f:
                json.dump({"icon_size_index": self.icon_size_index, "grid_size_index": self.grid_size_index,
                           "view": self.view_mode}, f)
        except OSError:
            pass

    @property
    def is_grid(self):
        return self.view_mode == "grid"

    @property
    def view(self):
        """The view in use: the grid (IconView) or the list (TreeView)."""
        return self.grid if self.is_grid else self.tree

    def _shown_size(self):
        return GRID_SIZES[self.grid_size_index] if self.is_grid else ICON_SIZES[self.icon_size_index]

    # The two views differ in API; everything else goes through these.
    def _selected_paths(self):
        return self.grid.get_selected_items() if self.is_grid else self.tree.get_selection().get_selected_rows()[1]

    def _unselect_all(self):
        (self.grid if self.is_grid else self.tree.get_selection()).unselect_all()

    def _select_path(self, path):
        (self.grid if self.is_grid else self.tree.get_selection()).select_path(path)

    def _is_selected(self, path):
        return (self.grid if self.is_grid else self.tree.get_selection()).path_is_selected(path)

    def _count_selected(self):
        return len(self._selected_paths())

    def _path_at(self, x, y):
        if self.is_grid:
            return self.grid.get_path_at_pos(int(x), int(y))
        hit = self.tree.get_path_at_pos(int(x), int(y))
        return hit[0] if hit else None

    def _set_cursor(self, path):
        self.view.set_cursor(path, None, False)

    def _scroll_to(self, path):
        if self.is_grid:
            self.grid.scroll_to_path(path, False, 0, 0)
        else:
            self.tree.scroll_to_cell(path, None, False, 0, 0)

    def _drop_row_at(self, x, y):
        if self.is_grid:
            # PyGObject gives (path, position), or (found, path, position)
            # in older versions; reading the first shape as the second
            # took the drop position (a number) for the item, so a drop on
            # a folder in the grid went into the folder being shown.
            found = self.grid.get_dest_item_at_pos(x, y)
            if not found:
                return None
            if len(found) == 3:
                return found[1] if found[0] else None
            return found[0]
        dest = self.tree.get_dest_row_at_pos(x, y)
        return dest[0] if dest else None

    def _highlight_drop(self, path):
        if self.is_grid:
            self.grid.set_drag_dest_item(path, Gtk.IconViewDropPosition.DROP_INTO if path else Gtk.IconViewDropPosition.NO_DROP)
        else:
            self.tree.set_drag_dest_row(path, Gtk.TreeViewDropPosition.INTO_OR_AFTER if path else Gtk.TreeViewDropPosition.BEFORE)

    def _set_view(self, mode):
        """Grid or list: the same items, selection kept."""
        keep = [i["path"] for i in self._selected()] if hasattr(self, "grid") else []
        self.view_mode = mode
        self.views.set_visible_child_name(mode)
        self.mode_btn.set_label("☰" if mode == "grid" else "▦")
        self.mode_btn.set_tooltip_text("List view (Ctrl+1)" if mode == "grid" else "Grid view (Ctrl+2)")
        (self.grid_item if mode == "grid" else self.list_item).set_active(True)
        self.grid.set_item_width(GRID_SIZES[self.grid_size_index] + 24)
        self._save_prefs()
        self._refresh(select=keep)
        self.view.grab_focus()

    def _set_sort(self, col, reverse):
        cur_col, cur_order = self.store.get_sort_column_id()
        col = cur_col if col is None else col
        if reverse is None:
            reverse = cur_order == Gtk.SortType.DESCENDING
        self.store.set_sort_column_id(col, Gtk.SortType.DESCENDING if reverse else Gtk.SortType.ASCENDING)

    def _icon_pixbuf(self, icon_name):
        try:
            return Gtk.IconTheme.get_default().load_icon(icon_name, self._shown_size(), Gtk.IconLookupFlags.FORCE_SIZE)
        except GLib.Error:
            return None

    # ---- Layout ----

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        self.back_btn = self._toolbtn("←", self._on_back, "Back (Alt+Left, or the mouse's back button)")
        self.forward_btn = self._toolbtn("→", self._on_forward, "Forward (Alt+Right)")
        self.up_btn = self._toolbtn("↑", self._on_up, "Parent folder (Alt+Up or Backspace)")
        self.home_btn = self._toolbtn("Home", self._on_home, "Home folder (Alt+Home)")
        self.trash_btn = self._toolbtn("Trash", self._on_show_trash, "What you deleted; restore it from here")
        self.devices_btn = self._toolbtn("Devices", self._on_devices, "USB drives and SD cards")
        for b in (self.back_btn, self.forward_btn, self.up_btn, self.home_btn, self.trash_btn, self.devices_btn):
            toolbar.pack_start(b, False, False, 0)

        self.path_entry = Gtk.Entry()
        self.path_entry.set_name("pathentry")
        self.path_entry.set_width_chars(8)  # shrinks first when the window is narrow
        self.path_entry.connect("activate", self._on_path_entered)
        toolbar.pack_start(self.path_entry, True, True, 0)

        # Hidden files, icon size, refresh: a View menu keeps the bar short.
        self.view_btn = Gtk.MenuButton(label="View ▾")
        self.view_btn.set_name("toolbtn")
        view_menu = Gtk.Menu()
        self.grid_item = Gtk.RadioMenuItem(label="Grid  (Ctrl+2)")
        self.list_item = Gtk.RadioMenuItem.new_with_label_from_widget(self.grid_item, "List  (Ctrl+1)")
        self.grid_item.connect("toggled", lambda i: i.get_active() and self._set_view("grid"))
        self.list_item.connect("toggled", lambda i: i.get_active() and self._set_view("list"))
        sort_item = Gtk.MenuItem(label="Sort by")
        sort_menu = Gtk.Menu()
        self.sort_items = {}
        group = None
        for col, label in ((COL_NAME, "Name"), (COL_BYTES, "Size"), (COL_TIME, "Modified")):
            it = Gtk.RadioMenuItem(label=label) if group is None else Gtk.RadioMenuItem.new_with_label_from_widget(group, label)
            group = group or it
            it.connect("toggled", lambda i, c=col: i.get_active() and self._set_sort(c, None))
            sort_menu.append(it)
            self.sort_items[col] = it
        sort_menu.append(Gtk.SeparatorMenuItem())
        self.reverse_item = Gtk.CheckMenuItem(label="Reversed")
        self.reverse_item.connect("toggled", lambda i: self._set_sort(None, i.get_active()))
        sort_menu.append(self.reverse_item)
        sort_menu.show_all()
        sort_item.set_submenu(sort_menu)
        self.hidden_item = Gtk.CheckMenuItem(label="Show Hidden Files  (Ctrl+H)")
        self.hidden_item.connect("toggled", lambda i: i.get_active() != self.show_hidden and self._on_toggle_hidden(None))
        self.zoom_in_item = Gtk.MenuItem(label="Larger Icons  (Ctrl++)")
        self.zoom_in_item.connect("activate", self._on_zoom_in)
        self.zoom_out_item = Gtk.MenuItem(label="Smaller Icons  (Ctrl+−)")
        self.zoom_out_item.connect("activate", self._on_zoom_out)
        normal = Gtk.MenuItem(label="Normal Size  (Ctrl+0)")
        normal.connect("activate", lambda _i: self._set_zoom(DEFAULT_GRID_SIZE_INDEX if self.is_grid else DEFAULT_ICON_SIZE_INDEX))
        refresh = Gtk.MenuItem(label="Refresh  (F5)")
        refresh.connect("activate", lambda _i: self._refresh())
        for item in (self.grid_item, self.list_item, sort_item, Gtk.SeparatorMenuItem(), self.hidden_item,
                     Gtk.SeparatorMenuItem(), self.zoom_in_item, self.zoom_out_item, normal,
                     Gtk.SeparatorMenuItem(), refresh):
            view_menu.append(item)
        view_menu.show_all()
        self.view_btn.set_popup(view_menu)
        self.mode_btn = self._toolbtn("", lambda _b: self._set_view("list" if self.is_grid else "grid"), None)
        toolbar.pack_start(self.mode_btn, False, False, 0)
        toolbar.pack_start(self.view_btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        # Two action bars: files, and the Trash.
        self.actions = Gtk.Stack()
        files_bar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        files_bar.set_name("toolbar")
        files_bar.set_border_width(8)
        self.new_folder_btn = self._toolbtn("New Folder", self._on_new_folder, "Ctrl+Shift+N")
        self.cut_btn = self._toolbtn("Cut", self._on_cut, "Ctrl+X")
        self.copy_btn = self._toolbtn("Copy", self._on_copy, "Ctrl+C")
        self.paste_btn = self._toolbtn("Paste", self._on_paste, "Ctrl+V")
        self.rename_btn = self._toolbtn("Rename", self._on_rename, "F2")
        self.delete_btn = self._toolbtn("Delete", self._on_trash, "Move to the Trash (Delete key); Ctrl+Z brings it back. "
                                        "Shift+Delete deletes for good.")
        self.open_btn = self._toolbtn("Open", self._on_open_selected, "Enter")
        self.undo_btn = self._toolbtn("Undo", self._on_undo, "Ctrl+Z")
        for b in (self.new_folder_btn, self.cut_btn, self.copy_btn, self.paste_btn, self.rename_btn,
                  self.delete_btn, self.open_btn):
            files_bar.pack_start(b, False, False, 0)
        files_bar.pack_end(self.undo_btn, False, False, 0)
        trash_bar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        trash_bar.set_name("toolbar")
        trash_bar.set_border_width(8)
        self.restore_btn = self._toolbtn("Restore", self._on_restore, "Put back where it was")
        self.purge_btn = self._toolbtn("Delete for Good", self._on_delete_forever, "Shift+Delete")
        self.empty_btn = self._toolbtn("Empty Trash", self._on_empty_trash, None)
        for b in (self.restore_btn, self.purge_btn):
            trash_bar.pack_start(b, False, False, 0)
        trash_bar.pack_end(self.empty_btn, False, False, 0)
        self.actions.add_named(files_bar, "files")
        self.actions.add_named(trash_bar, "trash")
        root.pack_start(self.actions, False, False, 0)

        list_scroller = Gtk.ScrolledWindow()
        self.store = Gtk.ListStore(GdkPixbuf.Pixbuf, str, str, str, str, bool, GObject.TYPE_INT64, float, bool, str)
        for col in (COL_NAME, COL_BYTES, COL_TIME):
            self.store.set_sort_func(col, self._sort, col)
        self.store.set_sort_column_id(COL_NAME, Gtk.SortType.ASCENDING)
        self.tree = Gtk.TreeView(model=self.store)
        self.tree.set_headers_visible(True)
        self.tree.set_rubber_banding(True)
        self.tree.set_search_column(COL_NAME)  # type a name to jump to it
        self.tree.set_tooltip_column(COL_TIP)
        self.tree.get_selection().set_mode(Gtk.SelectionMode.MULTIPLE)
        self.tree.connect("row-activated", self._on_row_activated)
        self.tree.get_selection().connect("changed", self._on_selection_changed)
        self.tree.connect("button-press-event", self._on_button_press)
        self.tree.connect("button-release-event", self._on_button_release)
        self.tree.connect("scroll-event", self._on_scroll)
        self.tree.connect("popup-menu", self._on_menu_key)

        icon_renderer = Gtk.CellRendererPixbuf()
        name_renderer = Gtk.CellRendererText()
        name_col = Gtk.TreeViewColumn("Name")
        name_col.pack_start(icon_renderer, False)
        name_col.add_attribute(icon_renderer, "pixbuf", COL_ICON)
        name_col.add_attribute(icon_renderer, "sensitive", COL_ACTIVE)
        name_col.pack_start(name_renderer, True)
        name_col.add_attribute(name_renderer, "text", COL_NAME)
        name_col.add_attribute(name_renderer, "sensitive", COL_ACTIVE)
        name_col.set_expand(True)
        name_col.set_resizable(True)
        name_col.set_sort_column_id(COL_NAME)
        self.tree.append_column(name_col)

        size_renderer = Gtk.CellRendererText()
        size_renderer.set_property("xalign", 1.0)
        size_col = Gtk.TreeViewColumn("Size", size_renderer, text=COL_SIZE)
        size_col.set_fixed_width(90)
        size_col.set_sort_column_id(COL_BYTES)
        self.tree.append_column(size_col)

        self.mtime_col = Gtk.TreeViewColumn("Modified", Gtk.CellRendererText(), text=COL_MTIME)
        self.mtime_col.set_fixed_width(150)
        self.mtime_col.set_sort_column_id(COL_TIME)
        self.tree.append_column(self.mtime_col)

        # Drag selected items out (to a folder here, another window, another
        # app); drop files in. Drops are handled by hand (not the tree's
        # row-reordering) so only folders light up as targets.
        self.tree.enable_model_drag_source(Gdk.ModifierType.BUTTON1_MASK, _uri_targets(),
                                           Gdk.DragAction.COPY | Gdk.DragAction.MOVE)

        # The grid: the same items as big icons and thumbnails, in rows.
        self.grid = Gtk.IconView(model=self.store)
        self.grid.set_pixbuf_column(COL_ICON)
        self.grid.set_text_column(COL_NAME)
        self.grid.set_tooltip_column(COL_TIP)
        self.grid.set_selection_mode(Gtk.SelectionMode.MULTIPLE)
        self.grid.set_item_padding(6)
        self.grid.set_column_spacing(6)
        self.grid.set_row_spacing(10)
        self.grid.set_margin(12)
        self.grid.connect("item-activated", lambda _v, path: self._activate_path(path))
        self.grid.connect("selection-changed", self._on_selection_changed)
        self.grid.enable_model_drag_source(Gdk.ModifierType.BUTTON1_MASK, _uri_targets(),
                                           Gdk.DragAction.COPY | Gdk.DragAction.MOVE)
        for w in (self.tree, self.grid):
            if w is self.grid:
                for sig, handler in (("button-press-event", self._on_button_press), ("button-release-event", self._on_button_release),
                                     ("scroll-event", self._on_scroll), ("popup-menu", self._on_menu_key)):
                    w.connect(sig, handler)
            w.connect("drag-begin", self._on_drag_begin)
            w.connect("drag-end", self._on_drag_end)
            w.connect("drag-data-get", self._on_drag_data_get)
            w.connect("drag-data-delete", lambda w, _c: w.stop_emission_by_name("drag-data-delete"))
            w.drag_dest_set(0, _uri_targets(), Gdk.DragAction.COPY | Gdk.DragAction.MOVE)
            w.connect("drag-motion", self._on_drag_motion)
            w.connect("drag-leave", lambda _w, _c, _t: self._highlight_drop(None))
            w.connect("drag-drop", self._on_drag_drop)
            w.connect("drag-data-received", self._on_drag_data_received)

        list_scroller.add(self.tree)
        grid_scroller = Gtk.ScrolledWindow()
        grid_scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        grid_scroller.add(self.grid)
        self.views = Gtk.Stack()
        self.views.add_named(grid_scroller, "grid")
        self.views.add_named(list_scroller, "list")
        root.pack_start(self.views, True, True, 0)

        status_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        status_row.set_name("statusrow")
        self.status = Gtk.Label(label="")
        self.status.set_name("statusbar")
        self.status.set_xalign(0)
        self.status.set_ellipsize(3)  # Pango.EllipsizeMode.END
        self.status.set_margin_start(10)
        self.status.set_margin_top(4)
        self.status.set_margin_bottom(4)
        status_row.pack_start(self.status, True, True, 0)
        self.progress = Gtk.ProgressBar()
        self.progress.set_valign(Gtk.Align.CENTER)
        self.progress.set_size_request(180, -1)
        self.progress.set_no_show_all(True)
        status_row.pack_start(self.progress, False, False, 0)
        self.cancel_btn = self._toolbtn("Cancel", self._on_cancel, "Stop the copy or move")
        self.cancel_btn.set_no_show_all(True)
        self.cancel_btn.set_margin_end(6)
        status_row.pack_start(self.cancel_btn, False, False, 0)
        root.pack_start(status_row, False, False, 0)

        self.connect("key-press-event", self._on_key)
        self.connect("button-press-event", self._on_window_button)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _toolbtn(self, label, handler, tip):
        btn = Gtk.Button(label=label)
        btn.set_name("toolbtn")
        if tip:
            btn.set_tooltip_text(tip)
        btn.connect("clicked", handler)
        return btn

    def _on_destroy(self, _w):
        FileManagerWindow.open_windows -= 1
        if self._proc:
            self._proc.terminate()
        if FileManagerWindow.open_windows <= 0 and Gtk.main_level() > 0:
            Gtk.main_quit()

    # ---- Listing ----

    @property
    def in_trash(self):
        return self.path == TRASH

    def _sort(self, model, a, b, col):
        da, db = model.get_value(a, COL_IS_DIR), model.get_value(b, COL_IS_DIR)
        if da != db:  # folders first, whichever way the column is sorted
            first = -1 if da else 1
            _, order = model.get_sort_column_id()
            return -first if order == Gtk.SortType.DESCENDING else first
        x, y = model.get_value(a, col), model.get_value(b, col)
        if col != COL_NAME and x == y:
            # Equal sizes or dates: by name, A to Z either way.
            x, y = model.get_value(a, COL_NAME).lower(), model.get_value(b, COL_NAME).lower()
            _, order = model.get_sort_column_id()
            return ((x > y) - (x < y)) * (-1 if order == Gtk.SortType.DESCENDING else 1)
        if col == COL_NAME:
            x, y = x.lower(), y.lower()
        return (x > y) - (x < y)

    def _refresh(self, select=None):
        """Reload the folder, keeping (or setting) which items are selected."""
        if select is None:
            select = [i["path"] for i in self._selected()]
        self._refresh_soon_id = 0
        self.path_entry.set_text("Trash" if self.in_trash else self.path)
        cut = set(self.clipboard["paths"]) if self.clipboard and self.clipboard["cut"] else set()
        self.store.clear()
        if self.in_trash:
            try:
                entries = engine.trash_list()
            except engine.EngineError as e:
                self._error(f"Can't open the Trash: {e}")
                entries = []
            for e in entries:
                when = e["deleted"].replace("T", " ")[:16]
                try:
                    ts = time.mktime(time.strptime(e["deleted"], "%Y-%m-%dT%H:%M:%S"))
                except ValueError:
                    ts = 0.0
                icon = fileops.guess_icon(e["name"], e["is_dir"], False)
                self.store.append([self._icon_pixbuf(icon), e["name"], fileops.human_size(e["size"], False),
                                   when, e["trashed"], e["is_dir"], e["size"], ts, True,
                                   GLib.markup_escape_text(f"From {os.path.dirname(e['original'])}")])
            self._summary = "The Trash is empty" if not entries else f"{_items(len(entries))} in the Trash"
        else:
            try:
                entries = fileops.list_dir(self.path, self.show_hidden)
            except fileops.FileOpError as e:
                self._error(f"Can't open {self.path}: {e}")
                return
            dirs = files = 0
            for e in entries:
                icon_name = fileops.guess_icon(e["name"], e["is_dir"], e["is_exec"])
                self.store.append([
                    self._icon_pixbuf(icon_name), e["name"], fileops.human_size(e["size"], e["is_dir"]),
                    fileops.human_time(e["mtime"]), e["path"], e["is_dir"], e["size"], e["mtime"],
                    e["path"] not in cut, None,
                ])
                dirs += e["is_dir"]
                files += not e["is_dir"]
            self._summary = f"{dirs} folder{'s' if dirs != 1 else ''}, {files} file{'s' if files != 1 else ''}"
        self._select_paths(select)
        self._update_action_sensitivity()
        self._show_selection_summary()
        if not self.in_trash:
            self._load_thumbnails()

    def _select_paths(self, paths):
        self._unselect_all()
        wanted = set(paths or [])
        first = None
        for row in self.store:
            if row[COL_PATH] in wanted:
                self._select_path(row.path)
                first = first or row.path
        if first is not None:
            if len(wanted) == 1:
                self._set_cursor(first)
            self._scroll_to(first)
            for row in self.store:  # set_cursor may have dropped the others
                if row[COL_PATH] in wanted:
                    self._select_path(row.path)

    # ---- Thumbnails ----

    def _load_thumbnails(self):
        """Pictures, videos, PDFs and folders get thumbnails (nexa-thumb, in
        the background); remembered while the window is open."""
        size = self._shown_size()
        want = 256 if size > 128 else 128
        files, folders = [], []
        for row in self.store:
            path, is_dir = row[COL_PATH], row[COL_IS_DIR]
            if not thumbs.wanted(path, is_dir):
                continue
            key = (path, row[COL_TIME], size)
            if key in self._thumb_cache:
                if self._thumb_cache[key] is not None:
                    row[COL_ICON] = self._thumb_cache[key]
                continue
            (folders if is_dir else files).append(path)
        if not files and not folders:
            return
        settings = themes.load_settings()
        accent = themes.THEMES[settings["theme"]]["accent"].lstrip("#")

        def ready(found):
            for row in self.store:
                png = found.get(row[COL_PATH])
                if not png:
                    continue
                try:
                    pix = GdkPixbuf.Pixbuf.new_from_file_at_scale(png, size, size, True)
                except GLib.Error:
                    continue
                self._thumb_cache[(row[COL_PATH], row[COL_TIME], size)] = pix
                row[COL_ICON] = pix

        self.thumbnailer.request(files, folders, want, accent, ready)

    def _navigate(self, path, remember=True):
        if path != TRASH and not os.path.isdir(path):
            self._error(f"Not a folder: {path}")
            return
        new = path if path == TRASH else os.path.abspath(path)
        if remember and new != self.path:
            self.back_stack.append(self.path)
            self.forward_stack.clear()
        self.path = new
        self.actions.set_visible_child_name("trash" if self.in_trash else "files")
        self.mtime_col.set_title("Deleted" if self.in_trash else "Modified")
        ctx = self.trash_btn.get_style_context()
        (ctx.add_class if self.in_trash else ctx.remove_class)("nx-active")
        self._watch()
        self._refresh(select=[])

    def _watch(self):
        """Refresh by itself when files change (another app, a download)."""
        if self._monitor:
            self._monitor.cancel()
            self._monitor = None
        target = os.path.join(engine_trash_dir(), "files") if self.in_trash else self.path
        try:
            self._monitor = Gio.File.new_for_path(target).monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, None)
            self._monitor.connect("changed", lambda *_a: self._refresh_soon())
        except GLib.Error:
            self._monitor = None

    def _refresh_soon(self):
        if not self._refresh_soon_id:
            self._refresh_soon_id = GLib.timeout_add(300, self._refresh_when_idle)

    def _refresh_when_idle(self):
        if self._dragging or self._pending_click is not None:
            return True  # try again once the mouse is done
        self._refresh()
        return False

    # ---- Selection ----

    def _selected(self):
        model = self.store
        return [{"path": model[p][COL_PATH], "name": model[p][COL_NAME], "is_dir": model[p][COL_IS_DIR],
                 "bytes": model[p][COL_BYTES]} for p in self._selected_paths()]

    def _selected_entry(self):
        sel = self._selected()
        return sel[0] if len(sel) == 1 else None

    def _update_action_sensitivity(self):
        n = len(self._selected())
        for b in (self.cut_btn, self.copy_btn, self.delete_btn, self.open_btn, self.restore_btn, self.purge_btn):
            b.set_sensitive(n > 0)
        self.rename_btn.set_sensitive(n == 1)
        self.paste_btn.set_sensitive(self.clipboard is not None and not self.in_trash)
        self.undo_btn.set_sensitive(bool(self.undo_stack) and not self.busy)
        self.empty_btn.set_sensitive(len(self.store) > 0)
        self.back_btn.set_sensitive(bool(self.back_stack))
        self.forward_btn.set_sensitive(bool(self.forward_stack))
        self.up_btn.set_sensitive(self.path != "/")
        index, sizes = (self.grid_size_index, GRID_SIZES) if self.is_grid else (self.icon_size_index, ICON_SIZES)
        self.zoom_out_item.set_sensitive(index > 0)
        self.zoom_in_item.set_sensitive(index < len(sizes) - 1)

    def _show_selection_summary(self):
        if self.busy:
            return
        sel = self._selected()
        if not sel:
            self.status.set_text(self._summary)
            return
        folders = sum(i["is_dir"] for i in sel)
        size = sum(i["bytes"] for i in sel if not i["is_dir"] or self.in_trash)
        parts = []
        if folders:
            parts.append(f"{folders} folder{'s' if folders != 1 else ''}")
        if size or folders < len(sel):
            parts.append(fileops.human_size(size, False))
        self.status.set_text(f"{_items(len(sel))} selected ({', '.join(parts)})")

    def _on_selection_changed(self, _sel):
        self._update_action_sensitivity()
        self._show_selection_summary()

    def select_all(self):
        (self.grid if self.is_grid else self.tree.get_selection()).select_all()

    # ---- Mouse ----

    def _on_button_press(self, view, event):
        if event.type != Gdk.EventType.BUTTON_PRESS:
            return False  # double-clicks open through row-activated
        if event.button in (8, 9):
            (self._on_back if event.button == 8 else self._on_forward)(None)
            return True
        hit = self._path_at(event.x, event.y)
        ctrl, shift, _alt = _mods(event.state)
        if event.button == 3:
            if hit is None:
                self._unselect_all()
            elif not self._is_selected(hit):
                self._unselect_all()
                self._select_path(hit)
                self._set_cursor(hit)
            self._popup(self._item_menu() if hit is not None else self._background_menu(), event)
            return True
        if event.button == 2:
            if hit is not None and self.store[hit][COL_IS_DIR] and not self.in_trash:
                self._open_window(self.store[hit][COL_PATH])
            return True
        if event.button == 1:
            if hit is None:
                if not (ctrl or shift):
                    self._unselect_all()
                return False  # a box selection can start here
            if not (ctrl or shift) and self._is_selected(hit) and self._count_selected() > 1:
                # Keep the whole selection so it can be dragged; a plain
                # click (no drag) narrows it to this item on release.
                self._pending_click = hit
                if self.is_grid:
                    # The grid narrows the selection to the pressed item
                    # itself (only that one was dragged, then moved); it
                    # must still see the press to start a drag, so the
                    # selection is put back right after it.
                    keep = self._selected_paths()
                    GLib.idle_add(self._restore_selection, keep)
                else:
                    self.tree.get_selection().set_select_function(lambda *_a: False, None)
        return False

    def _restore_selection(self, paths):
        if self._pending_click is not None:
            for p in paths:
                self.grid.select_path(p)
        return False

    def _on_button_release(self, view, _event):
        if self._pending_click is not None and not self._dragging:
            path, self._pending_click = self._pending_click, None
            if self.is_grid:
                self._unselect_all()
                self._select_path(path)
                self._set_cursor(path)
            else:
                self.tree.get_selection().set_select_function(lambda *_a: True, None)
                self.tree.set_cursor(path, None, False)
        return False

    def _on_window_button(self, _w, event):
        if event.button in (8, 9):
            (self._on_back if event.button == 8 else self._on_forward)(None)
            return True
        return False

    def _on_scroll(self, _view, event):
        if not event.state & Gdk.ModifierType.CONTROL_MASK:
            return False
        if event.direction == Gdk.ScrollDirection.UP:
            self._on_zoom_in(None)
        elif event.direction == Gdk.ScrollDirection.DOWN:
            self._on_zoom_out(None)
        elif event.direction == Gdk.ScrollDirection.SMOOTH:
            _ok, _dx, dy = event.get_scroll_deltas()
            (self._on_zoom_out if dy > 0 else self._on_zoom_in)(None)
        return True

    def _on_row_activated(self, _view, tree_path, _column):
        self._activate_path(tree_path)

    def _activate_path(self, tree_path):
        row = self.store[tree_path]
        if self.in_trash:
            self.status.set_text("Restore it to open it (right-click > Restore).")
        elif row[COL_IS_DIR]:
            self._navigate(row[COL_PATH])
        else:
            self._open_path(row[COL_PATH])

    # ---- Drag and drop ----

    def _on_drag_begin(self, _view, _ctx):
        self._dragging = True

    def _on_drag_end(self, view, _ctx):
        self._dragging = False
        if self._pending_click is not None:
            if not self.is_grid:
                self.tree.get_selection().set_select_function(lambda *_a: True, None)
            self._pending_click = None

    def _on_drag_data_get(self, _view, _ctx, data, _info, _time):
        data.set_uris([Gio.File.new_for_path(i["path"]).get_uri() for i in self._selected()])

    def _drop_folder(self, x, y):
        """The folder a drop at (x, y) goes into: the folder row under the
        pointer, or the folder being shown."""
        if self.in_trash:
            return None
        dest = self._drop_row_at(x, y)
        if dest is not None:
            row = self.store[dest]
            if row[COL_IS_DIR]:
                return dest, row[COL_PATH]
        return None, self.path

    def _on_drag_motion(self, view, ctx, x, y, t):
        target = self._drop_folder(x, y)
        dragged = {i["path"] for i in self._selected()} if Gtk.drag_get_source_widget(ctx) is view else set()
        if target is None or target[1] in dragged:
            self._highlight_drop(None)
            Gdk.drag_status(ctx, 0, t)
            return True
        _win, _x, _y, mask = view.get_window().get_device_position(ctx.get_device())
        ctrl, shift, _alt = _mods(mask)
        internal = Gtk.drag_get_source_widget(ctx) is not None  # from a Files window
        action = (Gdk.DragAction.COPY if ctrl else Gdk.DragAction.MOVE if shift
                  else Gdk.DragAction.MOVE if internal else Gdk.DragAction.COPY)
        if not ctx.get_actions() & action:
            action = ctx.get_suggested_action()
        self._highlight_drop(target[0])
        self._drop_action = action
        Gdk.drag_status(ctx, action, t)
        return True

    def _on_drag_drop(self, view, ctx, _x, _y, t):
        view.drag_get_data(ctx, Gdk.Atom.intern("text/uri-list", False), t)
        return True

    def _on_drag_data_received(self, view, ctx, x, y, data, _info, t):
        self._highlight_drop(None)
        target = self._drop_folder(x, y)
        paths = [Gio.File.new_for_uri(u).get_path() for u in (data.get_uris() or [])]
        paths = [p for p in paths if p]
        # Never ask the source to delete: a move is done here, by the engine.
        Gtk.drag_finish(ctx, bool(paths and target), False, t)
        if not paths or target is None or target[1] in paths:
            return
        self._transfer(paths, target[1], move=self._drop_action == Gdk.DragAction.MOVE)

    # ---- Keyboard ----

    def _on_key(self, _w, event):
        focus = self.get_focus()
        k = Gdk.keyval_to_lower(event.keyval)
        ctrl, shift, alt = _mods(event.state)
        if isinstance(focus, Gtk.Entry):
            if focus is self.path_entry and k == Gdk.KEY_Escape:
                self.path_entry.set_text("Trash" if self.in_trash else self.path)
                self.view.grab_focus()
                return True
            return False
        keys = {
            (True, False, False, Gdk.KEY_c): self._on_copy,
            (True, False, False, Gdk.KEY_x): self._on_cut,
            (True, False, False, Gdk.KEY_v): self._on_paste,
            (True, False, False, Gdk.KEY_a): lambda _b: self.select_all(),
            (True, False, False, Gdk.KEY_z): self._on_undo,
            (True, False, False, Gdk.KEY_h): self._on_toggle_hidden,
            (True, False, False, Gdk.KEY_l): lambda _b: self._focus_path(),
            (True, False, False, Gdk.KEY_r): lambda _b: self._refresh(),
            (True, False, False, Gdk.KEY_plus): self._on_zoom_in,
            (True, False, False, Gdk.KEY_equal): self._on_zoom_in,
            (True, False, False, Gdk.KEY_minus): self._on_zoom_out,
            (True, False, False, Gdk.KEY_0): lambda _b: self._set_zoom(DEFAULT_GRID_SIZE_INDEX if self.is_grid else DEFAULT_ICON_SIZE_INDEX),
            (True, True, False, Gdk.KEY_n): self._on_new_folder,
            (False, False, False, Gdk.KEY_F2): self._on_rename,
            (False, False, False, Gdk.KEY_F5): lambda _b: self._refresh(),
            (False, False, False, Gdk.KEY_Delete): self._on_delete_forever if self.in_trash else self._on_trash,
            (False, True, False, Gdk.KEY_Delete): self._on_delete_forever,
            (False, False, False, Gdk.KEY_BackSpace): self._on_up,
            (False, False, True, Gdk.KEY_Up): self._on_up,
            (False, False, True, Gdk.KEY_Left): self._on_back,
            (False, False, True, Gdk.KEY_Right): self._on_forward,
            (False, False, True, Gdk.KEY_Home): self._on_home,
            (False, False, True, Gdk.KEY_Return): lambda _b: self._properties(),
            (False, False, False, Gdk.KEY_Return): self._on_open_selected,
            (False, False, False, Gdk.KEY_KP_Enter): self._on_open_selected,
            (False, False, False, Gdk.KEY_Escape): lambda _b: self._unselect_all(),
            (True, False, False, Gdk.KEY_1): lambda _b: self._set_view("list"),
            (True, False, False, Gdk.KEY_2): lambda _b: self._set_view("grid"),
        }
        handler = keys.get((ctrl, shift, alt, k))
        if handler is None:
            return False
        handler(None)
        return True

    def _focus_path(self):
        self.path_entry.grab_focus()
        self.path_entry.select_region(0, -1)

    # ---- Menus ----

    def _popup(self, entries, event=None):
        menu = Gtk.Menu()
        for e in entries:
            if e is None:
                menu.append(Gtk.SeparatorMenuItem())
                continue
            label, handler, *rest = e
            if rest and rest[0] == "check":
                item = Gtk.CheckMenuItem(label=label)
                item.set_active(rest[1])
            else:
                item = Gtk.MenuItem(label=label)
                item.set_sensitive(rest[0] if rest else True)
            item.connect("activate", lambda _i, h=handler: h(None))
            menu.append(item)
        menu.show_all()
        menu.attach_to_widget(self.view, None)
        if event is not None:
            menu.popup_at_pointer(event)
        else:
            menu.popup_at_widget(self.view, Gdk.Gravity.CENTER, Gdk.Gravity.NORTH_WEST, None)
        self._menu = menu  # keep it alive while shown
        return menu

    def _item_menu(self):
        sel = self._selected()
        n = len(sel)
        single = sel[0] if n == 1 else None
        if self.in_trash:
            return [("Restore", self._on_restore), ("Delete for Good…", self._on_delete_forever), None,
                    ("Select All", lambda _b: self.select_all())]
        folder = single and single["is_dir"]
        return [
            ("Open", self._on_open_selected),
            ("Open in New Window", lambda _b: [self._open_window(i["path"]) for i in sel if i["is_dir"]], any(i["is_dir"] for i in sel)),
            ("Open With…", self._on_open_with, bool(single and not single["is_dir"])),
            None,
            ("Cut", self._on_cut), ("Copy", self._on_copy),
            ("Paste Into Folder", lambda _b: self._paste_into(single["path"]), bool(folder and self.clipboard)),
            ("Duplicate", self._on_duplicate),
            ("Rename…", self._on_rename, n == 1),
            None,
            ("Copy Location", self._on_copy_location),
            ("Send to a Device…", self._on_send_to_device, bool(sel) and not any(i["is_dir"] for i in sel)),
            ("Move to Trash", self._on_trash),
            ("Delete for Good…", self._on_delete_forever),
            None,
            ("Properties", lambda _b: self._properties()),
        ]

    def _on_send_to_device(self, _b=None):
        """Nexa Link: send the selected files to a nearby device."""
        from nexa_common import linkui
        files = [i["path"] for i in self._selected() if not i["is_dir"]]
        chosen = linkui.pick_device(self, len(files)) if files else None
        if chosen:
            linkui.send_files(chosen[0], chosen[1], files, lambda text, error=False: self.status.set_text(text))

    def _background_menu(self):
        if self.in_trash:
            return [("Empty Trash", self._on_empty_trash, len(self.store) > 0), ("Select All", lambda _b: self.select_all()),
                    ("Refresh", lambda _b: self._refresh())]
        return [
            ("New Folder…", self._on_new_folder),
            ("New Empty File…", self._on_new_file),
            ("Paste", self._on_paste, self.clipboard is not None),
            None,
            ("Select All", lambda _b: self.select_all()),
            ("Show Hidden Files", self._on_toggle_hidden, "check", self.show_hidden),
            ("Refresh", lambda _b: self._refresh()),
            None,
            ("Open Terminal Here", self._on_terminal, self._terminal() is not None),
            ("Properties", lambda _b: self._properties([{"path": self.path, "name": os.path.basename(self.path) or "/", "is_dir": True}])),
        ]

    def _on_menu_key(self, _view):
        self._popup(self._item_menu() if self._selected() else self._background_menu())
        return True

    # ---- Opening ----

    def _content_type(self, path):
        gfile = Gio.File.new_for_path(path)
        info = gfile.query_info(
            Gio.FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE, Gio.FileQueryInfoFlags.NONE, None,
        )
        return gfile, info.get_content_type()

    def _open_path(self, path):
        # Real freedesktop MIME association (Gio.AppInfo, reading the
        # mimeinfo.cache built from every app's MimeType=) rather than a
        # hardcoded extension chain here -- any future app just needs a
        # MimeType= entry in its .desktop file to become available, no
        # change needed in this file.
        gfile, content_type = self._content_type(path)
        app_info = Gio.AppInfo.get_default_for_type(content_type, False)
        if app_info is None:
            self._error(f"No app associated with this file type ({content_type})")
            return
        try:
            app_info.launch([gfile], None)
        except GLib.Error as e:
            self._error(f"Couldn't launch {app_info.get_display_name()}: {e.message}")

    def _on_open_selected(self, _btn):
        sel = self._selected()
        if self.in_trash or not sel:
            return
        folders = [i for i in sel if i["is_dir"]]
        if len(sel) == 1 and folders:
            self._navigate(folders[0]["path"])
            return
        for i in sel:
            if i["is_dir"]:
                self._open_window(i["path"])
            else:
                self._open_path(i["path"])

    def _open_window(self, path):
        win = FileManagerWindow(path)
        win.show_all()

    def _on_open_with(self, _btn):
        sel = self._selected_entry()
        if not sel or sel["is_dir"]:
            return
        gfile, content_type = self._content_type(sel["path"])
        candidates = Gio.AppInfo.get_all_for_type(content_type) or Gio.AppInfo.get_all()
        if not candidates:
            self._error("No applications are available to open this file.")
            return

        dialog = Gtk.Dialog(title="Open With", transient_for=self, modal=True)
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Open", Gtk.ResponseType.OK)
        dialog.set_default_response(Gtk.ResponseType.OK)
        box = dialog.get_content_area()
        box.set_border_width(10)

        listbox = Gtk.ListBox()
        for app in candidates:
            row = Gtk.ListBoxRow()
            row.app_info = app
            row.add(Gtk.Label(label=app.get_display_name(), xalign=0))
            listbox.add(row)
        listbox.select_row(listbox.get_row_at_index(0))
        box.add(listbox)

        default_check = Gtk.CheckButton(label="Set as default for this file type")
        default_check.set_margin_top(8)
        box.add(default_check)

        dialog.show_all()
        response = dialog.run()
        chosen_row = listbox.get_selected_row()
        set_default = default_check.get_active()
        dialog.destroy()

        if response != Gtk.ResponseType.OK or chosen_row is None:
            return
        app = chosen_row.app_info
        try:
            app.launch([gfile], None)
        except GLib.Error as e:
            self._error(f"Couldn't launch {app.get_display_name()}: {e.message}")
            return
        if set_default:
            try:
                app.set_as_default_for_type(content_type)
            except GLib.Error:
                pass

    def _terminal(self):
        for t in TERMINALS:
            for d in os.environ.get("PATH", "/usr/bin:/bin").split(":"):
                if os.access(os.path.join(d, t), os.X_OK):
                    return os.path.join(d, t)
        return None

    def _on_terminal(self, _btn):
        term = self._terminal()
        if term and not self.in_trash:
            subprocess.Popen([term], cwd=self.path, start_new_session=True,
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    # ---- Removable drives ----

    def _in_thread(self, work, done):
        def run():
            try:
                result, error = work(), None
            except removable.RemovableError as e:
                result, error = None, str(e)
            GLib.idle_add(done, result, error)
        threading.Thread(target=run, daemon=True).start()

    def _on_devices(self, _btn):
        self.devices_btn.set_sensitive(False)
        self._in_thread(removable.list_volumes, self._show_devices_menu)

    def _show_devices_menu(self, volumes, error):
        self.devices_btn.set_sensitive(True)
        menu = Gtk.Menu()
        if error or not volumes:
            item = Gtk.MenuItem(label=error or "No removable drives")
            item.set_sensitive(False)
            menu.append(item)
        for vol in volumes or []:
            name = removable.display_name(vol)
            open_item = Gtk.MenuItem(label=f"Open {name}")
            open_item.connect("activate", lambda _i, v=vol: self._open_device(v))
            menu.append(open_item)
            if vol["mountpoint"]:
                eject_item = Gtk.MenuItem(label=f"Eject {name}")
                eject_item.connect("activate", lambda _i, v=vol: self._eject_device(v))
                menu.append(eject_item)
        menu.show_all()
        menu.popup_at_widget(self.devices_btn, Gdk.Gravity.SOUTH_WEST, Gdk.Gravity.NORTH_WEST, None)
        return False

    def _open_device(self, vol):
        if vol["mountpoint"]:
            self._navigate(vol["mountpoint"])
            return
        self.status.set_text(f"Opening {removable.display_name(vol)}...")

        def done(result, error):
            if error:
                self._error(error)
                self._refresh()
            else:
                self._navigate(result["mountpoint"])
            return False

        self._in_thread(lambda: removable.mount(vol["name"]), done)

    def _eject_device(self, vol):
        name = removable.display_name(vol)
        self.status.set_text(f"Ejecting {name}...")

        def done(_result, error):
            if error:
                self._error(error)
            else:
                if self.path.startswith(vol["mountpoint"]):
                    self._navigate(os.environ.get("HOME", "/root"))
                self.status.set_text(f"{name} can be safely removed.")
            return False

        self._in_thread(lambda: removable.eject(vol["name"]), done)

    # ---- Navigation ----

    def _on_back(self, _btn):
        if self.back_stack:
            self.forward_stack.append(self.path)
            self._navigate(self.back_stack.pop(), remember=False)

    def _on_forward(self, _btn):
        if self.forward_stack:
            self.back_stack.append(self.path)
            self._navigate(self.forward_stack.pop(), remember=False)

    def _on_up(self, _btn):
        came_from = self.path
        if self.in_trash:
            self._navigate(os.environ.get("HOME", "/root"))
            return
        self._navigate(os.path.dirname(self.path.rstrip("/")) or "/")
        self._select_paths([came_from])  # keep your place

    def _on_home(self, _btn):
        self._navigate(os.environ.get("HOME", "/root"))

    def _on_show_trash(self, _btn):
        self._navigate(TRASH)

    def _on_path_entered(self, entry):
        text = entry.get_text().strip()
        if text.lower() in ("trash", TRASH, "trash:"):
            self._navigate(TRASH)
        else:
            self._navigate(os.path.expanduser(text) or "/")
        self.view.grab_focus()

    def _on_toggle_hidden(self, _btn):
        self.show_hidden = not self.show_hidden
        self.hidden_item.set_active(self.show_hidden)
        self._refresh()

    def _set_zoom(self, index):
        current, sizes = (self.grid_size_index, GRID_SIZES) if self.is_grid else (self.icon_size_index, ICON_SIZES)
        if 0 <= index < len(sizes) and index != current:
            if self.is_grid:
                self.grid_size_index = index
                self.grid.set_item_width(GRID_SIZES[index] + 24)
            else:
                self.icon_size_index = index
            self._save_prefs()
            self._refresh()

    def _on_zoom_out(self, _btn):
        self._set_zoom((self.grid_size_index if self.is_grid else self.icon_size_index) - 1)

    def _on_zoom_in(self, _btn):
        self._set_zoom((self.grid_size_index if self.is_grid else self.icon_size_index) + 1)

    # ---- Changing files (all through the engine) ----

    def _job(self, label, work, done):
        """Run an engine call off the main thread, with a progress bar when
        it moves bytes and a Cancel button."""
        if self.busy:
            self.status.set_text("Wait for the current job to finish, or cancel it.")
            return
        self.busy = True
        self.status.set_text(label + "…")
        self._update_action_sensitivity()

        def progress(p):
            GLib.idle_add(self._show_progress, label, p)

        def started(proc):
            self._proc = proc

        def run():
            try:
                result, error = work(progress, started), None
            except engine.EngineError as e:
                result, error = None, str(e)
            GLib.idle_add(finish, result, error)

        def finish(result, error):
            self.busy = False
            self._proc = None
            self.progress.hide()
            self.cancel_btn.hide()
            done(result, error)
            self._update_action_sensitivity()
            return False

        threading.Thread(target=run, daemon=True).start()

    def _show_progress(self, label, p):
        if not self.busy:
            return False
        total = p.get("total") or 0
        self.progress.show()
        self.cancel_btn.show()
        self.progress.set_fraction(min(1.0, p["done"] / total) if total else 0)
        if p.get("finishing"):
            self.progress.set_fraction(1.0)
            self.status.set_text(f"{label}: finishing writing to the drive...")
            return False
        done_s = fileops.human_size(p["done"], False)
        total_s = fileops.human_size(total, False)
        self.status.set_text(f"{label}: {p.get('current', '')} - {done_s} of {total_s}")
        return False

    def _on_cancel(self, _btn):
        if self._proc:
            self._proc.terminate()

    def _changed(self, result, error, select_key=None, undo=True):
        """Show what an engine call did, remember how to undo it, refresh."""
        if error:
            self._error(error)
            self._refresh()
            return
        steps = result.get("undo") or []
        if undo and steps:
            self.undo_stack.append((result.get("done", "the last change"), steps))
            del self.undo_stack[:-50]
        failed = result.get("failed") or []
        text = result.get("done", "Done")
        if undo and steps:
            text += " - Ctrl+Z to undo"
        select = result.get(select_key) if select_key else None
        if isinstance(select, str):
            select = [select]
        self._refresh(select=select)
        self.status.set_text(text)
        if failed:
            lines = "\n".join(f"{os.path.basename(f['path'])}: {f['error']}" for f in failed[:8])
            more = f"\n…and {len(failed) - 8} more" if len(failed) > 8 else ""
            self._error(f"{_items(len(failed))} couldn't be handled", lines + more)

    def _on_copy(self, _btn):
        self._to_clipboard(cut=False)

    def _on_cut(self, _btn):
        self._to_clipboard(cut=True)

    def _to_clipboard(self, cut):
        sel = self._selected()
        if not sel or self.in_trash:
            return
        self.clipboard = {"paths": [i["path"] for i in sel], "cut": cut}
        # Other apps get the locations as text (a terminal, an editor).
        Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text("\n".join(self.clipboard["paths"]), -1)
        verb = "Cut" if cut else "Copied"
        self._refresh()
        self.status.set_text(f"{verb} {_items(len(sel))} - go to a folder and paste (Ctrl+V)")

    def _clipboard_paths(self):
        if self.clipboard:
            return self.clipboard["paths"], self.clipboard["cut"]
        # Files copied in another app (a list of file:// links).
        uris = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).wait_for_uris() or []
        paths = [Gio.File.new_for_uri(u).get_path() for u in uris]
        return [p for p in paths if p], False

    def _on_paste(self, _btn):
        if not self.in_trash:
            self._paste_into(self.path)

    def _paste_into(self, folder):
        paths, cut = self._clipboard_paths()
        if not paths:
            self.status.set_text("Nothing to paste - copy or cut something first.")
            return
        if cut:
            self.clipboard = None  # a cut is pasted once
        self._transfer(paths, folder, move=cut)

    def _transfer(self, paths, folder, move):
        verb = "Moving" if move else "Copying"
        call = engine.move if move else engine.copy
        self._job(f"{verb} {_items(len(paths))}",
                  lambda progress, started: call(paths, folder, progress=progress, started=started),
                  lambda r, e: self._changed(r, e, "moved" if move else "created"))

    def _on_duplicate(self, _btn):
        sel = self._selected()
        if sel:
            self._job(f"Duplicating {_items(len(sel))}",
                      lambda progress, started: engine.duplicate([i["path"] for i in sel], progress=progress, started=started),
                      lambda r, e: self._changed(r, e, "created"))

    def _on_trash(self, _btn):
        sel = self._selected()
        if not sel or self.in_trash:
            return
        paths = [i["path"] for i in sel]

        def done(result, error):
            no_trash = [f["path"] for f in (result or {}).get("failed", []) if "no Trash" in f["error"]]
            if error and "no Trash" in error:
                no_trash = paths
                error = None
                result = {"done": "Nothing was moved to the Trash", "failed": []}
            if result:
                result["failed"] = [f for f in result.get("failed", []) if f["path"] not in no_trash]
            self._changed(result, error)
            if no_trash and self._confirm(
                    f"Delete {_items(len(no_trash))} for good?",
                    "This drive has no Trash, so they can't be brought back afterwards."):
                self._job("Deleting", lambda progress, started: engine.delete(no_trash, started=started),
                          lambda r, e: self._changed(r, e))

        self._job(f"Moving {_items(len(paths))} to the Trash",
                  lambda progress, started: engine.trash(paths, started=started), done)

    def _on_delete_forever(self, _btn):
        sel = self._selected()
        if not sel:
            return
        what = f"“{sel[0]['name']}”" if len(sel) == 1 else _items(len(sel))
        if not self._confirm(f"Delete {what} for good?", "This can't be undone."):
            return
        self._job("Deleting", lambda progress, started: engine.delete([i["path"] for i in sel], started=started),
                  lambda r, e: self._changed(r, e))

    def _on_restore(self, _btn):
        sel = self._selected()
        if sel and self.in_trash:
            self._job(f"Restoring {_items(len(sel))}",
                      lambda progress, started: engine.restore([i["path"] for i in sel], started=started),
                      lambda r, e: self._changed(r, e))

    def _on_empty_trash(self, _btn):
        if not len(self.store):
            return
        if not self._confirm("Empty the Trash?", f"{_items(len(self.store))} will be deleted for good."):
            return
        self._job("Emptying the Trash", lambda progress, started: engine.empty_trash(), lambda r, e: self._changed(r, e))

    def _on_undo(self, _btn):
        if not self.undo_stack or self.busy:
            return
        what, steps = self.undo_stack.pop()

        def done(result, error):
            if result:
                result["done"] = f"Undone: {what}"
            self._changed(result, error, undo=False)

        self._job("Undoing", lambda progress, started: engine.undo(steps), done)

    def _on_rename(self, _btn):
        sel = self._selected_entry()
        if not sel or self.in_trash:
            return
        new_name = self._prompt("Rename", "New name:", sel["name"], select_stem=not sel["is_dir"])
        if not new_name or new_name == sel["name"]:
            return
        try:
            result = engine.rename(sel["path"], new_name)
        except engine.EngineError as e:
            self._error(str(e))
            return
        self._changed(result, None, "path")

    def _create(self, folder):
        kind = "Folder" if folder else "File"
        name = self._prompt(f"New {kind}", f"{kind} name:", "New Folder" if folder else "Untitled.txt",
                            select_stem=not folder)
        if not name:
            return
        try:
            result = (engine.new_folder if folder else engine.new_file)(self.path, name)
        except engine.EngineError as e:
            self._error(str(e))
            return
        self._changed(result, None, "path")

    def _on_new_folder(self, _btn):
        if not self.in_trash:
            self._create(folder=True)

    def _on_new_file(self, _btn):
        if not self.in_trash:
            self._create(folder=False)

    def _on_copy_location(self, _btn):
        paths = [i["path"] for i in self._selected()] or [self.path]
        Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text("\n".join(paths), -1)
        self.status.set_text("Location copied" if len(paths) == 1 else f"{len(paths)} locations copied")

    # ---- Properties ----

    def _properties(self, items=None):
        items = items or self._selected()
        if not items or self.in_trash:
            if self.in_trash:
                return
            items = [{"path": self.path, "name": os.path.basename(self.path) or "/", "is_dir": True}]
        dialog = Gtk.Dialog(title="Properties", transient_for=self, modal=True)
        dialog.add_button("Close", Gtk.ResponseType.CLOSE)
        grid = Gtk.Grid(column_spacing=16, row_spacing=8)
        grid.set_border_width(14)
        dialog.get_content_area().add(grid)
        rows = []

        def add(label, value):
            key = Gtk.Label(label=label, xalign=1)
            key.get_style_context().add_class("dim-label")
            val = Gtk.Label(label=value, xalign=0, selectable=True)
            val.set_line_wrap(True)
            val.set_max_width_chars(48)
            grid.attach(key, 0, len(rows), 1, 1)
            grid.attach(val, 1, len(rows), 1, 1)
            rows.append(val)
            return val

        if len(items) == 1:
            p = items[0]["path"]
            add("Name", items[0]["name"])
            try:
                st = os.lstat(p)
            except OSError as e:
                st = None
                add("Error", str(e))
            if st is not None:
                if statmod.S_ISLNK(st.st_mode):
                    kind = f"Link to {os.readlink(p)}"
                elif statmod.S_ISDIR(st.st_mode):
                    kind = "Folder"
                else:
                    kind = Gio.content_type_get_description(self._content_type(p)[1])
                add("Type", kind)
                add("Location", os.path.dirname(p) or "/")
                size_label = add("Size", "Calculating…" if statmod.S_ISDIR(st.st_mode) else
                                 f"{fileops.human_size(st.st_size, False)} ({st.st_size:,} bytes)")
                add("Modified", fileops.human_time(st.st_mtime))
                try:
                    owner = pwd.getpwuid(st.st_uid).pw_name
                except KeyError:
                    owner = str(st.st_uid)
                add("Permissions", f"{statmod.filemode(st.st_mode)}  (owner: {owner})")
                if not statmod.S_ISDIR(st.st_mode):
                    size_label = None
        else:
            add("Selected", _items(len(items)))
            add("Location", os.path.dirname(items[0]["path"]))
            size_label = add("Total size", "Calculating…")

        alive = {"yes": True}
        if size_label is not None:
            def measure():
                try:
                    u = engine.size([i["path"] for i in items])
                    text = (f"{fileops.human_size(u['bytes'], False)} ({u['bytes']:,} bytes) - "
                            f"{u['files']} file{'s' if u['files'] != 1 else ''}, "
                            f"{max(0, u['folders'] - sum(1 for i in items if i['is_dir']))} folders inside")
                except engine.EngineError as e:
                    text = f"Couldn't measure: {e}"
                GLib.idle_add(lambda: alive["yes"] and size_label.set_text(text) and False)
            threading.Thread(target=measure, daemon=True).start()
        dialog.show_all()
        dialog.run()
        alive["yes"] = False
        dialog.destroy()

    # ---- Dialogs ----

    def _prompt(self, title, message, default_text, select_stem=False):
        dialog = Gtk.Dialog(title=title, transient_for=self, modal=True)
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "OK", Gtk.ResponseType.OK)
        box = dialog.get_content_area()
        box.set_border_width(10)
        box.add(Gtk.Label(label=message, xalign=0))
        entry = Gtk.Entry()
        entry.set_text(default_text)
        entry.set_activates_default(True)
        entry.set_width_chars(36)
        dialog.set_default_response(Gtk.ResponseType.OK)
        box.add(entry)
        dialog.show_all()
        # Select the name without its extension, ready to type over.
        stem = os.path.splitext(default_text)[0] if select_stem and not default_text.startswith(".") else default_text
        entry.select_region(0, len(stem))
        response = dialog.run()
        text = entry.get_text().strip()
        dialog.destroy()
        return text if response == Gtk.ResponseType.OK else None

    def _confirm(self, text, secondary):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
            buttons=Gtk.ButtonsType.YES_NO, text=text,
        )
        dialog.format_secondary_text(secondary)
        response = dialog.run()
        dialog.destroy()
        return response == Gtk.ResponseType.YES

    def _error(self, text, secondary=None):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
            buttons=Gtk.ButtonsType.OK, text=text,
        )
        if secondary:
            dialog.format_secondary_text(secondary)
        dialog.run()
        dialog.destroy()


def engine_trash_dir():
    data = os.environ.get("XDG_DATA_HOME")
    base = data if data and os.path.isabs(data) else os.path.join(os.environ.get("HOME", "/root"), ".local/share")
    return os.path.join(base, "Trash")


def _start_dir(argv):
    for arg in argv[1:]:
        if arg.startswith("trash:"):
            return TRASH  # "nexa-files trash:///" opens the Trash (Nexus, MicroBrain)
        path = Gio.File.new_for_commandline_arg(arg).get_path()
        if path and os.path.isdir(path):
            return path
        if path and os.path.exists(path):
            return os.path.dirname(path)
    return None


def main():
    Gtk.init()
    win = FileManagerWindow(_start_dir(sys.argv))
    win.show_all()
    Gtk.main()
