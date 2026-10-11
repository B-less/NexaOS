import os
import sys

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GdkPixbuf, GLib

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#toolbar { background-image: none; background-color: #252525; }
#toolbtn { background-image: none; background-color: #2f2f2f; color: #d4d4d4; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: #3a3a3a; }
#toolbtn:disabled { color: #5a5a5a; }
#filelabel { color: #9a9a9a; font-size: 9pt; }
#viewport { background-image: none; background-color: #101010; }
"""

IMAGE_EXTS = {".png", ".jpg", ".jpeg", ".gif", ".bmp", ".svg", ".webp"}


class ViewerWindow(Gtk.Window):
    def __init__(self, path=None):
        super().__init__(title="Nexa Viewer")
        self.set_icon_name("nexa-viewer")
        attach_titlebar(self, "Nexa Viewer")
        self.set_default_size(900, 650)

        self.path = None
        self.siblings = []
        self.index = -1
        self.pixbuf = None
        self.zoom = 1.0
        self.fit_mode = True

        self._build_ui()
        if path:
            self._load_siblings(os.path.abspath(path))
            self._show(os.path.abspath(path))

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
        fit_btn = self._toolbtn("Fit", self._on_fit)
        actual_btn = self._toolbtn("100%", self._on_actual)
        for b in (self.prev_btn, self.next_btn, zoom_out_btn, zoom_in_btn, fit_btn, actual_btn):
            toolbar.pack_start(b, False, False, 0)

        self.name_label = Gtk.Label(label="")
        self.name_label.set_name("filelabel")
        self.name_label.set_hexpand(True)
        self.name_label.set_xalign(1.0)
        toolbar.pack_start(self.name_label, True, True, 0)

        root.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        scroller.set_name("viewport")
        self.image = Gtk.Image()
        eventbox = Gtk.EventBox()
        eventbox.set_name("viewport")
        eventbox.add(self.image)
        scroller.add(eventbox)
        scroller.connect("size-allocate", self._on_viewport_resize)
        self.scroller = scroller
        root.pack_start(scroller, True, True, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )
        self._update_nav_sensitivity()

    def _toolbtn(self, label, handler):
        btn = Gtk.Button(label=label)
        btn.set_name("toolbtn")
        btn.connect("clicked", handler)
        return btn

    def _load_siblings(self, path):
        folder = os.path.dirname(path)
        try:
            names = sorted(
                n for n in os.listdir(folder)
                if os.path.splitext(n)[1].lower() in IMAGE_EXTS
            )
        except OSError:
            names = [os.path.basename(path)]
        self.siblings = [os.path.join(folder, n) for n in names] or [path]
        try:
            self.index = self.siblings.index(path)
        except ValueError:
            self.siblings = [path]
            self.index = 0

    def _show(self, path):
        try:
            self.pixbuf = GdkPixbuf.Pixbuf.new_from_file(path)
        except GLib.Error as e:
            self.pixbuf = None
            self.image.clear()
            self.name_label.set_text(f"Can't load {os.path.basename(path)}: {e.message}")
            return
        self.path = path
        self.fit_mode = True
        self.name_label.set_text(os.path.basename(path))
        self.set_title(os.path.basename(path))
        self._render()
        self._update_nav_sensitivity()

    def _update_nav_sensitivity(self):
        multi = len(self.siblings) > 1
        self.prev_btn.set_sensitive(multi)
        self.next_btn.set_sensitive(multi)

    def _render(self):
        if self.pixbuf is None:
            return
        if self.fit_mode:
            avail_w = max(self.scroller.get_allocated_width() - 4, 1)
            avail_h = max(self.scroller.get_allocated_height() - 4, 1)
            scale = min(
                avail_w / self.pixbuf.get_width(),
                avail_h / self.pixbuf.get_height(),
                1.0,
            )
        else:
            scale = self.zoom
        w = max(int(self.pixbuf.get_width() * scale), 1)
        h = max(int(self.pixbuf.get_height() * scale), 1)
        scaled = self.pixbuf.scale_simple(w, h, GdkPixbuf.InterpType.BILINEAR)
        self.image.set_from_pixbuf(scaled)

    def _on_viewport_resize(self, *_a):
        if self.fit_mode:
            self._render()

    def _on_prev(self, _btn):
        if not self.siblings:
            return
        self.index = (self.index - 1) % len(self.siblings)
        self._show(self.siblings[self.index])

    def _on_next(self, _btn):
        if not self.siblings:
            return
        self.index = (self.index + 1) % len(self.siblings)
        self._show(self.siblings[self.index])

    def _on_zoom_in(self, _btn):
        self.fit_mode = False
        self.zoom = min(self.zoom * 1.25, 8.0)
        self._render()

    def _on_zoom_out(self, _btn):
        self.fit_mode = False
        self.zoom = max(self.zoom / 1.25, 0.05)
        self._render()

    def _on_fit(self, _btn):
        self.fit_mode = True
        self._render()

    def _on_actual(self, _btn):
        self.fit_mode = False
        self.zoom = 1.0
        self._render()


def main():
    Gtk.init()
    path = sys.argv[1] if len(sys.argv) > 1 else None
    win = ViewerWindow(path)
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
