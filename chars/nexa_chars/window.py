import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#searchentry { background-image: none; background-color: @nexa_field; color: @nexa_text_strong; border-radius: 14px; padding: 6px 12px; border: 1px solid @nexa_border; }
#charbtn { background-image: none; background-color: @nexa_field; border-radius: 8px; font-size: 20pt; padding: 6px; }
#charbtn:hover { background-image: none; background-color: @nexa_hover; }
#statusbar { background-image: none; background-color: @nexa_surface; color: @nexa_ok; font-size: 9pt; }
"""

EMOJI = [
    ("😀", "grinning face"), ("😂", "joy"), ("😍", "heart eyes"), ("😎", "cool"),
    ("🥳", "party"), ("😢", "cry"), ("😡", "angry"), ("😱", "scream"),
    ("🤔", "thinking"), ("😴", "sleep"), ("🙂", "slight smile"), ("😉", "wink"),
    ("👍", "thumbs up"), ("👎", "thumbs down"), ("👏", "clap"), ("🙌", "raise hands"),
    ("🙏", "pray"), ("💪", "muscle"), ("👋", "wave"), ("✌️", "peace"),
    ("👌", "ok hand"), ("🤝", "handshake"), ("❤️", "heart"), ("💔", "broken heart"),
    ("⭐", "star"), ("✨", "sparkles"), ("🔥", "fire"), ("💯", "hundred"),
    ("🎉", "party popper"), ("🎂", "cake"), ("🎁", "gift"), ("🏆", "trophy"),
    ("☀️", "sun"), ("🌙", "moon"), ("☁️", "cloud"), ("⚡", "lightning"),
    ("🌈", "rainbow"), ("❄️", "snowflake"), ("💧", "droplet"), ("🌊", "wave water"),
    ("🐶", "dog"), ("🐱", "cat"), ("🐭", "mouse"), ("🦊", "fox"),
    ("🐼", "panda"), ("🐵", "monkey"), ("🐔", "chicken"), ("🐦", "bird"),
    ("🍎", "apple"), ("🍕", "pizza"), ("🍔", "burger"), ("🍟", "fries"),
    ("☕", "coffee"), ("🍺", "beer"), ("🍩", "donut"), ("🍫", "chocolate"),
    ("⚽", "soccer"), ("🏀", "basketball"), ("🎮", "video game"), ("🎵", "music note"),
    ("📱", "phone"), ("💻", "laptop"), ("🖥️", "desktop"), ("⌨️", "keyboard"),
    ("📷", "camera"), ("🔋", "battery"), ("💡", "bulb"), ("🔒", "lock"),
    ("🔓", "unlock"), ("🔑", "key"), ("📁", "folder"), ("📄", "document"),
    ("✅", "check mark"), ("❌", "cross mark"), ("⚠️", "warning"), ("❓", "question"),
    ("❗", "exclamation"), ("➕", "plus"), ("➖", "minus"), ("🔄", "refresh"),
    ("🚀", "rocket"), ("✈️", "airplane"), ("🚗", "car"), ("🚲", "bicycle"),
    ("🏠", "house"), ("🏢", "building"), ("🌍", "globe"), ("🗺️", "map"),
]


class CharsWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa Characters")
        self.set_icon_name("nexa-chars")
        attach_titlebar(self, "Nexa Characters")
        self.set_default_size(560, 480)
        self._build_ui()
        self._populate("")

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        self.search = Gtk.Entry()
        self.search.set_name("searchentry")
        self.search.set_placeholder_text("Search…")
        self.search.connect("changed", self._on_search_changed)
        toolbar.pack_start(self.search, True, True, 0)
        root.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.flow = Gtk.FlowBox()
        self.flow.set_selection_mode(Gtk.SelectionMode.NONE)
        self.flow.set_valign(Gtk.Align.START)
        self.flow.set_homogeneous(True)
        scroller.add(self.flow)
        root.pack_start(scroller, True, True, 0)

        self.status = Gtk.Label(label="Click a character to copy it")
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

    def _populate(self, query):
        for child in self.flow.get_children():
            self.flow.remove(child)
        query = query.lower().strip()
        for char, name in EMOJI:
            if query and query not in name:
                continue
            btn = Gtk.Button(label=char)
            btn.set_name("charbtn")
            btn.set_tooltip_text(name)
            btn.connect("clicked", self._on_pick, char, name)
            self.flow.add(btn)
        self.flow.show_all()

    def _on_search_changed(self, entry):
        self._populate(entry.get_text())

    def _on_pick(self, _btn, char, name):
        clipboard = Gtk.Clipboard.get_default(Gdk.Display.get_default())
        clipboard.set_text(char, -1)
        clipboard.store()
        self.status.set_text(f"Copied {char} ({name})")


def main():
    Gtk.init()
    win = CharsWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
