"""Launchpad: every app in a full-screen grid over the (blurred) desktop.

Super+Space or the Dock's Launchpad icon opens it; typing searches at once,
Enter opens the first match, Escape or a click on empty space closes it.
"""
import time

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gdk, GLib, Gtk, GtkLayerShell, Pango

from nexa_common import desktopapps, themes
from .panel import run_panel

COLUMNS = 7
ICON_PX = 72
ANIM_MS = 220
SELF_NAME = "Launchpad"  # its own desktop entry, left out of the grid


def launchpad_css(t, opacity):
    backdrop = themes._rgba(t["bg"], max(0.55, min(opacity, 0.75)))
    hover = "rgba(255, 255, 255, 0.10)" if t["dark"] else "rgba(0, 0, 0, 0.07)"
    accent = themes._rgb_of(t["accent"])
    return f"""
window {{ background-color: {backdrop}; background-image: none; }}
#lp-search {{
    background-color: {hover}; background-image: none; color: {t['text']};
    caret-color: {t['accent']}; border: 1px solid {themes._rgba(accent, 0.45)};
    border-radius: 14px; padding: 8px 14px; font-size: 12pt; box-shadow: none;
}}
#lp-app {{
    background-color: transparent; background-image: none; border: none;
    box-shadow: none; border-radius: 18px; padding: 12px 6px;
    transition: background-color 140ms ease-out;
}}
#lp-app:hover, #lp-app:focus {{ background-color: {hover}; }}
#lp-name {{ color: {t['text']}; font-size: 10pt; }}
#lp-empty {{ color: {t['muted']}; font-size: 12pt; }}
scrolledwindow, viewport, flowbox, flowboxchild {{ background-color: transparent; background-image: none; }}
"""


class Launchpad(Gtk.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="Launchpad")
        visual = self.get_screen().get_rgba_visual()
        if visual:
            self.set_visual(visual)
        GtkLayerShell.init_for_window(self)
        GtkLayerShell.set_namespace(self, "nexa-launchpad")  # Hyprland blurs and fades nexa-* layers
        GtkLayerShell.set_layer(self, GtkLayerShell.Layer.OVERLAY)
        GtkLayerShell.set_keyboard_mode(self, GtkLayerShell.KeyboardMode.EXCLUSIVE)
        GtkLayerShell.set_exclusive_zone(self, -1)  # cover the bar too
        for edge in (GtkLayerShell.Edge.TOP, GtkLayerShell.Edge.BOTTOM,
                     GtkLayerShell.Edge.LEFT, GtkLayerShell.Edge.RIGHT):
            GtkLayerShell.set_anchor(self, edge, True)

        settings = themes.load_settings()
        css = Gtk.CssProvider()
        css.load_from_data(launchpad_css(themes.THEMES[settings["theme"]], settings["opacity"]).encode())
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), css,
                                                 Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

        self.apps = [a for a in desktopapps.load_apps() if a["name"] != SELF_NAME]

        backdrop = Gtk.EventBox()
        backdrop.set_visible_window(False)
        backdrop.connect("button-press-event", self._on_backdrop_click)
        self.add(backdrop)

        self.body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=28)
        self.body.set_margin_top(60)
        self.body.set_margin_bottom(40)
        self.body.set_halign(Gtk.Align.CENTER)
        backdrop.add(self.body)

        self.search = Gtk.SearchEntry(placeholder_text="Search")
        self.search.set_name("lp-search")
        self.search.set_halign(Gtk.Align.CENTER)
        self.search.set_size_request(360, -1)
        self.search.connect("search-changed", lambda *_a: self._fill())
        self.search.connect("activate", self._launch_first)
        self.body.pack_start(self.search, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.grid = Gtk.FlowBox()
        self.grid.set_selection_mode(Gtk.SelectionMode.NONE)
        self.grid.set_max_children_per_line(COLUMNS)
        self.grid.set_min_children_per_line(COLUMNS)
        self.grid.set_homogeneous(True)
        self.grid.set_column_spacing(18)
        self.grid.set_row_spacing(18)
        self.grid.set_valign(Gtk.Align.START)
        scroller.add(self.grid)
        self.body.pack_start(scroller, True, True, 0)

        self.connect("key-press-event", self._on_key)
        self._closing = False
        self.matches = []
        self._fill()
        self.body.set_opacity(0)
        self.connect("map", lambda *_a: self._animate(True))
        GLib.idle_add(self.search.grab_focus)

    def _app_button(self, app):
        btn = Gtk.Button()
        btn.set_name("lp-app")
        btn.set_tooltip_text(app["comment"] or app["name"])
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        image = Gtk.Image.new_from_icon_name(app["icon"], Gtk.IconSize.DIALOG)
        image.set_pixel_size(ICON_PX)
        box.pack_start(image, False, False, 0)
        name = Gtk.Label(label=app["name"])
        name.set_name("lp-name")
        name.set_ellipsize(Pango.EllipsizeMode.END)
        name.set_max_width_chars(12)
        box.pack_start(name, False, False, 0)
        btn.add(box)
        btn.set_size_request(120, -1)
        btn.connect("clicked", lambda *_a: self._launch(app))
        return btn

    def _fill(self):
        for child in self.grid.get_children():
            child.destroy()
        query = self.search.get_text().strip()
        self.matches = desktopapps.search(self.apps, query) if query else self.apps
        for app in self.matches:
            self.grid.add(self._app_button(app))
        if not self.matches:
            empty = Gtk.Label(label=f'No apps match "{query}"')
            empty.set_name("lp-empty")
            self.grid.add(empty)
        self.grid.show_all()

    def _launch_first(self, *_a):
        if self.matches:
            self._launch(self.matches[0])

    def _launch(self, app):
        desktopapps.launch(app["exec"])
        self.close()

    def _on_backdrop_click(self, _w, event):
        # Clicks on the grid's buttons never get here; anything else closes.
        self.close()
        return True

    def _on_key(self, _w, event):
        if event.keyval == Gdk.KEY_Escape:
            if self.search.get_text():
                self.search.set_text("")
            else:
                self.close()
            return True
        if not self.search.has_focus() and event.string and event.string.isprintable():
            self.search.grab_focus_without_selecting()
            return self.search.event(event)
        return False

    def _animate(self, opening, done=None):
        """Fade in while the grid settles from slightly zoomed out, like
        macOS; reversed on close."""
        start = time.monotonic()

        def step():
            t = min(1.0, (time.monotonic() - start) * 1000 / ANIM_MS)
            eased = 1 - (1 - t) ** 3
            shown = eased if opening else 1 - eased
            self.body.set_opacity(shown)
            inset = int(round(30 * (1 - shown)))
            self.body.set_margin_top(60 + inset)
            self.body.set_margin_start(inset)
            self.body.set_margin_end(inset)
            if t >= 1.0:
                if done:
                    done()
                return False
            return True
        GLib.timeout_add(12, step)

    def close(self):
        if self._closing:
            return
        self._closing = True
        self._animate(False, done=lambda: Gtk.ApplicationWindow.close(self))


def main():
    return run_panel("os.nexa.Launchpad", Launchpad)
