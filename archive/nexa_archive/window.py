import os
import sys

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk

from . import archiveops
from nexa_common.titlebar import attach_titlebar

CSS = b"""
#emptyhint { color: @nexa_muted; font-size: 11pt; }
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#toolbtn:disabled { color: @nexa_disabled; }
treeview { background-image: none; background-color: @nexa_bg; color: @nexa_text; }
treeview:selected { background-image: none; background-color: @nexa_selected; color: @nexa_selected_text; }
treeview header button { background-image: none; background-color: @nexa_surface; color: @nexa_muted; }
#statusbar { background-image: none; background-color: @nexa_surface; color: @nexa_muted; font-size: 9pt; }
"""

COL_ICON, COL_NAME, COL_SIZE = range(3)


def _empty_state(text, on_open):
    """What an empty window shows: what it's for, and a way to start."""
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
    box.set_valign(Gtk.Align.CENTER)
    box.set_halign(Gtk.Align.CENTER)
    label = Gtk.Label(label=text)
    label.set_name("emptyhint")
    label.set_justify(Gtk.Justification.CENTER)
    box.pack_start(label, False, False, 0)
    btn = Gtk.Button(label="Open…")
    btn.set_name("toolbtn")
    btn.set_halign(Gtk.Align.CENTER)
    btn.connect("clicked", lambda _b: on_open(None))
    box.pack_start(btn, False, False, 0)
    return box


def _choose_file(parent, title, patterns):
    dialog = Gtk.FileChooserDialog(title=title, transient_for=parent, action=Gtk.FileChooserAction.OPEN)
    dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Open", Gtk.ResponseType.OK)
    filt = Gtk.FileFilter()
    filt.set_name(title)
    for p in patterns:
        filt.add_pattern(p)
    dialog.add_filter(filt)
    path = dialog.get_filename() if dialog.run() == Gtk.ResponseType.OK else None
    dialog.destroy()
    return path


class ArchiveWindow(Gtk.Window):
    def __init__(self, path=None):
        super().__init__(title="Nexa Archive")
        self.set_icon_name("nexa-archive")
        attach_titlebar(self, "Nexa Archive")
        self.set_default_size(700, 520)

        self.path = os.path.abspath(path) if path else None
        self._build_ui()
        if self.path:
            self._load()

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        open_btn = self._toolbtn("Open…", self._on_open)
        open_btn.set_tooltip_text("Open an archive (Ctrl+O)")
        toolbar.pack_start(open_btn, False, False, 0)
        self.extract_btn = self._toolbtn("Extract All…", self._on_extract_all)
        self.extract_btn.set_sensitive(False)
        toolbar.pack_start(self.extract_btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.store = Gtk.ListStore(str, str, str)
        self.view = Gtk.TreeView(model=self.store)
        self.view.set_headers_visible(True)

        icon_renderer = Gtk.CellRendererPixbuf()
        name_renderer = Gtk.CellRendererText()
        name_col = Gtk.TreeViewColumn("Name")
        name_col.pack_start(icon_renderer, False)
        name_col.add_attribute(icon_renderer, "icon-name", COL_ICON)
        name_col.pack_start(name_renderer, True)
        name_col.add_attribute(name_renderer, "text", COL_NAME)
        name_col.set_expand(True)
        name_col.set_resizable(True)
        self.view.append_column(name_col)

        size_renderer = Gtk.CellRendererText()
        size_renderer.set_property("xalign", 1.0)
        size_col = Gtk.TreeViewColumn("Size", size_renderer, text=COL_SIZE)
        size_col.set_fixed_width(100)
        self.view.append_column(size_col)

        scroller.add(self.view)
        self.pages = Gtk.Stack()
        self.pages.add_named(_empty_state("Open a zip or tar archive to see what's inside.\nCtrl+O, or open one from Files.",
                                          self._on_open), "empty")
        self.pages.add_named(scroller, "list")
        self.pages.show_all()  # so a file opened at start can switch to its page
        root.pack_start(self.pages, True, True, 0)
        self.connect("key-press-event", self._on_key)

        self.status = Gtk.Label(label="")
        self.status.set_name("statusbar")
        self.status.set_xalign(0)
        self.status.set_margin_start(10)
        self.status.set_margin_top(4)
        self.status.set_margin_bottom(4)
        root.pack_start(self.status, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _toolbtn(self, label, handler):
        btn = Gtk.Button(label=label)
        btn.set_name("toolbtn")
        btn.connect("clicked", handler)
        return btn

    def _load(self):
        self.set_title(os.path.basename(self.path))
        self.store.clear()
        try:
            entries = archiveops.list_contents(self.path)
        except archiveops.ArchiveError as e:
            self._error(str(e))
            return
        for e in entries:
            icon = "folder" if e["is_dir"] else "text-x-generic"
            size = "" if e["is_dir"] else _human_size(e["size"])
            self.store.append([icon, e["name"], size])
        self.status.set_text(f"{len(entries)} entries")
        self.extract_btn.set_sensitive(True)
        self.pages.set_visible_child_name("list")

    def _on_extract_all(self, _btn):
        if not self.path:
            return
        default_dir = os.path.dirname(self.path) or "."
        dest = self._choose_dest(default_dir)
        if not dest:
            return
        try:
            archiveops.extract_all(self.path, dest)
        except archiveops.ArchiveError as e:
            self._error(str(e))
            return
        self.status.set_text(f"Extracted to {dest}")

    def _choose_dest(self, default_dir):
        dialog = Gtk.FileChooserDialog(
            title="Extract to…", transient_for=self,
            action=Gtk.FileChooserAction.SELECT_FOLDER,
        )
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Select", Gtk.ResponseType.OK)
        dialog.set_current_folder(default_dir)
        response = dialog.run()
        dest = dialog.get_filename() if response == Gtk.ResponseType.OK else None
        dialog.destroy()
        return dest

    def _on_open(self, _btn):
        path = _choose_file(self, "Archives", ["*.zip", "*.tar", "*.tar.gz", "*.tgz", "*.tar.xz", "*.txz", "*.tar.bz2", "*.tbz2"])
        if path:
            self.path = os.path.abspath(path)
            self._load()

    def _on_key(self, _w, event):
        if event.state & Gdk.ModifierType.CONTROL_MASK and Gdk.keyval_to_lower(event.keyval) == Gdk.KEY_o:
            self._on_open(None)
            return True
        return False

    def _error(self, text):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
            buttons=Gtk.ButtonsType.OK, text=text,
        )
        dialog.run()
        dialog.destroy()


def _human_size(n):
    size = float(n)
    for unit in ("B", "KB", "MB", "GB"):
        if size < 1024:
            return f"{size:.0f}{unit}" if unit == "B" else f"{size:.1f}{unit}"
        size /= 1024
    return f"{size:.1f}TB"


def main():
    Gtk.init()
    path = sys.argv[1] if len(sys.argv) > 1 else None
    win = ArchiveWindow(path)
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
