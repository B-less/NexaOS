"""FileTrail's window in Files ("Where did it go?"); see trail.py."""
from gi.repository import GLib, Gtk, Pango

from nexa_common.titlebar import attach_titlebar

from . import engine
from .trail import describe, place

CSS = b"""
#trail { background-image: none; background-color: @nexa_bg; }
#trail list, #trail list row { background-image: none; background-color: transparent; }
#trail-row { background-image: none; background-color: @nexa_surface; border-radius: 10px; }
#trail-intro, #trail-status { color: @nexa_muted; }
#trail-name { color: @nexa_text; }
#trail-what { color: @nexa_text; }
#trail-now { color: @nexa_muted; font-size: 9pt; }
"""


class TrailWindow(Gtk.Window):
    def __init__(self, parent, show_cb, after_change):
        super().__init__(title="Where did it go?", transient_for=parent, modal=False)
        attach_titlebar(self, "Where did it go?")
        self.set_name("trail")
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(self.get_screen(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 2)
        self.set_default_size(640, 520)
        self.show_cb, self.after_change = show_cb, after_change
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        box.set_border_width(12)
        self.add(box)
        intro = Gtk.Label(xalign=0, wrap=True)
        intro.set_name("trail-intro")
        intro.set_text("Everything Files moved, renamed, copied or put in the Trash, and where it is now. "
                       "Only names and places are kept, never what's inside; switch it off in "
                       "Settings > Magic Layer.")
        box.pack_start(intro, False, False, 0)
        self.search = Gtk.SearchEntry(placeholder_text="Part of a name (“report”)")
        self.search.connect("search-changed", lambda *_a: self._schedule())
        box.pack_start(self.search, False, False, 0)
        self.list = Gtk.ListBox()
        self.list.set_selection_mode(Gtk.SelectionMode.NONE)
        self.list.set_name("trail-list")
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.add(self.list)
        box.pack_start(scroll, True, True, 0)
        bottom = Gtk.Box(spacing=8)
        self.status = Gtk.Label(xalign=0)
        self.status.set_name("trail-status")
        bottom.pack_start(self.status, True, True, 0)
        forget = Gtk.Button(label="Forget all of it")
        forget.set_name("toolbtn")
        forget.connect("clicked", self._on_forget)
        bottom.pack_end(forget, False, False, 0)
        box.pack_start(bottom, False, False, 0)
        self._pending = 0
        self.items = []
        self.reload()

    def _schedule(self):
        if self._pending:
            GLib.source_remove(self._pending)
        self._pending = GLib.timeout_add(200, self._fire)

    def _fire(self):
        self._pending = 0
        self.reload()
        return False

    def reload(self):
        for row in self.list.get_children():
            row.destroy()
        try:
            self.items = engine.trail(self.search.get_text().strip(), 300)
        except engine.EngineError as e:
            self.items = []
            self.status.set_text(str(e))
            return
        if not self.items:
            self.status.set_text("Nothing yet." if not self.search.get_text().strip() else "Nothing by that name.")
        else:
            self.status.set_text(f"{len(self.items)} change(s)")
        for item in self.items:
            self.list.add(self._row(item))
        self.list.show_all()

    def _row(self, item):
        row = Gtk.Box(spacing=10)
        row.set_name("trail-row")
        row.set_border_width(10)
        row.set_margin_bottom(6)
        col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        name = Gtk.Label(xalign=0)
        name.set_markup(f"<b>{GLib.markup_escape_text(item['name'])}</b>")
        name.set_name("trail-name")
        name.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
        what, now = describe(item)
        col.pack_start(name, False, False, 0)
        for text, style in ((what, "trail-what"), (now, "trail-now")):
            lab = Gtk.Label(label=text, xalign=0, wrap=True)
            lab.set_name(style)
            col.pack_start(lab, False, False, 0)
        row.pack_start(col, True, True, 0)
        if item.get("status") == "here":
            show = Gtk.Button(label="Show")
            show.set_name("toolbtn")
            show.set_valign(Gtk.Align.CENTER)
            show.connect("clicked", lambda _b, p=item["now"]: self.show_cb(p))
            row.pack_end(show, False, False, 0)
        if item.get("put_back"):
            back = Gtk.Button(label="Put it back")
            back.set_name("toolbtn")
            back.set_valign(Gtk.Align.CENTER)
            back.set_tooltip_text(f"Back to {place(item['from'] or item['to'])}")
            back.connect("clicked", lambda _b, i=item: self._put_back(i))
            row.pack_end(back, False, False, 0)
        return row

    def _put_back(self, item):
        try:
            r = engine.undo([item["put_back"]])
            self.status.set_text(r.get("done", "Done"))
        except engine.EngineError as e:
            self.status.set_text(str(e))
        self.after_change()
        self.reload()

    def _on_forget(self, _b):
        d = Gtk.MessageDialog(transient_for=self, modal=True, message_type=Gtk.MessageType.QUESTION,
                              buttons=Gtk.ButtonsType.OK_CANCEL,
                              text="Forget every change FileTrail has kept?")
        d.format_secondary_text("Your files stay where they are; only this list is cleared.")
        ok = d.run() == Gtk.ResponseType.OK
        d.destroy()
        if ok:
            try:
                engine.trail_forget()
            except engine.EngineError as e:
                self.status.set_text(str(e))
            self.reload()
