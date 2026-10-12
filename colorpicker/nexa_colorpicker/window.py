import os
import subprocess
import tempfile

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GdkPixbuf, GLib

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#swatch { border-radius: 8px; border: 2px solid @nexa_border; }
#hexlabel { color: @nexa_text_strong; font-size: 14pt; font-weight: bold; font-family: monospace; }
#statusbar { background-image: none; background-color: @nexa_surface; color: @nexa_muted; font-size: 9pt; }
"""


class ColorPickerWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa Color Picker")
        self.set_icon_name("nexa-colorpicker")
        attach_titlebar(self, "Nexa Color Picker")
        self.set_default_size(760, 600)

        self.pixbuf = None
        self.display_scale = 1.0
        self.current_rgb = (0, 0, 0)
        self._build_ui()

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        capture_btn = self._toolbtn("Capture Screen", self._on_capture)
        toolbar.pack_start(capture_btn, False, False, 0)

        self.swatch = Gtk.DrawingArea()
        self.swatch.set_name("swatch")
        self.swatch.set_size_request(32, 32)
        self.swatch.connect("draw", self._on_draw_swatch)
        toolbar.pack_start(self.swatch, False, False, 0)

        self.hex_label = Gtk.Label(label="#000000")
        self.hex_label.set_name("hexlabel")
        toolbar.pack_start(self.hex_label, False, False, 0)

        copy_btn = self._toolbtn("Copy Hex", self._on_copy)
        toolbar.pack_start(copy_btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.image = Gtk.Image()
        eventbox = Gtk.EventBox()
        eventbox.add(self.image)
        eventbox.connect("button-press-event", self._on_image_click)
        scroller.add(eventbox)
        root.pack_start(scroller, True, True, 0)

        self.status = Gtk.Label(label="Click Capture Screen, then click anywhere on the image to sample its color")
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

    def _on_capture(self, _btn):
        fd, path = tempfile.mkstemp(suffix=".png", prefix="nexa-colorpicker-")
        os.close(fd)
        try:
            subprocess.run(["grim", path], check=True, timeout=10)
            self.pixbuf = GdkPixbuf.Pixbuf.new_from_file(path)
        except (subprocess.SubprocessError, GLib.Error, OSError) as e:
            self.status.set_text(f"Capture failed: {e}")
            return
        finally:
            try:
                os.unlink(path)
            except OSError:
                pass

        avail_w = 900
        scale = min(avail_w / self.pixbuf.get_width(), 1.0)
        self.display_scale = scale
        w = int(self.pixbuf.get_width() * scale)
        h = int(self.pixbuf.get_height() * scale)
        scaled = self.pixbuf.scale_simple(w, h, GdkPixbuf.InterpType.BILINEAR)
        self.image.set_from_pixbuf(scaled)
        self.status.set_text("Click anywhere on the image to sample its color")

    def _on_image_click(self, _widget, event):
        if self.pixbuf is None:
            return
        x = int(event.x / self.display_scale)
        y = int(event.y / self.display_scale)
        x = max(0, min(x, self.pixbuf.get_width() - 1))
        y = max(0, min(y, self.pixbuf.get_height() - 1))
        pixels = self.pixbuf.get_pixels()
        stride = self.pixbuf.get_rowstride()
        channels = self.pixbuf.get_n_channels()
        offset = y * stride + x * channels
        r, g, b = pixels[offset], pixels[offset + 1], pixels[offset + 2]
        self.current_rgb = (r, g, b)
        self.hex_label.set_text(f"#{r:02x}{g:02x}{b:02x}".upper())
        self.swatch.queue_draw()
        self.status.set_text(f"RGB({r}, {g}, {b})")

    def _on_draw_swatch(self, widget, cr):
        r, g, b = self.current_rgb
        cr.set_source_rgb(r / 255, g / 255, b / 255)
        w = widget.get_allocated_width()
        h = widget.get_allocated_height()
        cr.rectangle(0, 0, w, h)
        cr.fill()
        return False

    def _on_copy(self, _btn):
        clipboard = Gtk.Clipboard.get_default(Gdk.Display.get_default())
        clipboard.set_text(self.hex_label.get_text(), -1)
        clipboard.store()
        self.status.set_text(f"Copied {self.hex_label.get_text()}")


def main():
    Gtk.init()
    win = ColorPickerWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
