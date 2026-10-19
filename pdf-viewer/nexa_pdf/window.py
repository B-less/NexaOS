import os
import sys

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Poppler", "0.18")
from gi.repository import Gtk, Gdk, GLib, Poppler

from nexa_common.titlebar import attach_titlebar

CSS = b"""
#emptyhint { color: @nexa_muted; font-size: 11pt; }
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#toolbtn:disabled { color: @nexa_disabled; }
#pagelabel { color: @nexa_muted; font-size: 9pt; }
#viewport { background-image: none; background-color: @nexa_bg_deep; }
"""


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
        open_btn = self._toolbtn("Open…", self._on_open)
        open_btn.set_tooltip_text("Open a PDF (Ctrl+O)")
        toolbar.pack_start(open_btn, False, False, 0)
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
        self.pages = Gtk.Stack()
        self.pages.add_named(_empty_state("Open a PDF to read it.\nCtrl+O, or open one from Files.", self._on_open), "empty")
        self.pages.add_named(scroller, "doc")
        self.pages.show_all()  # so a file opened at start can switch to its page
        root.pack_start(self.pages, True, True, 0)
        self.connect("key-press-event", self._on_key)

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
        self.pages.set_visible_child_name("doc")
        self._render()

    def _on_open(self, _btn):
        path = _choose_file(self, "PDF documents", ["*.pdf", "*.PDF"])
        if path:
            self._load(path)

    def _on_key(self, _w, event):
        if event.state & Gdk.ModifierType.CONTROL_MASK and Gdk.keyval_to_lower(event.keyval) == Gdk.KEY_o:
            self._on_open(None)
            return True
        return False

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
