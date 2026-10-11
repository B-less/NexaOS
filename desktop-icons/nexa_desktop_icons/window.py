import configparser
import glob
import shlex

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gtk, Gdk, GLib, GtkLayerShell

CSS = b"""
window { background-image: none; background-color: rgba(0, 0, 0, 0); }
#icongrid { background-image: none; background-color: rgba(0, 0, 0, 0); }
#iconbtn { background-image: none; background-color: rgba(0, 0, 0, 0); border: none; padding: 6px; border-radius: 8px; }
#iconbtn:hover { background-image: none; background-color: rgba(255, 255, 255, 0.10); }
#iconlabel { color: #ffffff; font-size: 9pt; font-weight: 600; }
menu { background-image: none; background-color: #232323; color: #e0e0e0; border: 1px solid #3c3c3c; padding: 4px; }
menuitem { color: #e0e0e0; padding: 6px 10px; border-radius: 6px; }
menuitem:hover { background-image: none; background-color: #2c3a52; color: #dbe4f5; }
"""

APPS_DIR = "/usr/share/applications"

# Not meant to be launched directly from the desktop: foot-server/footclient
# are the split halves of a client/server terminal setup ($mod+Return already
# covers "open a terminal" via plain foot), and the framebuffer desktop is
# the pre-Wayland legacy GUI, not part of the current session.
SKIP_NAMES = {"Foot Server", "Foot Client", "Nexa Framebuffer Desktop"}


def _load_apps():
    apps = []
    for path in sorted(glob.glob(f"{APPS_DIR}/*.desktop")):
        cp = configparser.ConfigParser(interpolation=None, strict=False)
        try:
            cp.read(path)
        except configparser.Error:
            continue
        if "Desktop Entry" not in cp:
            continue
        entry = cp["Desktop Entry"]
        if entry.get("Type", "Application") != "Application":
            continue
        if entry.getboolean("NoDisplay", fallback=False):
            continue
        name = entry.get("Name")
        exec_line = entry.get("Exec")
        if not name or not exec_line or name in SKIP_NAMES:
            continue
        icon = entry.get("Icon", "application-x-executable")
        apps.append({"name": name, "exec": exec_line, "icon": icon})
    return apps


def _launch(exec_line):
    # Desktop-entry Exec fields may carry %u/%f placeholders we don't
    # support (no file/url is ever passed from a desktop icon click).
    args = [a for a in shlex.split(exec_line) if not a.startswith("%")]
    GLib.spawn_async(args, flags=GLib.SpawnFlags.SEARCH_PATH | GLib.SpawnFlags.DO_NOT_REAP_CHILD)


class DesktopIconsWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Desktop")
        self._make_transparent()
        self._init_layer_shell()
        self._build_ui()

    def _make_transparent(self):
        screen = self.get_screen()
        visual = screen.get_rgba_visual()
        if visual and screen.is_composited():
            self.set_visual(visual)
        self.set_app_paintable(True)

    def _init_layer_shell(self):
        GtkLayerShell.init_for_window(self)
        GtkLayerShell.set_layer(self, GtkLayerShell.Layer.BOTTOM)
        GtkLayerShell.set_keyboard_mode(self, GtkLayerShell.KeyboardMode.NONE)
        GtkLayerShell.set_exclusive_zone(self, 0)
        for edge in (GtkLayerShell.Edge.TOP, GtkLayerShell.Edge.BOTTOM,
                     GtkLayerShell.Edge.LEFT, GtkLayerShell.Edge.RIGHT):
            GtkLayerShell.set_anchor(self, edge, True)

    # Icon button (84px) + its padding/spacing + a wrapped 2-line label,
    # rounded up a bit -- used to work out how many icons fit in a column
    # before wrapping to a new one.
    ICON_ITEM_HEIGHT = 132
    TOP_MARGIN = 40  # clears waybar's 30px height with a little breathing room
    SIDE_MARGIN = 16
    COLUMN_SPACING = 8

    def _build_ui(self):
        outer = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.COLUMN_SPACING)
        outer.set_name("icongrid")
        outer.set_halign(Gtk.Align.START)
        outer.set_valign(Gtk.Align.START)
        outer.set_margin_top(self.TOP_MARGIN)
        outer.set_margin_start(self.SIDE_MARGIN)
        self.add(outer)

        # Built as plain Box columns rather than a GtkFlowBox: a FlowBox's
        # wrapping direction turned out to depend on orientation in a way
        # that didn't match testing here (VERTICAL produced a single wide
        # row, not a wrapping column), so this lays out icons top-to-bottom
        # per column and starts a new column itself once a column is full --
        # deterministic, and never lets icons run off the bottom of the
        # screen regardless of how many apps are installed.
        max_per_column = self._max_icons_per_column()
        column = None
        for i, app in enumerate(_load_apps()):
            if i % max_per_column == 0:
                column = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
                column.set_name("icongrid")
                outer.pack_start(column, False, False, 0)
            column.pack_start(self._make_icon(app), False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _max_icons_per_column(self):
        height = 800
        display = Gdk.Display.get_default()
        if display and display.get_n_monitors() > 0:
            height = display.get_monitor(0).get_geometry().height
        else:
            screen = Gdk.Screen.get_default()
            if screen:
                height = screen.get_height()
        available = max(height - self.TOP_MARGIN - 20, self.ICON_ITEM_HEIGHT)
        return max(1, available // self.ICON_ITEM_HEIGHT)

    def _make_icon(self, app):
        btn = Gtk.Button()
        btn.set_name("iconbtn")
        btn.set_relief(Gtk.ReliefStyle.NONE)
        btn.set_size_request(84, 84)
        btn.connect("clicked", lambda *_: _launch(app["exec"]))
        btn.connect("button-press-event", self._on_icon_button_press, app)

        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        image = Gtk.Image.new_from_icon_name(app["icon"], Gtk.IconSize.DIALOG)
        box.pack_start(image, False, False, 0)
        label = Gtk.Label(label=app["name"])
        label.set_name("iconlabel")
        label.set_line_wrap(True)
        label.set_justify(Gtk.Justification.CENTER)
        label.set_max_width_chars(10)
        box.pack_start(label, False, False, 0)
        btn.add(box)
        return btn

    def _on_icon_button_press(self, _widget, event, app):
        if event.button == 3:  # right-click
            self._show_context_menu(app, event)
            return True
        return False

    def _show_context_menu(self, app, event):
        menu = Gtk.Menu()

        open_item = Gtk.MenuItem(label="Open")
        open_item.connect("activate", lambda *_a: _launch(app["exec"]))
        menu.append(open_item)

        menu.append(Gtk.SeparatorMenuItem())

        props_item = Gtk.MenuItem(label="Properties")
        props_item.connect("activate", lambda *_a: self._show_properties(app))
        menu.append(props_item)

        menu.show_all()
        menu.popup_at_pointer(event)

    def _show_properties(self, app):
        dialog = Gtk.MessageDialog(
            modal=True, buttons=Gtk.ButtonsType.CLOSE, text=app["name"],
        )
        dialog.format_secondary_text(f"Command: {app['exec']}\nIcon: {app['icon']}")
        dialog.run()
        dialog.destroy()


def main():
    Gtk.init()
    win = DesktopIconsWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
