"""Control Centre: quick toggles, volume and brightness, battery, and the
notification list (mako's history) with Do Not Disturb."""
import os
import re

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk, Pango

from . import sysinfo
from .panel import Panel, power, run_quiet, spawn, run_panel

CLEARED_FILE = os.path.join(os.environ.get("XDG_CACHE_HOME", os.path.expanduser("~/.cache")),
                            "nexa", "notifications-cleared")


def _cleared_up_to():
    try:
        with open(CLEARED_FILE) as f:
            return int(f.read().strip() or 0)
    except (OSError, ValueError):
        return 0


class ControlCentre(Panel):
    ANCHOR = "right"
    WIDTH = 400

    def __init__(self, app):
        super().__init__(app)
        now = GLib.DateTime.new_now_local()
        head = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        head.pack_start(self.label(now.format("%H:%M"), "title"), False, False, 0)
        head.pack_start(self.label(now.format("%A %d %B"), "muted"), False, False, 0)
        self.body.pack_start(head, False, False, 0)

        grid = Gtk.Grid(column_spacing=10, row_spacing=10, column_homogeneous=True)
        self.dnd = sysinfo.dnd_on(run_quiet(["makoctl", "mode"]))
        tiles = [
            ("Wi-Fi", self._wifi_label(), False, lambda b: self._open(["nexa-network"])),
            ("Do Not Disturb", "On" if self.dnd else "Off", self.dnd, self._toggle_dnd),
            ("Personalize", "Themes", False, lambda b: self._open(["nexa-settings"])),
            ("Screenshot", "Capture", False, lambda b: self._open(["nexa-screenshot"])),
        ]
        for i, (title, sub, on, cb) in enumerate(tiles):
            grid.attach(self._tile(title, sub, on, cb), i % 2, i // 2, 1, 1)
        self.body.pack_start(grid, False, False, 0)

        vol = sysinfo.parse_volume(run_quiet(["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"]))
        if vol is not None:
            self.body.pack_start(self._slider("Volume" + (" (muted)" if vol[1] else ""), vol[0], self._set_volume),
                                 False, False, 0)
        bright = sysinfo.parse_brightness(run_quiet(["brightnessctl", "-m"]))
        if bright is not None:
            self.body.pack_start(self._slider("Brightness", bright, self._set_brightness), False, False, 0)
        bat = sysinfo.battery()
        if bat is not None:
            self.body.pack_start(self.label(f"Battery {bat[0]}%  ·  {bat[1]}", "muted"), False, False, 0)

        self.body.pack_start(Gtk.Separator(), False, False, 0)
        nhead = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        nhead.pack_start(self.label("NOTIFICATIONS", "section"), True, True, 0)
        clear = Gtk.Button(label="Clear all")
        clear.connect("clicked", self._clear_all)
        nhead.pack_end(clear, False, False, 0)
        self.body.pack_start(nhead, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroller.set_min_content_height(180)
        scroller.set_max_content_height(300)
        self.notes = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        scroller.add(self.notes)
        self.body.pack_start(scroller, True, True, 0)
        self._fill_notifications()

        self.body.pack_start(Gtk.Separator(), False, False, 0)
        foot = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        for text, cb in (("Settings", lambda b: self._open(["nexa-settings"])),
                         ("Lock", lambda b: self._power("lock"))):
            btn = Gtk.Button(label=text)
            btn.connect("clicked", cb)
            foot.pack_start(btn, False, False, 0)
        off = Gtk.Button(label="Power")
        off.set_name("power")
        off.connect("clicked", lambda b: self._open(["nexa-power"]))
        foot.pack_end(off, False, False, 0)
        self.body.pack_start(foot, False, False, 0)

    # ---- pieces ----

    def _tile(self, title, subtitle, on, callback):
        btn = Gtk.Button()
        btn.set_name("tile")
        if on:
            btn.get_style_context().add_class("on")
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        box.pack_start(self.label(title), False, False, 0)
        sub = self.label(subtitle, "muted")
        sub.set_ellipsize(Pango.EllipsizeMode.END)
        box.pack_start(sub, False, False, 0)
        btn.add(box)
        btn.connect("clicked", callback)
        return btn

    def _slider(self, title, value, on_change):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        box.pack_start(self.label(title, "section"), False, False, 0)
        scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 100, 1)
        scale.set_value(value)
        scale.set_draw_value(True)
        scale.set_value_pos(Gtk.PositionType.RIGHT)
        scale.connect("value-changed", lambda s: on_change(int(s.get_value())))
        box.pack_start(scale, False, False, 0)
        return box

    def _wifi_label(self):
        out = run_quiet(["sh", "-c", "for d in /sys/class/net/*/wireless; do [ -e \"$d\" ] && basename $(dirname $d); done"])
        for dev in out.split():
            info = re.sub(r"\x1b\[[0-9;]*m", "", run_quiet(["iwctl", "station", dev, "show"], timeout=3))
            for line in info.splitlines():
                if "Connected network" in line:
                    return line.split("Connected network", 1)[1].strip()
        return "Not connected"

    def _fill_notifications(self):
        for child in self.notes.get_children():
            self.notes.remove(child)
        items = sysinfo.notifications(run_quiet(["makoctl", "list"]), run_quiet(["makoctl", "history"]),
                                      _cleared_up_to())
        if not items:
            self.notes.pack_start(self.label("You're all caught up.", "muted"), False, False, 6)
        for n in items[:20]:
            card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            card.set_name("notif")
            if n["app"]:
                card.pack_start(self.label(n["app"], "accent"), False, False, 0)
            card.pack_start(self.label(n["summary"], wrap=True), False, False, 0)
            if n["body"]:
                card.pack_start(self.label(n["body"], "muted", wrap=True), False, False, 0)
            self.notes.pack_start(card, False, False, 0)
        self.notes.show_all()

    # ---- actions ----

    def _open(self, argv):
        spawn(argv)
        self.close()

    def _power(self, action):
        power(action)
        self.close()

    def _toggle_dnd(self, btn):
        run_quiet(["makoctl", "mode", "-t", "do-not-disturb"])
        self.dnd = not self.dnd
        ctx = btn.get_style_context()
        (ctx.add_class if self.dnd else ctx.remove_class)("on")
        sub = btn.get_child().get_children()[1]
        sub.set_text("On" if self.dnd else "Off")

    def _set_volume(self, pct):
        run_quiet(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", f"{pct}%"])

    def _set_brightness(self, pct):
        run_quiet(["brightnessctl", "set", f"{max(pct, 1)}%"])

    def _clear_all(self, _btn):
        items = sysinfo.notifications(run_quiet(["makoctl", "list"]), run_quiet(["makoctl", "history"]))
        newest = max((n["id"] for n in items), default=_cleared_up_to())
        os.makedirs(os.path.dirname(CLEARED_FILE), exist_ok=True)
        with open(CLEARED_FILE, "w") as f:
            f.write(str(newest))
        run_quiet(["makoctl", "dismiss", "--all"])
        self._fill_notifications()


def main():
    return run_panel("os.nexa.ControlCentre", ControlCentre)
