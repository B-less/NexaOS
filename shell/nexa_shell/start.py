"""NexaOS start menu: search, pinned apps, all apps, power options."""
import socket

from gi.repository import GLib, Gtk, Pango

from nexa_common import desktopapps
from .panel import Panel, power, run_panel

# Shown first, in this order, when they are installed.
PINNED = ["Files", "Nexa Browser", "Foot", "Settings", "Nexa Software", "NexaOS Assistant",
          "Nexa Notes", "Music", "Editor", "Calculator", "Nexa System Monitor", "Screenshot"]
PINNED_COLUMNS = 6


class StartMenu(Panel):
    ANCHOR = "left"
    WIDTH = 640

    def __init__(self, app):
        super().__init__(app)
        self.apps = desktopapps.load_apps()

        head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        head.pack_start(self.label("NexaOS", "title"), False, False, 0)
        head.pack_end(self.label(self._greeting(), "muted", xalign=1.0), False, False, 0)
        self.body.pack_start(head, False, False, 0)

        self.search = Gtk.SearchEntry()
        self.search.set_placeholder_text("Search apps")
        self.search.connect("search-changed", lambda *_a: self._refresh())
        self.search.connect("activate", self._launch_first)
        self.body.pack_start(self.search, False, False, 0)

        self.pinned_title = self.label("PINNED", "section")
        self.body.pack_start(self.pinned_title, False, False, 0)
        self.pinned = Gtk.FlowBox()
        self.pinned.set_selection_mode(Gtk.SelectionMode.NONE)
        self.pinned.set_max_children_per_line(PINNED_COLUMNS)
        self.pinned.set_min_children_per_line(PINNED_COLUMNS)
        self.pinned.set_homogeneous(True)
        for app in self._pinned_apps():
            self.pinned.add(self._tile(app))
        self.body.pack_start(self.pinned, False, False, 0)

        self.list_title = self.label("ALL APPS", "section")
        self.body.pack_start(self.list_title, False, False, 0)
        scroller = Gtk.ScrolledWindow()
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroller.set_min_content_height(260)
        scroller.set_max_content_height(260)
        self.results = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        scroller.add(self.results)
        self.body.pack_start(scroller, True, True, 0)

        self.body.pack_start(Gtk.Separator(), False, False, 0)
        foot = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        foot.pack_start(self.label(f"user @ {socket.gethostname()}", "muted"), False, False, 0)
        # pack_end fills right-to-left, so this reads Lock, Sleep, Restart, Shut down.
        for text, action in (("Shut down", "poweroff"), ("Restart", "reboot"), ("Sleep", "suspend"), ("Lock", "lock")):
            btn = Gtk.Button(label=text)
            btn.set_name("power")
            btn.connect("clicked", lambda _b, a=action: self._power(a))
            foot.pack_end(btn, False, False, 0)
        self.body.pack_start(foot, False, False, 0)

        self._refresh()
        GLib.idle_add(self.search.grab_focus)

    def _greeting(self):
        hour = GLib.DateTime.new_now_local().get_hour()
        return "Good morning" if hour < 12 else "Good afternoon" if hour < 18 else "Good evening"

    def _pinned_apps(self):
        by_name = {a["name"]: a for a in self.apps}
        pinned = [by_name[n] for n in PINNED if n in by_name]
        return pinned or self.apps[:PINNED_COLUMNS * 2]

    def _tile(self, app):
        btn = Gtk.Button()
        btn.set_tooltip_text(app["comment"] or app["name"])
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        box.pack_start(Gtk.Image.new_from_icon_name(app["icon"], Gtk.IconSize.DND), False, False, 0)
        name = Gtk.Label(label=app["name"])
        name.set_ellipsize(Pango.EllipsizeMode.END)
        name.set_max_width_chars(11)
        box.pack_start(name, False, False, 0)
        btn.add(box)
        btn.connect("clicked", lambda *_a: self._launch(app))
        return btn

    def _row(self, app):
        btn = Gtk.Button()
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        row.pack_start(Gtk.Image.new_from_icon_name(app["icon"], Gtk.IconSize.LARGE_TOOLBAR), False, False, 0)
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        text.pack_start(self.label(app["name"]), False, False, 0)
        if app["comment"]:
            c = self.label(app["comment"], "muted")
            c.set_ellipsize(Pango.EllipsizeMode.END)
            text.pack_start(c, False, False, 0)
        row.pack_start(text, True, True, 0)
        btn.add(row)
        btn.connect("clicked", lambda *_a: self._launch(app))
        return btn

    def _refresh(self):
        query = self.search.get_text().strip()
        searching = bool(query)
        self.pinned_title.set_visible(not searching)
        self.pinned.set_visible(not searching)
        self.list_title.set_text("RESULTS" if searching else "ALL APPS")
        for child in self.results.get_children():
            self.results.remove(child)
        self.matches = desktopapps.search(self.apps, query)
        if not self.matches:
            self.results.pack_start(self.label(f'No apps match "{query}".', "muted"), False, False, 8)
        for app in self.matches:
            self.results.pack_start(self._row(app), False, False, 0)
        self.results.show_all()

    def _launch_first(self, *_a):
        if getattr(self, "matches", None):
            self._launch(self.matches[0])

    def _launch(self, app):
        desktopapps.launch(app["exec"])
        self.close()

    def _power(self, action):
        power(action)
        self.close()


def main():
    return run_panel("os.nexa.StartMenu", StartMenu)
