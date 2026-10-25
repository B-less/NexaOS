"""The desktop's "For you" card: what Nexa Patterns (nexa-patternd, Rust)
expects you to open next, drawn on the desktop itself - on the layer-shell
BOTTOM layer, above the wallpaper and below every window, so it's there
whenever the desktop is and never covers your work.

Click an app or file to open it; hover for why it's suggested; right-click
for Don't suggest this, Hide for today, Move the card, Pattern settings.
The card shows only suggestions confident enough to be worth it, and
nothing at all when there's nothing to say (or Patterns is off)."""
import os
import time

import cairo
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gdk, Gio, GLib, Gtk, GtkLayerShell, Pango

from nexa_common import desktopapps, patterns, themes
from .panel import spawn

REFRESH_S = 300
ICON_PX = 40
HEX_R = 30
BAR_H = 44  # room for the top bar


def _rgb(hex_colour):
    h = hex_colour.lstrip("#")
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))


def card_css(t, pal):
    r, g, b = (int(c * 255) for c in _rgb(pal["surface"] if pal["surface"].startswith("#") else t["surface"]))
    ar, ag, ab = (int(c * 255) for c in _rgb(t["accent"]))
    return f"""
#nx-card {{ background-color: rgba({r}, {g}, {b}, 0.78); border-radius: 16px;
            border: 1px solid rgba({ar}, {ag}, {ab}, 0.35); padding: 14px 16px;
            box-shadow: inset 0 1px rgba(255, 255, 255, 0.14); }}
#nx-card-title {{ color: {pal['text_strong']}; font-family: Inter, sans-serif; font-weight: 700; font-size: 11pt; }}
#nx-card-clock {{ color: {pal['muted']}; font-family: "JetBrains Mono", monospace; font-size: 9pt; }}
.nx-sugg {{ background: none; border: none; box-shadow: none; padding: 4px; border-radius: 10px; color: {pal['text']}; }}
.nx-sugg:hover {{ background-color: rgba({ar}, {ag}, {ab}, 0.16); }}
.nx-sugg-name {{ font-family: Inter, sans-serif; font-size: 9pt; color: {pal['text']}; }}
.nx-sugg-line {{ font-family: Inter, sans-serif; font-size: 9.5pt; color: {pal['text']}; }}
.nx-sugg-note {{ font-family: Inter, sans-serif; font-size: 9pt; color: {pal['muted']}; }}
"""


class HexIcon(Gtk.Overlay):
    """An app icon on a soft hexagon in the accent colour (the Hexa motif)."""

    def __init__(self, icon_name, accent):
        super().__init__()
        self.accent = accent
        area = Gtk.DrawingArea()
        area.set_size_request(HEX_R * 2, HEX_R * 2)
        area.connect("draw", self._draw)
        self.add(area)
        image = Gtk.Image.new_from_icon_name(icon_name or "application-x-executable", Gtk.IconSize.DIALOG)
        # A NexaHexa tile is a hexagon already: full size, nothing behind it.
        self.tile = themes.hexa_tile(icon_name)
        image.set_pixel_size(HEX_R * 2 if self.tile else ICON_PX - 10)
        self.add_overlay(image)

    def _draw(self, area, cr):
        import math
        if self.tile:
            return False
        w, h = area.get_allocated_width(), area.get_allocated_height()
        cx, cy, r = w / 2, h / 2, min(w, h) / 2 - 1
        cr.new_path()
        for i in range(6):
            a = math.pi / 6 + i * math.pi / 3
            cr.line_to(cx + r * math.cos(a), cy + r * math.sin(a))
        cr.close_path()
        cr.set_source_rgba(*self.accent, 0.18)
        cr.fill_preserve()
        cr.set_source_rgba(*self.accent, 0.45)
        cr.set_line_width(1)
        cr.stroke()


