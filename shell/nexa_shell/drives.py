"""USB sticks and SD cards on the desktop.

The system broker (nexa-brokerd, Rust) mounts a drive when it is plugged in
and writes the current list to /run/nexa/drives.json after every change
(plugged in, opened, closed, ejected, pulled out). This layer watches that
file and shows each drive as a hexagon with its name and size, on the
layer-shell BOTTOM layer: above the wallpaper, below every window, top-left
(the For you card is top-right).

Click: open it in Files (mounting it first if it isn't). Right-click: Open,
Eject. Nothing is shown when no drive is plugged in."""
import json
import os
import threading

import cairo
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gdk, Gio, GLib, Gtk, GtkLayerShell, Pango

from nexa_common import removable, themes
from .desktop import BAR_H, HexIcon, _rgb
from .panel import spawn

STATE = os.environ.get("NEXA_DRIVES_STATE", "/run/nexa/drives.json")


def drives_css(t, pal):
    ar, ag, ab = (int(c * 255) for c in _rgb(t["accent"]))
    return f"""
.nx-drive {{ background: none; border: none; box-shadow: none; padding: 6px 4px; border-radius: 12px; }}
.nx-drive:hover {{ background-color: rgba({ar}, {ag}, {ab}, 0.18); }}
.nx-drive-name {{ font-family: Inter, sans-serif; font-size: 9.5pt; font-weight: 600; color: #ffffff;
                  text-shadow: 0 1px 3px rgba(0, 0, 0, 0.8); }}
.nx-drive-size {{ font-family: Inter, sans-serif; font-size: 8.5pt; color: rgba(255, 255, 255, 0.78);
                  text-shadow: 0 1px 3px rgba(0, 0, 0, 0.8); }}
"""


def drive_name(d):
    """Its label, or what it is: the size is shown on the line below."""
    if d.get("label"):
        return d["label"]
    return "SD card" if str(d.get("name", "")).startswith("mmcblk") else "USB drive"


def read_state(path=STATE):
    """The drives the broker last published (empty if none or unreadable)."""
    try:
        with open(path) as f:
            data = json.load(f)
    except (OSError, ValueError):
        return []
    # Anything but {"drives": [...]} (a half-written or odd file) means none.
    drives = data.get("drives", []) if isinstance(data, dict) else []
    if not isinstance(drives, list):
        return []
    return [d for d in drives if isinstance(d, dict) and d.get("name")]


class DesktopDrives(Gtk.Window):
    def __init__(self):
        super().__init__(title="Drives")
        self.shown = None
        visual = self.get_screen().get_rgba_visual()
        if visual:
            self.set_visual(visual)
        self.set_app_paintable(True)
        self.connect("draw", self._clear)
        GtkLayerShell.init_for_window(self)
        GtkLayerShell.set_namespace(self, "nexa-desktop-drives")
        GtkLayerShell.set_layer(self, GtkLayerShell.Layer.BOTTOM)
        GtkLayerShell.set_keyboard_mode(self, GtkLayerShell.KeyboardMode.NONE)
        GtkLayerShell.set_exclusive_zone(self, 0)
        for edge, margin in ((GtkLayerShell.Edge.TOP, BAR_H + 12), (GtkLayerShell.Edge.LEFT, 24)):
            GtkLayerShell.set_anchor(self, edge, True)
            GtkLayerShell.set_margin(self, edge, margin)

        self.provider = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), self.provider,
                                                 Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 2)
        self._load_theme()
        self._theme_monitor = Gio.File.new_for_path(themes.settings_path()).monitor_file(Gio.FileMonitorFlags.NONE, None)
        self._theme_monitor.connect("changed", lambda *_a: self._load_theme())
        # The broker replaces the file atomically: watch its folder.
        self._monitor = Gio.File.new_for_path(os.path.dirname(STATE)).monitor_directory(Gio.FileMonitorFlags.NONE, None)
        self._monitor.connect("changed", lambda *_a: GLib.timeout_add(150, self.refresh))
        self.box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.add(self.box)
        self.shown = None
        self.refresh()
        GLib.timeout_add_seconds(30, self._tick)  # in case an event was missed

    def _clear(self, _w, cr):
        cr.set_operator(cairo.OPERATOR_CLEAR)
        cr.paint()
        cr.set_operator(cairo.OPERATOR_OVER)
        return False

    def _load_theme(self):
        themes.follow_icon_theme()
        t = themes.THEMES[themes.load_settings()["theme"]]
        self.accent = _rgb(t["accent"])
        self.provider.load_from_data(drives_css(t, themes.app_palette(t)).encode())
        if self.shown:
            self.shown = None
            self.refresh()

    def _tick(self):
        self.refresh()
        return True

    def refresh(self):
        drives = read_state()
        key = json.dumps(drives, sort_keys=True)
        if key == self.shown:
            return False
        self.shown = key
        for child in self.box.get_children():
            self.box.remove(child)
        for d in drives:
            self.box.pack_start(self._tile(d), False, False, 0)
        if drives:
            self.show_all()
        else:
            self.hide()
        return False

    def _tile(self, d):
        btn = Gtk.Button()
        btn.get_style_context().add_class("nx-drive")
        btn.set_relief(Gtk.ReliefStyle.NONE)
        btn.set_tooltip_text(f"{drive_name(d)} - click to open, right-click to eject")
        col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        col.pack_start(HexIcon("drive-removable-media", self.accent), False, False, 0)
        name = Gtk.Label(label=drive_name(d))
        name.get_style_context().add_class("nx-drive-name")
        name.set_max_width_chars(14)
        name.set_ellipsize(Pango.EllipsizeMode.END)
        col.pack_start(name, False, False, 0)
        size = Gtk.Label(label=removable.human_size(d.get("size_bytes") or 0))
        size.get_style_context().add_class("nx-drive-size")
        col.pack_start(size, False, False, 0)
        btn.add(col)
        btn.connect("clicked", lambda _b: self.open(d))
        btn.connect("button-press-event", self._on_press, d)
        return btn

    def _on_press(self, btn, event, d):
        if event.button != 3:
            return False
        menu = Gtk.Menu()
        for label, cb in (("Open", lambda *_a: self.open(d)), ("Eject", lambda *_a: self.eject(d))):
            item = Gtk.MenuItem(label=label)
            item.connect("activate", cb)
            menu.append(item)
        menu.show_all()
        menu.popup_at_pointer(event)
        return True

    def open(self, d):
        def work():
            mp = d.get("mountpoint")
            try:
                if not mp:
                    mp = removable.mount(d["name"]).get("mountpoint")
            except removable.RemovableError as e:
                GLib.idle_add(spawn, ["notify-send", "Couldn't open the drive", str(e)])
                return
            if mp:
                GLib.idle_add(spawn, ["nexa-files", mp])

        threading.Thread(target=work, daemon=True).start()

    def eject(self, d):
        name = drive_name(d)

        def work():
            try:
                removable.eject(d["name"])
                GLib.idle_add(spawn, ["notify-send", f"{name} can be unplugged now"])
            except removable.RemovableError as e:
                GLib.idle_add(spawn, ["notify-send", f"Couldn't eject {name}", str(e)])

        threading.Thread(target=work, daemon=True).start()


def main():
    Gtk.init()
    win = DesktopDrives()
    win.connect("destroy", Gtk.main_quit)
    Gtk.main()
    return 0
