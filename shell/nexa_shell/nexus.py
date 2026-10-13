"""Nexus: the NexaOS launcher (replaces the start menu and the app grid).

A full-screen layer over the dimmed, blurred desktop. Apps sit in a
honeycomb of hexagon tiles (rows of 7 and 6, so alternate rows interlock),
favourites first. Typing searches apps, Settings pages, does arithmetic, or
hands the question to the assistant (see search.py). Power actions sit at
the bottom. Super+Space, Super+D, the bar's NexaOS button and the Orbit
dock's first icon all open it; Escape or a click on empty space closes it.
"""
import os
import pwd
import socket
import time

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gdk, GLib, Gtk, GtkLayerShell, Pango

from nexa_common import design, desktopapps, themes
from . import search
from .panel import power, run_panel, spawn

ROW_LONG, ROW_SHORT = 7, 6
TILE_W, TILE_HEX_R, ICON_PX, TILE_GAP = 116, 44, 46, 6
ANIM_MS = design.DURATION_MS["panel"]
SELF_NAMES = {"Nexus", "Launchpad"}
# Shown first, in this order, when installed (desktop-file names).
FAVOURITES = ["Files", "Nexa Browser", "Foot", "Settings", "Nexa Software", "NexaOS Assistant",
              "Nexa Notes", "Music", "Editor", "Nexa Pulse", "Calculator", "Screenshot"]


def _rgb(hex_colour):
    h = hex_colour.lstrip("#")
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))


def nexus_css(t, opacity):
    pal = themes.app_palette(t)
    backdrop = themes._rgba(t["bg"], max(0.6, min(opacity, 0.8)))
    hover = "rgba(255, 255, 255, 0.08)" if t["dark"] else "rgba(0, 0, 0, 0.06)"
    return f"""
window {{ background-color: {backdrop}; background-image: none; }}
#nx-greeting {{ color: {pal['text_strong']}; font-size: {design.TYPE_SCALE['display']}pt; font-weight: 300; }}
#nx-date {{ color: {pal['muted']}; font-size: {design.TYPE_SCALE['title']}pt; }}
#nx-search {{
    background-color: {hover}; background-image: none; color: {pal['text_strong']};
    caret-color: {t['accent']}; border: 1px solid {themes._rgba(themes._rgb_of(t['accent']), 0.5)};
    border-radius: 22px; padding: 10px 18px; font-size: 13pt; box-shadow: none; min-width: 520px;
}}
#nx-tile {{ background: none; border: none; box-shadow: none; padding: 0; }}
#nx-name {{ color: {pal['text']}; font-size: {design.TYPE_SCALE['body']}pt; }}
#nx-row {{ background: none; border: none; box-shadow: none; border-radius: {design.RADIUS['medium']}px; padding: 8px 14px; }}
#nx-row:hover, #nx-row:focus {{ background-color: {hover}; }}
#nx-title {{ color: {pal['text_strong']}; font-size: 11pt; }}
#nx-sub {{ color: {pal['muted']}; font-size: 9pt; }}
#nx-power {{ background: none; border: none; box-shadow: none; padding: 0; }}
#nx-user {{ color: {pal['muted']}; font-size: 9pt; }}
scrolledwindow, viewport {{ background: none; }}
"""


class HexTile(Gtk.Button):
    """An app as a hexagon tile: the hexagon is drawn, the icon sits in it,
    the name underneath; the hexagon lights up in the accent on hover."""

    def __init__(self, app, colours, on_click):
        super().__init__()
        self.set_name("nx-tile")
        self.set_relief(Gtk.ReliefStyle.NONE)
        self.set_tooltip_text(app["comment"] or app["name"])
        self.colours = colours
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        overlay = Gtk.Overlay()
        area = Gtk.DrawingArea()
        area.set_size_request(TILE_HEX_R * 2, int(TILE_HEX_R * 2))
        area.connect("draw", self._draw_hex)
        overlay.add(area)
        image = Gtk.Image.new_from_icon_name(app["icon"], Gtk.IconSize.DIALOG)
        image.set_pixel_size(ICON_PX)
        overlay.add_overlay(image)
        box.pack_start(overlay, False, False, 0)
        name = Gtk.Label(label=app["name"])
        name.set_name("nx-name")
        name.set_ellipsize(Pango.EllipsizeMode.END)
        name.set_max_width_chars(12)
        name.set_width_chars(12)
        box.pack_start(name, False, False, 0)
        self.set_size_request(TILE_W, -1)
        self.add(box)
        self.area = area
        self.connect("state-flags-changed", lambda *_a: area.queue_draw())
        self.connect("clicked", lambda *_a: on_click(app))

    def _draw_hex(self, area, cr):
        w, h = area.get_allocated_width(), area.get_allocated_height()
        hot = bool(self.get_state_flags() & (Gtk.StateFlags.PRELIGHT | Gtk.StateFlags.FOCUSED))
        pts = design.hex_points(w / 2, h / 2, min(w, h) / 2 - 2)
        cr.move_to(*pts[0])
        for p in pts[1:]:
            cr.line_to(*p)
        cr.close_path()
        surface = self.colours["surface"]
        cr.set_source_rgba(*surface, 0.85 if hot else 0.55)
        cr.fill_preserve()
        accent = self.colours["accent"]
        cr.set_source_rgba(*accent, 0.95 if hot else 0.25)
        cr.set_line_width(2 if hot else 1)
        cr.stroke()
        return False


