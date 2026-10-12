"""Shared base for the shell's pop-up panels (start menu, Control Centre).

Each panel is a full-screen, transparent layer-shell surface above every
window: the panel itself sits in one corner, and a click anywhere else, or
Escape, closes it. Running the same command again while it is open closes
it too (Gtk.Application's single instance), so one key or bar button
toggles it.
"""
import json
import subprocess
import time

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gdk, Gio, GLib, Gtk, GtkLayerShell

from nexa_common import themes


def run_quiet(argv, timeout=5):
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return r.stdout if r.returncode == 0 else ""
    except (OSError, subprocess.SubprocessError):
        return ""


def spawn(argv):
    try:
        subprocess.Popen(argv, start_new_session=True,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError:
        pass


def power(action):
    """Lock, sleep, restart or shut down through the existing helpers."""
    if action == "lock":
        spawn(["nexa-lock"])
        return
    payload = json.dumps({"action": action})
    spawn(["sh", "-c", f"echo '{payload}' | doas /usr/local/bin/nexa-power-helper"])


# Open/close animation: fade plus a short slide down into place, eased out
# (fast start, gentle landing), like macOS's Control Center.
ANIM_MS = 170
SLIDE_PX = 8  # margins cannot go below 0, so at most REST_MARGIN
REST_MARGIN = 8


class Panel(Gtk.ApplicationWindow):
    """Subclasses fill self.body and set ANCHOR to "left" or "right"."""
    ANCHOR = "left"
    WIDTH = 600

    def __init__(self, app):
        super().__init__(application=app, title="NexaOS panel")
        screen = self.get_screen()
        visual = screen.get_rgba_visual()
        if visual:
            self.set_visual(visual)
        self.set_app_paintable(True)

        GtkLayerShell.init_for_window(self)
        GtkLayerShell.set_namespace(self, "nexa-panel")  # Hyprland blurs nexa-* layers
        GtkLayerShell.set_layer(self, GtkLayerShell.Layer.OVERLAY)
        GtkLayerShell.set_keyboard_mode(self, GtkLayerShell.KeyboardMode.EXCLUSIVE)
        GtkLayerShell.set_exclusive_zone(self, 0)
        for edge in (GtkLayerShell.Edge.TOP, GtkLayerShell.Edge.BOTTOM,
                     GtkLayerShell.Edge.LEFT, GtkLayerShell.Edge.RIGHT):
            GtkLayerShell.set_anchor(self, edge, True)

        settings = themes.load_settings()
        css = Gtk.CssProvider()
        css.load_from_data(themes.panel_css(themes.THEMES[settings["theme"]], settings["opacity"]).encode())
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

        # Backdrop: clicks that reach it (outside the panel) close the panel.
        backdrop = Gtk.EventBox()
        backdrop.set_visible_window(False)
        backdrop.connect("button-press-event", lambda *_a: self.close() or True)
        self.add(backdrop)

        holder = Gtk.EventBox()  # swallows clicks on the panel itself
        holder.connect("button-press-event", lambda *_a: True)
        holder.set_halign(Gtk.Align.START if self.ANCHOR == "left" else Gtk.Align.END)
        holder.set_valign(Gtk.Align.START)
        holder.set_margin_top(REST_MARGIN)
        holder.set_margin_start(8)
        holder.set_margin_end(8)
        backdrop.add(holder)
        self.holder = holder

        self.body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        self.body.set_name("panel")
        self.body.set_size_request(self.WIDTH, -1)
        holder.add(self.body)

        self.connect("key-press-event", self._on_key)
        self._closing = False
        self.body.set_opacity(0)
        self.connect("map", lambda *_a: self._animate(opening=True))

    def _animate(self, opening, done=None):
        start = time.monotonic()

        def step():
            t = min(1.0, (time.monotonic() - start) * 1000 / ANIM_MS)
            eased = 1 - (1 - t) ** 3
            shown = eased if opening else 1 - eased
            self.body.set_opacity(shown)
            # Slides down from above its resting place as it appears.
            self.holder.set_margin_top(max(0, int(round(REST_MARGIN - SLIDE_PX * (1 - shown)))))
            if t >= 1.0:
                if done:
                    done()
                return False
            return True

        GLib.timeout_add(12, step)

    def close(self):
        """Animate out, then really close. Every way of dismissing a panel
        (Escape, clicking outside, launching an app, the toggle key) ends here."""
        if self._closing:
            return
        self._closing = True
        self._animate(opening=False, done=lambda: Gtk.ApplicationWindow.close(self))

    def _on_key(self, _w, event):
        if event.keyval == Gdk.KEY_Escape:
            self.close()
            return True
        return False

    def label(self, text, name=None, xalign=0.0, wrap=False):
        lbl = Gtk.Label(label=text)
        lbl.set_xalign(xalign)
        if name:
            lbl.set_name(name)
        if wrap:
            lbl.set_line_wrap(True)
        return lbl


def run_panel(app_id, window_class):
    """Single instance: a second launch closes the open panel."""
    app = Gtk.Application(application_id=app_id, flags=Gio.ApplicationFlags.FLAGS_NONE)
    state = {"win": None}

    def on_activate(application):
        if state["win"] is not None:
            state["win"].close()
            return
        win = window_class(application)
        win.connect("destroy", lambda *_a: application.quit())
        state["win"] = win
        win.show_all()

    app.connect("activate", on_activate)
    return app.run([])
