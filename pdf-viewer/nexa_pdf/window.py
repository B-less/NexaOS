import os
import sys

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Poppler", "0.18")
from gi.repository import Gtk, Gdk, GLib, Poppler

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#toolbar { background-image: none; background-color: #252525; }
#toolbtn { background-image: none; background-color: #2f2f2f; color: #d4d4d4; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: #3a3a3a; }
#toolbtn:disabled { color: #5a5a5a; }
#pagelabel { color: #9a9a9a; font-size: 9pt; }
#viewport { background-image: none; background-color: #101010; }
"""


class PdfWindow(Gtk.Window):
    def __init__(self, path=None):
        super().__init__(title="Nexa PDF")
        self.set_icon_name("nexa-pdf")
        attach_titlebar(self, "Nexa PDF")
        self.set_default_size(800, 900)

        self.doc = None
        self.page_index = 0
        self.zoom = 1.0

        self._build_ui()
        if path:
            self._load(path)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        self.prev_btn = self._toolbtn("◀", self._on_prev)
        self.next_btn = self._toolbtn("▶", self._on_next)
        zoom_out_btn = self._toolbtn("−", self._on_zoom_out)
        zoom_in_btn = self._toolbtn("+", self._on_zoom_in)
        for b in (self.prev_btn, self.next_btn, zoom_out_btn, zoom_in_btn):
            toolbar.pack_start(b, False, False, 0)
        self.page_label = Gtk.Label(label="")
        self.page_label.set_name("pagelabel")
        self.page_label.set_hexpand(True)
        self.page_label.set_xalign(1.0)
        toolbar.pack_start(self.page_label, True, True, 0)
        root.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        scroller.set_name("viewport")
        self.drawing = Gtk.DrawingArea()
        self.drawing.connect("draw", self._on_draw)
        scroller.add(self.drawing)
        root.pack_start(scroller, True, True, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )
        self._update_sensitivity()

    def _toolbtn(self, label, handler):
        btn = Gtk.Button(label=label)
        btn.set_name("toolbtn")
        btn.connect("clicked", handler)
        return btn

    def _load(self, path):
        uri = GLib.filename_to_uri(os.path.abspath(path), None)
        try:
            self.doc = Poppler.Document.new_from_file(uri, None)
        except GLib.Error as e:
            self._error(f"Can't open {os.path.basename(path)}: {e.message}")
            return
        self.page_index = 0
        self.set_title(os.path.basename(path))
        self._render()

    def _current_page(self):
        if not self.doc:
            return None
        return self.doc.get_page(self.page_index)

    def _render(self):
        page = self._current_page()
        if page is None:
            return
        w, h = page.get_size()
        self.drawing.set_size_request(int(w * self.zoom), int(h * self.zoom))
        n = self.doc.get_n_pages()
        self.page_label.set_text(f"Page {self.page_index + 1} of {n}")
        self._update_sensitivity()
        self.drawing.queue_draw()

    def _on_draw(self, widget, cr):
        page = self._current_page()
        if page is None:
            return False
        cr.set_source_rgb(1, 1, 1)
        w = widget.get_allocated_width()
        h = widget.get_allocated_height()
        cr.rectangle(0, 0, w, h)
        cr.fill()
        cr.scale(self.zoom, self.zoom)
        page.render(cr)
        return False

    def _update_sensitivity(self):
        n = self.doc.get_n_pages() if self.doc else 0
        self.prev_btn.set_sensitive(self.page_index > 0)
        self.next_btn.set_sensitive(self.page_index < n - 1)

    def _on_prev(self, _btn):
        if self.page_index > 0:
            self.page_index -= 1
            self._render()

    def _on_next(self, _btn):
        if self.doc and self.page_index < self.doc.get_n_pages() - 1:
            self.page_index += 1
            self._render()

    def _on_zoom_in(self, _btn):
        self.zoom = min(self.zoom * 1.2, 4.0)
        self._render()

    def _on_zoom_out(self, _btn):
        self.zoom = max(self.zoom / 1.2, 0.2)
        self._render()

    def _error(self, text):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
            buttons=Gtk.ButtonsType.OK, text=text,
        )
        dialog.run()
        dialog.destroy()


def main():
    Gtk.init()
    path = sys.argv[1] if len(sys.argv) > 1 else None
    win = PdfWindow(path)
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
