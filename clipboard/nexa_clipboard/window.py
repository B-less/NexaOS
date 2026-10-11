import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#toolbar { background-image: none; background-color: #252525; }
#toolbtn { background-image: none; background-color: #2f2f2f; color: #d4d4d4; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: #3a3a3a; }
treeview { background-image: none; background-color: #1e1e1e; color: #d4d4d4; }
treeview:selected { background-image: none; background-color: #2c3a52; color: #dbe4f5; }
#statusbar { background-image: none; background-color: #202020; color: #8a8a8a; font-size: 9pt; }
"""

MAX_ENTRIES = 50


class ClipboardWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa Clipboard")
        self.set_icon_name("nexa-clipboard")
        attach_titlebar(self, "Nexa Clipboard")
        self.set_default_size(480, 520)

        self._last_seen = None
        self._suppress_next = None
        self._build_ui()
        GLib.timeout_add(800, self._on_poll)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        self.copy_btn = self._toolbtn("Copy", self._on_copy_selected)
        self.copy_btn.set_sensitive(False)
        clear_btn = self._toolbtn("Clear History", self._on_clear)
        toolbar.pack_start(self.copy_btn, False, False, 0)
        toolbar.pack_start(clear_btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.store = Gtk.ListStore(str)
        self.view = Gtk.TreeView(model=self.store)
        self.view.set_headers_visible(False)
        self.view.connect("row-activated", self._on_row_activated)
        self.view.get_selection().connect("changed", self._on_selection_changed)
        renderer = Gtk.CellRendererText()
        renderer.set_property("ellipsize", 3)
        col = Gtk.TreeViewColumn("Text", renderer, text=0)
        col.set_expand(True)
        self.view.append_column(col)
        scroller.add(self.view)
        root.pack_start(scroller, True, True, 0)

        self.status = Gtk.Label(label="Watching clipboard…")
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

    def _on_poll(self):
        clipboard = Gtk.Clipboard.get_default(Gdk.Display.get_default())
        clipboard.request_text(self._on_clipboard_text)
        return True

    def _on_clipboard_text(self, _clipboard, text):
        if not text:
            return
        if text == self._last_seen or text == self._suppress_next:
            return
        self._last_seen = text
        # Drop an existing identical entry so re-copying moves it to the top
        # instead of leaving a duplicate lower in the list.
        for row in self.store:
            if row[0] == text:
                self.store.remove(row.iter)
                break
        self.store.insert(0, [text])
        while len(self.store) > MAX_ENTRIES:
            last_iter = self.store.iter_nth_child(None, len(self.store) - 1)
            self.store.remove(last_iter)
        self.status.set_text(f"{len(self.store)} entries")

    def _selected_text(self):
        model, it = self.view.get_selection().get_selected()
        return model[it][0] if it else None

    def _on_selection_changed(self, _sel):
        self.copy_btn.set_sensitive(self._selected_text() is not None)

    def _copy(self, text):
        clipboard = Gtk.Clipboard.get_default(Gdk.Display.get_default())
        self._suppress_next = text
        self._last_seen = text
        clipboard.set_text(text, -1)
        clipboard.store()

    def _on_row_activated(self, _view, tree_path, _column):
        self._copy(self.store[tree_path][0])

    def _on_copy_selected(self, _btn):
        text = self._selected_text()
        if text:
            self._copy(text)

    def _on_clear(self, _btn):
        self.store.clear()
        self.status.set_text("0 entries")


def main():
    Gtk.init()
    win = ClipboardWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