class Nexus(Gtk.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="Nexus")
        visual = self.get_screen().get_rgba_visual()
        if visual:
            self.set_visual(visual)
        GtkLayerShell.init_for_window(self)
        GtkLayerShell.set_namespace(self, "nexa-nexus")  # Hyprland blurs nexa-* layers
        GtkLayerShell.set_layer(self, GtkLayerShell.Layer.OVERLAY)
        GtkLayerShell.set_keyboard_mode(self, GtkLayerShell.KeyboardMode.EXCLUSIVE)
        GtkLayerShell.set_exclusive_zone(self, -1)
        for edge in (GtkLayerShell.Edge.TOP, GtkLayerShell.Edge.BOTTOM,
                     GtkLayerShell.Edge.LEFT, GtkLayerShell.Edge.RIGHT):
            GtkLayerShell.set_anchor(self, edge, True)

        settings = themes.load_settings()
        t = themes.THEMES[settings["theme"]]
        pal = themes.app_palette(t)
        self.colours = {"accent": _rgb(t["accent"]), "surface": _rgb(pal["surface_hi"]), "crit": _rgb(t["crit"]),
                        "muted": _rgb(pal["muted"]), "text": _rgb(pal["text"])}
        css = Gtk.CssProvider()
        css.load_from_data(nexus_css(t, settings["opacity"]).encode())
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), css,
                                                 Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

        self.apps = [a for a in desktopapps.load_apps() if a["name"] not in SELF_NAMES]

        backdrop = Gtk.EventBox()
        backdrop.set_visible_window(False)
        backdrop.connect("button-press-event", lambda *_a: self.close() or True)
        self.add(backdrop)

        self.body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18)
        self.body.set_halign(Gtk.Align.CENTER)
        self.body.set_margin_top(48)
        self.body.set_margin_bottom(28)
        backdrop.add(self.body)

        greeting = Gtk.Label(label=self._greeting())
        greeting.set_name("nx-greeting")
        self.body.pack_start(greeting, False, False, 0)
        date = Gtk.Label(label=GLib.DateTime.new_now_local().format("%A, %d %B"))
        date.set_name("nx-date")
        self.body.pack_start(date, False, False, 0)

        self.search = Gtk.SearchEntry(placeholder_text="Search apps and settings, calculate, or ask Nexa")
        self.search.set_name("nx-search")
        self.search.set_halign(Gtk.Align.CENTER)
        self.search.connect("search-changed", lambda *_a: self._refresh())
        self.search.connect("activate", self._activate_first)
        self.body.pack_start(self.search, False, False, 6)

        self.scroller = Gtk.ScrolledWindow()
        self.scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.scroller.add(self.content)
        self.body.pack_start(self.scroller, True, True, 0)

        self.body.pack_start(self._power_bar(), False, False, 0)

        self.connect("key-press-event", self._on_key)
        self._closing = False
        self.results = []
        self._refresh()
        self.body.set_opacity(0)
        self.connect("map", lambda *_a: self._animate(True))
        GLib.idle_add(self.search.grab_focus)

    # ---- content ---------------------------------------------------------

    def _greeting(self):
        hour = GLib.DateTime.new_now_local().get_hour()
        part = "Good morning" if hour < 12 else "Good afternoon" if hour < 18 else "Good evening"
        try:
            name = pwd.getpwuid(os.getuid()).pw_gecos.split(",")[0] or pwd.getpwuid(os.getuid()).pw_name
        except KeyError:
            name = ""
        return f"{part}, {name.capitalize()}" if name and name != "root" else part

    def _ordered_apps(self):
        by_name = {a["name"]: a for a in self.apps}
        first = [by_name[n] for n in FAVOURITES if n in by_name]
        rest = [a for a in self.apps if a not in first]
        return first + rest

    def _refresh(self):
        for child in self.content.get_children():
            child.destroy()
        query = self.search.get_text().strip()
        if query:
            self.results = search.search(self.apps, query)
            self._fill_results()
        else:
            self.results = []
            self._fill_honeycomb(self._ordered_apps())
        self.content.show_all()

    def _fill_honeycomb(self, apps):
        """Rows of 7 and 6 tiles, each centred: the shorter rows sit half a
        tile in, so the hexagons interlock like a honeycomb."""
        grid = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        grid.set_halign(Gtk.Align.CENTER)
        i, row_index = 0, 0
        while i < len(apps):
            n = ROW_LONG if row_index % 2 == 0 else ROW_SHORT
            row = Gtk.Box(spacing=TILE_GAP)
            row.set_halign(Gtk.Align.START)
            if row_index % 2:
                row.set_margin_start((TILE_W + TILE_GAP) // 2)  # half a tile in: the rows interlock
            for app in apps[i:i + n]:
                row.pack_start(HexTile(app, self.colours, self._launch), False, False, 0)
            grid.pack_start(row, False, False, 0)
            i += n
            row_index += 1
        self.content.pack_start(grid, False, False, 0)

    def _fill_results(self):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        box.set_halign(Gtk.Align.CENTER)
        box.set_size_request(620, -1)
        for r in self.results:
            btn = Gtk.Button()
            btn.set_name("nx-row")
            btn.set_relief(Gtk.ReliefStyle.NONE)
            row = Gtk.Box(spacing=14)
            image = Gtk.Image.new_from_icon_name(r["icon"], Gtk.IconSize.DND)
            image.set_pixel_size(32)
            row.pack_start(image, False, False, 0)
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            title = Gtk.Label(label=r["title"], xalign=0, ellipsize=Pango.EllipsizeMode.END)
            title.set_name("nx-title")
            text.pack_start(title, False, False, 0)
            if r["subtitle"]:
                sub = Gtk.Label(label=r["subtitle"], xalign=0, ellipsize=Pango.EllipsizeMode.END)
                sub.set_name("nx-sub")
                text.pack_start(sub, False, False, 0)
            row.pack_start(text, True, True, 0)
            btn.add(row)
            btn.connect("clicked", lambda _b, res=r: self._activate(res))
            box.pack_start(btn, False, False, 0)
        self.content.pack_start(box, False, False, 0)

    def _power_bar(self):
        bar = Gtk.Box(spacing=18)
        bar.set_halign(Gtk.Align.CENTER)
        user = Gtk.Label(label=f"{pwd.getpwuid(os.getuid()).pw_name} @ {socket.gethostname()}")
        user.set_name("nx-user")
        bar.pack_start(user, False, False, 12)
        for icon, label, action in (("system-lock-screen-symbolic", "Lock", "lock"),
                                    ("weather-clear-night-symbolic", "Sleep", "suspend"),
                                    ("view-refresh-symbolic", "Restart", "reboot"),
                                    ("system-shutdown-symbolic", "Shut down", "poweroff")):
            btn = Gtk.Button()
            btn.set_name("nx-power")
            btn.set_relief(Gtk.ReliefStyle.NONE)
            btn.set_tooltip_text(label)
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            box.pack_start(Gtk.Image.new_from_icon_name(icon, Gtk.IconSize.LARGE_TOOLBAR), False, False, 0)
            lbl = Gtk.Label(label=label)
            lbl.set_name("nx-sub")
            box.pack_start(lbl, False, False, 0)
            btn.add(box)
            btn.connect("clicked", lambda _b, a=action: (power(a), self.close()))
            bar.pack_start(btn, False, False, 0)
        return bar

    # ---- actions ---------------------------------------------------------

    def _launch(self, app):
        desktopapps.launch(app["exec"])
        self.close()

    def _activate(self, result):
        if result["kind"] == "calc":
            Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text(result["value"], -1)
            self.search.set_text(result["value"])
            self.search.set_position(-1)
            return
        if result["command"]:
            spawn(result["command"])
        self.close()

    def _activate_first(self, *_a):
        if self.results:
            self._activate(self.results[0])
        else:
            apps = self._ordered_apps()
            if apps and not self.search.get_text().strip():
                return
            if apps:
                self._launch(apps[0])

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

    # ---- motion ----------------------------------------------------------

    def _animate(self, opening, done=None):
        start = time.monotonic()

        def step():
            t = min(1.0, (time.monotonic() - start) * 1000 / ANIM_MS) if ANIM_MS else 1.0
            eased = 1 - (1 - t) ** 3
            shown = eased if opening else 1 - eased
            self.body.set_opacity(shown)
            self.body.set_margin_top(48 + int(round(24 * (1 - shown))))
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
    return run_panel("os.nexa.Nexus", Nexus)
