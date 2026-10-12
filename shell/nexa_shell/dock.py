"""NexaOS Dock: pinned and running apps along the bottom edge, magnifying
under the pointer like the macOS Dock. Clicking opens an app, or brings a
running one forward (restoring it if minimised); a launching app's icon
bounces until its window appears. Follows theme changes live."""
import math
import os
import time

import cairo
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
gi.require_version("PangoCairo", "1.0")
from gi.repository import Gdk, GdkPixbuf, Gio, GLib, Gtk, GtkLayerShell, Pango, PangoCairo

from nexa_common import compositor, desktopapps, themes
from .panel import spawn

# Desktop-file ids, in order.
# Desktop-file ids, in order; a tuple lists names the same app has had in
# different Alpine releases (foot: org.codeberg.dnkl.foot in 3.19, foot in 3.24).
PINNED_CHOICES = ["nexa-files", "nexa-browser", ("foot", "org.codeberg.dnkl.foot"), "nexa-settings",
                  "nexa-software", "nexa-ai", "nexa-notes", "nexa-music"]
# Window app_id -> desktop-file id, where they differ.
ALIASES = {"foot": "org.codeberg.dnkl.foot"}
PINNED = []  # resolved against the installed apps when the Dock starts


def _resolve_pinned(installed):
    out = []
    for choice in PINNED_CHOICES:
        options = choice if isinstance(choice, tuple) else (choice,)
        out.append(next((o for o in options if o in installed), options[0]))
    return out
START_ID = "__start__"

BASE = 48          # icon size at rest
MAG = 1.75         # size of the icon right under the pointer, relative to BASE
REACH = 2.6        # how many icon slots the magnification spreads over
GAP = 10           # space between icons
PAD = 10           # dock padding around the icons
BOTTOM = 8         # gap between the dock and the screen edge
LABEL_H = 36       # room above the tallest icon for the name label
ICON_PX = 128      # resolution icons are loaded at (scaled down when drawn)
BOUNCE_S = 0.55    # one bounce
BOUNCE_LIMIT = 8.0 # stop bouncing if no window shows up by then

DOCK_H = BASE + 2 * PAD
SURFACE_H = int(BASE * MAG) + PAD + BOTTOM + LABEL_H


def _rgba(hex_colour, alpha=1.0):
    h = hex_colour.lstrip("#")
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)) + (alpha,)


def _running_app_ids():
    """app_ids of every open window, including minimised ones."""
    found = set()
    for w in compositor.windows():
        app_id = w["app_id"]
        # Only translate when the window's own id isn't an installed app.
        found.add(app_id if app_id in _INSTALLED else ALIASES.get(app_id, app_id))
    return found


_INSTALLED = set()


