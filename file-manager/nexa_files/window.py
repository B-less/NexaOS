import json
import os
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, Gio, GLib, GdkPixbuf

from . import fileops
from nexa_common import removable
from nexa_common.titlebar import attach_titlebar

ICON_SIZES = [16, 20, 24, 32, 48, 64]
DEFAULT_ICON_SIZE_INDEX = 2  # 24px, matches the size the old fixed icon-name rendering used
CONFIG_PATH = os.path.expanduser("~/.config/nexa-files/config.json")

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#pathentry { background-image: none; background-color: @nexa_field; color: @nexa_text_strong; border-radius: 6px; padding: 6px 10px; border: 1px solid @nexa_border; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#toolbtn:disabled { color: @nexa_disabled; }
treeview { background-image: none; background-color: @nexa_bg; color: @nexa_text; }
treeview:selected { background-image: none; background-color: @nexa_selected; color: @nexa_selected_text; }
treeview header button { background-image: none; background-color: @nexa_surface; color: @nexa_muted; }
#statusbar { background-image: none; background-color: @nexa_surface; color: @nexa_muted; font-size: 9pt; }
"""

COL_ICON, COL_NAME, COL_SIZE, COL_MTIME, COL_PATH, COL_IS_DIR = range(6)


class FileManagerWindow(Gtk.Window):
    def __init__(self, start_dir=None):
        super().__init__(title="Files")
        self.set_icon_name("nexa-files")
        attach_titlebar(self, "Files")
        self.set_default_size(820, 560)
        self.path = os.path.abspath(start_dir or os.environ.get("HOME", "/root"))
        self.show_hidden = False
        self.clipboard_path = None
        self.icon_size_index = self._load_icon_size_index()

        self._build_ui()
        self._refresh()

    def _load_icon_size_index(self):
        try:
            with open(CONFIG_PATH) as f:
                index = int(json.load(f).get("icon_size_index", DEFAULT_ICON_SIZE_INDEX))
        except (OSError, ValueError, TypeError):
            return DEFAULT_ICON_SIZE_INDEX
        return index if 0 <= index < len(ICON_SIZES) else DEFAULT_ICON_SIZE_INDEX

    def _save_icon_size_index(self):
        try:
            os.makedirs(os.path.dirname(CONFIG_PATH), exist_ok=True)
            with open(CONFIG_PATH, "w") as f:
                json.dump({"icon_size_index": self.icon_size_index}, f)
        except OSError:
            pass

    def _icon_pixbuf(self, icon_name):
        try:
            return Gtk.IconTheme.get_default().load_icon(
                icon_name, ICON_SIZES[self.icon_size_index], 0,
            )
        except GLib.Error:
            return None

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)

        self.up_btn = self._toolbtn("↑ Up", self._on_up)
        self.home_btn = self._toolbtn("Home", self._on_home)
        toolbar.pack_start(self.up_btn, False, False, 0)
        toolbar.pack_start(self.home_btn, False, False, 0)
        self.devices_btn = self._toolbtn("Devices", self._on_devices)
        toolbar.pack_start(self.devices_btn, False, False, 0)

        self.path_entry = Gtk.Entry()
        self.path_entry.set_name("pathentry")
        self.path_entry.connect("activate", self._on_path_entered)
        toolbar.pack_start(self.path_entry, True, True, 0)

        self.hidden_btn = self._toolbtn("Hidden: off", self._on_toggle_hidden)
        toolbar.pack_start(self.hidden_btn, False, False, 0)

        self.zoom_out_btn = self._toolbtn("−", self._on_zoom_out)
        self.zoom_out_btn.set_tooltip_text("Smaller icons")
        self.zoom_in_btn = self._toolbtn("+", self._on_zoom_in)
        self.zoom_in_btn.set_tooltip_text("Larger icons")
        toolbar.pack_start(self.zoom_out_btn, False, False, 0)
        toolbar.pack_start(self.zoom_in_btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        actions.set_name("toolbar")
        actions.set_border_width(8)
        self.new_folder_btn = self._toolbtn("New Folder", self._on_new_folder)
        self.rename_btn = self._toolbtn("Rename", self._on_rename)
        self.delete_btn = self._toolbtn("Delete", self._on_delete)
        self.copy_btn = self._toolbtn("Copy", self._on_copy)
        self.paste_btn = self._toolbtn("Paste", self._on_paste)
        self.open_btn = self._toolbtn("Open", self._on_open_selected)
        for b in (self.new_folder_btn, self.rename_btn, self.delete_btn,
                  self.copy_btn, self.paste_btn, self.open_btn):
            actions.pack_start(b, False, False, 0)
        root.pack_start(actions, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.store = Gtk.ListStore(GdkPixbuf.Pixbuf, str, str, str, str, bool)
        self.view = Gtk.TreeView(model=self.store)
        self.view.set_headers_visible(True)
        self.view.connect("row-activated", self._on_row_activated)
        self.view.get_selection().connect("changed", self._on_selection_changed)

        icon_renderer = Gtk.CellRendererPixbuf()
        name_renderer = Gtk.CellRendererText()
        name_col = Gtk.TreeViewColumn("Name")
        name_col.pack_start(icon_renderer, False)
        name_col.add_attribute(icon_renderer, "pixbuf", COL_ICON)
        name_col.pack_start(name_renderer, True)
        name_col.add_attribute(name_renderer, "text", COL_NAME)
        name_col.set_expand(True)
        name_col.set_resizable(True)
        self.view.append_column(name_col)

        size_renderer = Gtk.CellRendererText()
        size_renderer.set_property("xalign", 1.0)
        size_col = Gtk.TreeViewColumn("Size", size_renderer, text=COL_SIZE)
        size_col.set_fixed_width(90)
        self.view.append_column(size_col)

        mtime_col = Gtk.TreeViewColumn("Modified", Gtk.CellRendererText(), text=COL_MTIME)
        mtime_col.set_fixed_width(150)
        self.view.append_column(mtime_col)

        scroller.add(self.view)
        root.pack_start(scroller, True, True, 0)

        self.status = Gtk.Label(label="")
        self.status.set_name("statusbar")
        self.status.set_xalign(0)
        self.status.set_margin_start(10)
        self.status.set_margin_top(4)
        self.status.set_margin_bottom(4)
        root.pack_start(self.status, False, False, 0)

        gesture = Gtk.GestureMultiPress.new(self.view)
        gesture.set_button(3)
        gesture.connect("pressed", self._on_right_click)
        self._right_click_gesture = gesture

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

        self._update_action_sensitivity()

    def _toolbtn(self, label, handler):
        btn = Gtk.Button(label=label)
        btn.set_name("toolbtn")
        btn.connect("clicked", handler)
        return btn

    def _refresh(self):
        self.path_entry.set_text(self.path)
        self.store.clear()
        try:
            entries = fileops.list_dir(self.path, self.show_hidden)
        except fileops.FileOpError as e:
            self._error(f"Can't open {self.path}: {e}")
            return
        dirs = 0
        files = 0
        for e in entries:
            icon_name = fileops.guess_icon(e["name"], e["is_dir"], e["is_exec"])
            self.store.append([
                self._icon_pixbuf(icon_name), e["name"], fileops.human_size(e["size"], e["is_dir"]),
                fileops.human_time(e["mtime"]), e["path"], e["is_dir"],
            ])
            dirs += e["is_dir"]
            files += not e["is_dir"]
        self.status.set_text(f"{dirs} folder{'s' if dirs != 1 else ''}, {files} file{'s' if files != 1 else ''}")
        self._update_action_sensitivity()

    def _navigate(self, path):
        if not os.path.isdir(path):
            self._error(f"Not a directory: {path}")
            return
        self.path = os.path.abspath(path)
        self._refresh()

    def _selected_entry(self):
        model, it = self.view.get_selection().get_selected()
        if it is None:
            return None
        return {
            "path": model[it][COL_PATH], "name": model[it][COL_NAME],
            "is_dir": model[it][COL_IS_DIR],
        }

    def _update_action_sensitivity(self):
        sel = self._selected_entry()
        for b in (self.rename_btn, self.delete_btn, self.copy_btn, self.open_btn):
            b.set_sensitive(sel is not None)
        self.paste_btn.set_sensitive(self.clipboard_path is not None)
        self.up_btn.set_sensitive(self.path != "/")
        self.zoom_out_btn.set_sensitive(self.icon_size_index > 0)
        self.zoom_in_btn.set_sensitive(self.icon_size_index < len(ICON_SIZES) - 1)

    def _on_selection_changed(self, _sel):
        self._update_action_sensitivity()

    def _on_zoom_out(self, _btn):
        if self.icon_size_index > 0:
            self.icon_size_index -= 1
            self._save_icon_size_index()
            self._refresh()

    def _on_zoom_in(self, _btn):
        if self.icon_size_index < len(ICON_SIZES) - 1:
            self.icon_size_index += 1
            self._save_icon_size_index()
            self._refresh()

    def _on_row_activated(self, _view, tree_path, _column):
        it = self.store.get_iter(tree_path)
        path, is_dir = self.store[it][COL_PATH], self.store[it][COL_IS_DIR]
        if is_dir:
            self._navigate(path)
        else:
            self._open_path(path)

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

    def _on_up(self, _btn):
        self._navigate(os.path.dirname(self.path.rstrip("/")) or "/")

    def _on_home(self, _btn):
        self._navigate(os.environ.get("HOME", "/root"))

    def _on_path_entered(self, entry):
        self._navigate(entry.get_text().strip() or "/")

    def _on_toggle_hidden(self, _btn):
        self.show_hidden = not self.show_hidden
        self.hidden_btn.set_label(f"Hidden: {'on' if self.show_hidden else 'off'}")
        self._refresh()

    def _on_open_selected(self, _btn):
        sel = self._selected_entry()
        if not sel:
            return
        if sel["is_dir"]:
            self._navigate(sel["path"])
        else:
            self._open_path(sel["path"])

    def _on_new_folder(self, _btn):
        name = self._prompt("New Folder", "Folder name:", "New Folder")
        if not name:
            return
        try:
            fileops.make_dir(self.path, name)
        except fileops.FileOpError as e:
            self._error(str(e))
        self._refresh()

    def _on_rename(self, _btn):
        sel = self._selected_entry()
        if not sel:
            return
        new_name = self._prompt("Rename", "New name:", sel["name"])
        if not new_name or new_name == sel["name"]:
            return
        try:
            fileops.rename(sel["path"], new_name)
        except fileops.FileOpError as e:
            self._error(str(e))
        self._refresh()

    def _on_delete(self, _btn):
        sel = self._selected_entry()
        if not sel:
            return
        if not self._confirm(f"Delete '{sel['name']}'?", "This cannot be undone."):
            return
        try:
            fileops.delete(sel["path"])
        except fileops.FileOpError as e:
            self._error(str(e))
        self._refresh()

    def _on_copy(self, _btn):
        sel = self._selected_entry()
        if sel:
            self.clipboard_path = sel["path"]
            self.status.set_text(f"Copied: {sel['name']} (select a folder and Paste, or Paste here)")
            self._update_action_sensitivity()

    def _on_paste(self, _btn):
        if not self.clipboard_path:
            return
        sel = self._selected_entry()
        dest_dir = sel["path"] if (sel and sel["is_dir"]) else self.path
        try:
            fileops.copy_into(self.clipboard_path, dest_dir)
        except fileops.FileOpError as e:
            self._error(str(e))
        self._refresh()

    def _on_right_click(self, _gesture, _n_press, x, y):
        path_info = self.view.get_path_at_pos(int(x), int(y))
        if path_info:
            self.view.set_cursor(path_info[0])
        menu = Gtk.Menu()
        for label, handler in (
            ("Open", self._on_open_selected), ("Open With…", self._on_open_with),
            ("Rename", self._on_rename),
            ("Copy", self._on_copy), ("Paste", self._on_paste),
            ("Delete", self._on_delete), ("New Folder", self._on_new_folder),
        ):
            item = Gtk.MenuItem(label=label)
            item.connect("activate", handler)
            menu.append(item)
        menu.show_all()
        menu.popup_at_pointer(None)

    def _prompt(self, title, message, default_text):
        dialog = Gtk.Dialog(title=title, transient_for=self, modal=True)
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "OK", Gtk.ResponseType.OK)
        box = dialog.get_content_area()
        box.set_border_width(10)
        box.add(Gtk.Label(label=message))
        entry = Gtk.Entry()
        entry.set_text(default_text)
        entry.set_activates_default(True)
        dialog.set_default_response(Gtk.ResponseType.OK)
        box.add(entry)
        dialog.show_all()
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

    def _error(self, text):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
            buttons=Gtk.ButtonsType.OK, text=text,
        )
        dialog.run()
        dialog.destroy()


def main():
    Gtk.init()
    win = FileManagerWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