class DesktopCard(Gtk.Window):
    def __init__(self):
        super().__init__(title="For you")
        visual = self.get_screen().get_rgba_visual()
        if visual:
            self.set_visual(visual)
        self.set_app_paintable(True)
        self.connect("draw", self._clear)
        GtkLayerShell.init_for_window(self)
        GtkLayerShell.set_namespace(self, "nexa-desktop-card")  # Hyprland blurs nexa-* layers
        GtkLayerShell.set_layer(self, GtkLayerShell.Layer.BOTTOM)
        GtkLayerShell.set_keyboard_mode(self, GtkLayerShell.KeyboardMode.NONE)
        GtkLayerShell.set_exclusive_zone(self, 0)

        self.provider = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), self.provider,
                                                 Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 2)
        self._load_theme()
        self._theme_monitor = Gio.File.new_for_path(themes.settings_path()).monitor_file(Gio.FileMonitorFlags.NONE, None)
        self._theme_monitor.connect("changed", lambda *_a: self._load_theme())
        # Settings > Privacy changes show at once.
        self._cfg_monitor = Gio.File.new_for_path(patterns.config_path()).monitor_file(Gio.FileMonitorFlags.NONE, None)
        self._cfg_monitor.connect("changed", lambda *_a: GLib.timeout_add(300, self.refresh))

        self.card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.card.set_name("nx-card")
        self.add(self.card)
        self.shown = {"apps": [], "file": None, "learning": False}
        self.position = None
        self.refresh()
        GLib.timeout_add_seconds(REFRESH_S, self._tick)

    def _clear(self, _w, cr):
        cr.set_operator(cairo.OPERATOR_CLEAR)
        cr.paint()
        cr.set_operator(cairo.OPERATOR_OVER)
        return False

    def _load_theme(self):
        themes.follow_icon_theme()
        settings = themes.load_settings()
        t = themes.THEMES[settings["theme"]]
        self.accent = _rgb(t["accent"])
        self.provider.load_from_data(card_css(t, themes.app_palette(t)).encode())
        self.queue_draw()

    def _tick(self):
        self.refresh()
        return True

    def _place(self, position):
        if position == self.position:
            return
        self.position = position
        edges = {"top-left": ("TOP", "LEFT"), "top-right": ("TOP", "RIGHT"),
                 "bottom-right": ("BOTTOM", "RIGHT"), "center": ()}.get(position, ("TOP", "RIGHT"))
        for edge in ("TOP", "BOTTOM", "LEFT", "RIGHT"):
            e = getattr(GtkLayerShell.Edge, edge)
            GtkLayerShell.set_anchor(self, e, edge in edges)
            GtkLayerShell.set_margin(self, e, BAR_H + 12 if edge == "TOP" else 120 if edge == "BOTTOM" else 24)

    def refresh(self):
        try:
            data = patterns.suggestions("card")
        except patterns.PatternsError:
            data = {"show": False, "apps": [], "file": None, "learning": False}
        self._place(data.get("position", "top-right"))
        apps, file, learning = data.get("apps", []), data.get("file"), data.get("learning", False)
        if not data.get("show") and not learning:
            apps, file = [], None
        self.shown = {"apps": apps, "file": file, "learning": learning}
        for child in self.card.get_children():
            child.destroy()
        if not apps and not file and not learning:
            self.hide()  # nothing worth saying: no card at all
            return False
        head = Gtk.Box(spacing=12)
        title = Gtk.Label(label="For you", xalign=0)
        title.set_name("nx-card-title")
        head.pack_start(title, True, True, 0)
        clock = Gtk.Label(label=time.strftime("%H:%M"))
        clock.set_name("nx-card-clock")
        head.pack_end(clock, False, False, 0)
        self.card.pack_start(head, False, False, 0)
        if learning and not apps:
            note = Gtk.Label(label="Learning how you use NexaOS.\nSuggestions appear here after an hour or two of use.", xalign=0)
            note.get_style_context().add_class("nx-sugg-note")
            self.card.pack_start(note, False, False, 0)
        if apps:
            row = Gtk.Box(spacing=8)
            for s in apps:
                row.pack_start(self._app_button(s), False, False, 0)
            self.card.pack_start(row, False, False, 0)
        if file:
            self.card.pack_start(self._file_button(file), False, False, 0)
        self.card.show_all()
        self.show_all()
        return False

    def _app_button(self, s):
        btn = Gtk.Button()
        btn.get_style_context().add_class("nx-sugg")
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        box.pack_start(HexIcon(s["entry"].get("icon"), self.accent), False, False, 0)
        name = Gtk.Label(label=s["entry"]["name"])
        name.set_max_width_chars(11)
        name.set_ellipsize(Pango.EllipsizeMode.END)
        name.get_style_context().add_class("nx-sugg-name")
        box.pack_start(name, False, False, 0)
        btn.add(box)
        btn.set_tooltip_text(s["text"])
        btn.connect("clicked", lambda _b: self._open_app(s))
        btn.connect("button-press-event", lambda _b, ev: self._menu(ev, s) if ev.button == 3 else False)
        btn.suggestion = s
        return btn

    def _file_button(self, f):
        path = f["path"]
        btn = Gtk.Button()
        btn.get_style_context().add_class("nx-sugg")
        folder = os.path.basename(os.path.dirname(path)) or "/"
        label = Gtk.Label(label=f"\U0001F4C4  {os.path.basename(path)}  ({folder})", xalign=0)
        label.set_max_width_chars(36)
        label.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
        label.get_style_context().add_class("nx-sugg-line")
        btn.add(label)
        btn.set_tooltip_text(f"You've been working on this lately, at about this time ({f['app']}).")
        btn.connect("clicked", lambda _b: self._open_file(path))
        btn.connect("button-press-event", lambda _b, ev: self._menu(ev, None) if ev.button == 3 else False)
        return btn

    def _open_app(self, s):
        patterns.launch(s)

    def _open_file(self, path):
        try:
            Gio.AppInfo.launch_default_for_uri(Gio.File.new_for_path(path).get_uri(), None)
        except GLib.Error:
            pass

    def _menu(self, event, s):
        menu = Gtk.Menu()

        def item(label, action, sensitive=True):
            mi = Gtk.MenuItem(label=label)
            mi.set_sensitive(sensitive)
            mi.connect("activate", lambda *_a: action())
            menu.append(mi)
            return mi

        if s:
            why = item(s["text"], lambda: None, False)
            why.get_child().set_line_wrap(True)
            why.get_child().set_max_width_chars(40)
            menu.append(Gtk.SeparatorMenuItem())
            item(f"Don't suggest {s['entry']['name']}", lambda: self.dont_suggest(s))
        item("Hide for today", self.hide_today)
        move = Gtk.MenuItem(label="Move the card")
        sub = Gtk.Menu()
        for pos, label in patterns.POSITIONS.items():
            mi = Gtk.MenuItem(label=label)
            mi.connect("activate", lambda *_a, p=pos: self.move_to(p))
            sub.append(mi)
        move.set_submenu(sub)
        menu.append(move)
        menu.append(Gtk.SeparatorMenuItem())
        item("Pattern settings…", lambda: spawn(["nexa-settings", "privacy"]))
        menu.show_all()
        menu.attach_to_widget(self, None)
        menu.popup_at_pointer(event)
        self._open_menu = menu
        return True

    def dont_suggest(self, s):
        patterns.never_suggest(s["app"])
        self.refresh()

    def hide_today(self):
        patterns.hide_for_today()
        self.refresh()

    def move_to(self, position):
        patterns.update_config(card_position=position)
        self.refresh()


def main():
    Gtk.init()
    card = DesktopCard()
    card.connect("destroy", Gtk.main_quit)
    Gtk.main()
    return 0