class Dock(Gtk.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="NexaOS Dock")
        visual = self.get_screen().get_rgba_visual()
        if visual:
            self.set_visual(visual)
        self.set_app_paintable(True)

        GtkLayerShell.init_for_window(self)
        GtkLayerShell.set_namespace(self, "nexa-dock")  # Hyprland blurs nexa-* layers
        GtkLayerShell.set_layer(self, GtkLayerShell.Layer.TOP)
        GtkLayerShell.set_anchor(self, GtkLayerShell.Edge.BOTTOM, True)
        GtkLayerShell.set_keyboard_mode(self, GtkLayerShell.KeyboardMode.NONE)
        GtkLayerShell.set_exclusive_zone(self, DOCK_H + BOTTOM)

        self.apps = {a["id"]: a for a in desktopapps.load_apps()}
        _INSTALLED.clear()
        _INSTALLED.update(self.apps)
        PINNED[:] = _resolve_pinned(self.apps)
        self.items = []
        self.pointer_x = None
        self.hover = None
        self._ticking = False
        self._load_theme()

        self.area = Gtk.DrawingArea()
        self.area.add_events(Gdk.EventMask.POINTER_MOTION_MASK | Gdk.EventMask.LEAVE_NOTIFY_MASK |
                             Gdk.EventMask.ENTER_NOTIFY_MASK | Gdk.EventMask.BUTTON_PRESS_MASK)
        self.area.connect("draw", self._draw)
        self.area.connect("motion-notify-event", self._on_motion)
        self.area.connect("enter-notify-event", self._on_enter)
        self.area.connect("leave-notify-event", self._on_leave)
        self.area.connect("button-press-event", self._on_click)
        self.add(self.area)

        self._rebuild(set())
        self._refresh_running()
        GLib.timeout_add(1500, self._refresh_running)
        self._theme_monitor = Gio.File.new_for_path(themes.settings_path()).monitor_file(
            Gio.FileMonitorFlags.NONE, None)
        self._theme_monitor.connect("changed", lambda *_a: (self._load_theme(), self.area.queue_draw()))
        self.area.connect("size-allocate", lambda *_a: self.pointer_x is None and self._set_input_region(False))
        # For screenshots/tests: pretend the pointer rests at this x position.
        preview_x = os.environ.get("NEXA_DOCK_PREVIEW_X")
        if preview_x:
            GLib.timeout_add(500, self._preview, float(preview_x))

    def _preview(self, x):
        self.pointer_x = x
        layout, _, _ = self._layout(self.area.get_allocated_width())
        self.hover = next((i for i, (lx, size, _) in enumerate(layout) if lx <= x <= lx + size), None)
        self._targets()
        self._animate()
        return False

    # ---- model ----

    def _load_theme(self):
        s = themes.load_settings()
        t = themes.THEMES[s["theme"]]
        self.colours = {
            "tray": _rgba("#%02x%02x%02x" % t["bg"], max(s["opacity"], 0.55)),
            "edge": _rgba(t["accent"], 0.35),
            "dot": _rgba(t["accent"]),
            "label_bg": _rgba("#%02x%02x%02x" % t["surface"], 0.95),
            "label_fg": _rgba(t["text"]),
            "divider": _rgba(t["muted"], 0.4),
            "logo": _rgba(t["accent"]),
        }

    def _icon(self, name):
        theme = Gtk.IconTheme.get_default()
        for candidate in (name, "application-x-executable"):
            try:
                if candidate.startswith("/"):
                    return GdkPixbuf.Pixbuf.new_from_file_at_size(candidate, ICON_PX, ICON_PX)
                return theme.load_icon(candidate, ICON_PX, Gtk.IconLookupFlags.FORCE_SIZE)
            except GLib.Error:
                continue
        return None

    def _item(self, app_id, pinned):
        app = self.apps.get(app_id)
        if app is None and app_id != START_ID:
            return None
        old = next((i for i in self.items if i["id"] == app_id), None)
        if old:
            old["pinned"] = pinned
            return old
        if app_id == START_ID:
            return {"id": START_ID, "name": "NexaOS", "pixbuf": None, "pinned": True,
                    "running": False, "scale": 1.0, "bounce": None}
        return {"id": app_id, "name": app["name"], "exec": app["exec"], "pixbuf": self._icon(app["icon"]),
                "pinned": pinned, "running": False, "scale": 1.0, "bounce": None}

    def _rebuild(self, running_ids):
        items = [self._item(a, a == START_ID or a in PINNED) for a in self._wanted_ids(running_ids)]
        items = [i for i in items if i]
        for it in items:
            it["running"] = it["id"] in running_ids
            if it["running"]:
                it["bounce"] = None
        self.items = items
        width = int(len(items) * (BASE + GAP) + 2 * PAD + 3 * BASE * (MAG - 1) + 40)
        self.area.set_size_request(width, SURFACE_H)
        self.area.queue_draw()

    def _wanted_ids(self, running):
        extra = sorted(r for r in running if r in self.apps and r not in PINNED)
        return [START_ID] + [a for a in PINNED if a in self.apps] + extra

    def _refresh_running(self):
        running = _running_app_ids()
        if self._wanted_ids(running) != [i["id"] for i in self.items]:
            self._rebuild(running)
        else:
            changed = False
            for it in self.items:
                now_running = it["id"] in running
                if now_running != it["running"]:
                    it["running"], changed = now_running, True
                if now_running:
                    it["bounce"] = None
            if changed:
                self.area.queue_draw()
        return True

    # ---- layout ----

    def _layout(self, width):
        """x, size and base centre of every icon; magnification is measured
        against the resting layout so the icons don't chase the pointer."""
        n = len(self.items)
        rest_total = n * BASE + (n - 1) * GAP
        rest_left = (width - rest_total) / 2
        sizes = [BASE * it["scale"] for it in self.items]
        total = sum(sizes) + (n - 1) * GAP
        x = (width - total) / 2
        out = []
        for i, it in enumerate(self.items):
            centre_rest = rest_left + i * (BASE + GAP) + BASE / 2
            out.append((x, sizes[i], centre_rest))
            x += sizes[i] + GAP
        return out, rest_left, rest_total

    def _targets(self):
        width = self.area.get_allocated_width()
        layout, _, _ = self._layout(width)
        for it, (_, _, centre) in zip(self.items, layout):
            if self.pointer_x is None:
                it["target"] = 1.0
            else:
                d = abs(self.pointer_x - centre) / (BASE + GAP)
                f = max(0.0, math.cos(min(d / REACH, 1.0) * math.pi / 2))
                it["target"] = 1 + (MAG - 1) * f

    def _animate(self):
        if self._ticking:
            return
        self._ticking = True

        def tick():
            moving = False
            now = time.monotonic()
            for it in self.items:
                target = it.get("target", 1.0)
                if abs(it["scale"] - target) > 0.002:
                    it["scale"] += (target - it["scale"]) * 0.28
                    moving = True
                else:
                    it["scale"] = target
                if it["bounce"] is not None:
                    if now - it["bounce"] > BOUNCE_LIMIT:
                        it["bounce"] = None
                    else:
                        moving = True
            self.area.queue_draw()
            if not moving:
                self._ticking = False
            return moving

        GLib.timeout_add(16, tick)

    # ---- input ----

    def _set_input_region(self, whole_surface):
        """Only the tray takes clicks at rest, so the empty space above it
        (room for magnified icons) doesn't block the windows behind it."""
        w, h = self.area.get_allocated_width(), self.area.get_allocated_height()
        if whole_surface:
            region = cairo.Region(cairo.RectangleInt(0, 0, w, h))
        else:
            _, left, total = self._layout(w)
            region = cairo.Region(cairo.RectangleInt(int(left - PAD), h - BOTTOM - DOCK_H,
                                                     int(total + 2 * PAD), DOCK_H))
        self.input_shape_combine_region(region)

    def _on_enter(self, _w, event):
        self._set_input_region(True)
        return self._on_motion(_w, event)

    def _on_motion(self, _w, event):
        h = self.area.get_allocated_height()
        if event.y < h - BOTTOM - DOCK_H - BASE * (MAG - 1) - 4:
            return self._on_leave(_w, event)  # above the magnified icons
        self.pointer_x = event.x
        layout, _, _ = self._layout(self.area.get_allocated_width())
        self.hover = next((i for i, (x, size, _) in enumerate(layout) if x - GAP / 2 <= event.x <= x + size + GAP / 2),
                          None)
        self._targets()
        self._animate()
        return False

    def _on_leave(self, _w, _event):
        self.pointer_x = None
        self.hover = None
        self._targets()
        self._animate()
        self._set_input_region(False)
        return False

    def _on_click(self, _w, event):
        if event.button != 1 or self.hover is None:
            return False
        it = self.items[self.hover]
        if it["id"] == START_ID:
            spawn(["nexa-start"])
        elif it["running"]:
            window_id = next((k for k, v in ALIASES.items() if v == it["id"]), it["id"])
            compositor.focus_app(window_id)
        else:
            desktopapps.launch(it["exec"])
            it["bounce"] = time.monotonic()
            self._animate()
        return True

    # ---- drawing ----

    def _draw(self, _w, cr):
        w, h = self.area.get_allocated_width(), self.area.get_allocated_height()
        cr.set_operator(cairo.OPERATOR_SOURCE)
        cr.set_source_rgba(0, 0, 0, 0)
        cr.paint()
        cr.set_operator(cairo.OPERATOR_OVER)

        layout, _, _ = self._layout(w)
        # The tray grows with the magnified icons, like the macOS Dock.
        first_x = layout[0][0] if layout else w / 2
        last_x = layout[-1][0] + layout[-1][1] if layout else w / 2
        tray_x, tray_w = first_x - PAD, last_x - first_x + 2 * PAD
        tray_y = h - BOTTOM - DOCK_H
        self._rounded(cr, tray_x, tray_y, tray_w, DOCK_H, 18)
        cr.set_source_rgba(*self.colours["tray"])
        cr.fill_preserve()
        cr.set_source_rgba(*self.colours["edge"])
        cr.set_line_width(1)
        cr.stroke()

        icon_bottom = h - BOTTOM - PAD
        now = time.monotonic()
        for i, (it, (x, size, _)) in enumerate(zip(self.items, layout)):
            lift = 0.0
            if it["bounce"] is not None:
                phase = ((now - it["bounce"]) % BOUNCE_S) / BOUNCE_S
                lift = abs(math.sin(phase * math.pi)) * BASE * 0.45
            y = icon_bottom - size - lift
            if it["id"] == START_ID:
                self._logo(cr, x + size / 2, y + size / 2, size / 2)
            elif it["pixbuf"] is not None:
                cr.save()
                cr.translate(x, y)
                k = size / it["pixbuf"].get_width()
                cr.scale(k, k)
                Gdk.cairo_set_source_pixbuf(cr, it["pixbuf"], 0, 0)
                cr.get_source().set_filter(cairo.FILTER_GOOD)
                cr.paint()
                cr.restore()
            if it["running"]:
                cr.arc(x + size / 2, h - BOTTOM - 4, 2.5, 0, 2 * math.pi)
                cr.set_source_rgba(*self.colours["dot"])
                cr.fill()
            nxt = self.items[i + 1] if i + 1 < len(self.items) else None
            if nxt is not None and it["pinned"] and not nxt["pinned"]:
                dx = x + size + GAP / 2
                cr.move_to(dx, tray_y + 10)
                cr.line_to(dx, tray_y + DOCK_H - 10)
                cr.set_source_rgba(*self.colours["divider"])
                cr.set_line_width(1)
                cr.stroke()

        if self.hover is not None and self.hover < len(self.items):
            x, size, _ = layout[self.hover]
            self._label(cr, self.items[self.hover]["name"], x + size / 2, icon_bottom - size - 10)
        return True

    def _logo(self, cr, cx, cy, r):
        """The NexaOS mark: hexagon outline, ring and core dot."""
        cr.set_source_rgba(*self.colours["logo"])
        cr.set_line_width(max(2.0, r * 0.09))
        for k in range(7):
            a = math.radians(-90 + 60 * k)
            (cr.move_to if k == 0 else cr.line_to)(cx + r * 0.9 * math.cos(a), cy + r * 0.9 * math.sin(a))
        cr.close_path()
        cr.stroke()
        cr.set_line_width(max(1.0, r * 0.05))
        cr.arc(cx, cy, r * 0.38, 0, 2 * math.pi)
        cr.stroke()
        cr.arc(cx, cy, r * 0.2, 0, 2 * math.pi)
        cr.fill()

    def _label(self, cr, text, cx, bottom):
        layout = PangoCairo.create_layout(cr)
        layout.set_font_description(Pango.FontDescription("Noto Sans 10"))
        layout.set_text(text, -1)
        tw, th = layout.get_pixel_size()
        bw, bh = tw + 20, th + 10
        self._rounded(cr, cx - bw / 2, bottom - bh, bw, bh, bh / 2)
        cr.set_source_rgba(*self.colours["label_bg"])
        cr.fill()
        cr.move_to(cx - tw / 2, bottom - bh + 5)
        cr.set_source_rgba(*self.colours["label_fg"])
        PangoCairo.show_layout(cr, layout)

    @staticmethod
    def _rounded(cr, x, y, w, h, r):
        cr.new_sub_path()
        cr.arc(x + w - r, y + r, r, -math.pi / 2, 0)
        cr.arc(x + w - r, y + h - r, r, 0, math.pi / 2)
        cr.arc(x + r, y + h - r, r, math.pi / 2, math.pi)
        cr.arc(x + r, y + r, r, math.pi, 3 * math.pi / 2)
        cr.close_path()


def main():
    app = Gtk.Application(application_id="os.nexa.Dock", flags=Gio.ApplicationFlags.FLAGS_NONE)
    state = {}

    def on_activate(application):
        if state.get("win"):
            return  # already running
        state["win"] = Dock(application)
        state["win"].show_all()

    app.connect("activate", on_activate)
    return app.run([])
