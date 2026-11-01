"""The PDF Viewer: all pages in one scroll, fit width / fit page / zoom,
page thumbnails and contents in a sidebar, search with every match marked,
go to page, links, selecting and copying text, turning, dark reading mode,
presentation, passwords, and each document opening where it was left.

Details, contents, search, thumbnails and page text come from nexa-pdfdoc
(C), a confined process of its own: a big PDF never freezes the window,
and a PDF that makes it stop is refused before the window opens it. The
pages on screen are drawn here with poppler."""
import json
import math
import os
import sys
import threading
import time
from collections import OrderedDict

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Poppler", "0.18")
gi.require_version("PangoCairo", "1.0")
from gi.repository import Gtk, Gdk, Gio, GLib, Pango, PangoCairo, Poppler
import cairo

from nexa_common.titlebar import attach_titlebar

from . import engine

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar, #findbar { background-image: none; background-color: @nexa_surface; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 10px; padding: 4px 10px; border: none; box-shadow: none; }
#toolbtn:hover { background-color: @nexa_hover; }
#toolbtn:checked { background-color: @nexa_selected; color: @nexa_selected_text; }
#toolbtn:disabled { color: @nexa_disabled; }
#pagelabel, #zoomlabel { color: @nexa_muted; font-size: 9pt; }
#zoomlabel { font-family: "JetBrains Mono", monospace; min-width: 44px; }
#findbar entry, #pageentry { background-image: none; background-color: @nexa_field; color: @nexa_text; border: 1px solid @nexa_border; border-radius: 6px; }
#findcount { color: @nexa_muted; font-size: 9pt; }
#findcount.bad { color: @nexa_crit; }
#sidebar { background-image: none; background-color: @nexa_surface; }
#sidebar list, #sidebar row { background-color: transparent; }
#sidebar row:selected { background-color: @nexa_selected; border-radius: 10px; }
#sidebar treeview { background-color: @nexa_surface; color: @nexa_text; }
#sidebar treeview:selected { background-color: @nexa_selected; color: @nexa_selected_text; }
#sidebar .pnum { color: @nexa_muted; font-size: 8.5pt; }
#sidebar stackswitcher button { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border: none; box-shadow: none; }
#sidebar stackswitcher button:checked { background-color: @nexa_selected; color: @nexa_selected_text; }
#sideempty { color: @nexa_muted; }
"""

GAP, MARGIN = 12, 16
ZOOM_STEP = 1.2
MIN_SCALE, MAX_SCALE = 0.1, 8.0
CACHE_PAGES = 14
THUMB_WIDTH = 120


def config_path():
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "nexa-pdf", "settings.json")


def load_settings():
    s = {"dark": False, "sidebar": True, "docs": {}, "recent": []}
    try:
        with open(config_path()) as f:
            s.update({k: v for k, v in json.load(f).items() if k in s})
    except (OSError, ValueError):
        pass
    return s


def save_settings(s):
    p = config_path()
    try:
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p + ".tmp", "w") as f:
            json.dump(s, f)
        os.replace(p + ".tmp", p)
    except OSError:
        pass


def cache_dir(path):
    import hashlib
    try:
        st = os.stat(path)
        key = f"{path}\0{st.st_mtime_ns}\0{st.st_size}"
    except OSError:
        key = path
    base = os.environ.get("XDG_CACHE_HOME") or os.path.join(os.path.expanduser("~"), ".cache")
    return os.path.join(base, "nexa-pdf", "thumbs", hashlib.sha1(key.encode()).hexdigest()[:20])


class PdfWindow(Gtk.Window):
    def __init__(self, path=None):
        super().__init__(title="PDF")
        self.set_icon_name("nexa-pdf")
        attach_titlebar(self, "PDF")
        self.set_default_size(1000, 760)
        self.settings = load_settings()
        self.path = None
        self.password = None
        self.doc = None
        self.meta = {}
        self.sizes = []
        self.rects = []
        self.mode = "width"
        self.scale = 1.0
        self.rotation = 0
        self.cache = OrderedDict()
        self.links = {}
        self.matches = []
        self.match_index = -1
        self.search_proc = None
        self.search_gen = 0
        self.sel = None
        self.sel_rects = []
        self.sel_text = ""
        self.press = None
        self.present = False
        self.present_page = 0
        self.current = 0
        self.thumb_gen = 0
        self.save_timer = 0
        self.problem = None

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self._build_ui()
        self.connect("key-press-event", self._on_key)
        self.connect("delete-event", lambda *_a: self._remember() or False)
        self.drag_dest_set(Gtk.DestDefaults.ALL, [], Gdk.DragAction.COPY)
        self.drag_dest_add_uri_targets()
        self.connect("drag-data-received", self._on_drop)
        if path:
            GLib.idle_add(lambda: self.open_path(os.path.abspath(path)) and False)
        self._sync()

    # ---------- building ----------

    def _btn(self, label, tip, fn, toggle=False):
        b = Gtk.ToggleButton(label=label) if toggle else Gtk.Button(label=label)
        b.set_name("toolbtn")
        b.set_tooltip_text(tip)
        b.connect("toggled" if toggle else "clicked", lambda *_a: fn())
        return b

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.add(root)
        t = Gtk.Box(spacing=6)
        t.set_name("toolbar")
        t.set_border_width(8)
        self.toolbar = t
        t.pack_start(self._btn("Open", "Open a PDF (Ctrl+O)", self.on_open), False, False, 0)
        self.side_btn = self._btn("Pages", "Pages and contents (F9)", self._toggle_sidebar, toggle=True)
        t.pack_start(self.side_btn, False, False, 0)
        t.pack_start(Gtk.Separator(orientation=Gtk.Orientation.VERTICAL), False, False, 4)
        self.prev_btn = self._btn("‹", "Previous page", lambda: self.go_to(self.current - 1))
        self.page_entry = Gtk.Entry()
        self.page_entry.set_name("pageentry")
        self.page_entry.set_width_chars(4)
        self.page_entry.set_alignment(1.0)
        self.page_entry.set_tooltip_text("Go to page (Ctrl+G)")
        self.page_entry.connect("activate", self._on_page_entry)
        self.page_label = Gtk.Label()
        self.page_label.set_name("pagelabel")
        self.next_btn = self._btn("›", "Next page", lambda: self.go_to(self.current + 1))
        for w in (self.prev_btn, self.page_entry, self.page_label, self.next_btn):
            t.pack_start(w, False, False, 0)
        t.pack_start(Gtk.Separator(orientation=Gtk.Orientation.VERTICAL), False, False, 4)
        self.zoom_out_btn = self._btn("−", "Zoom out (Ctrl −, or Ctrl and the wheel)", lambda: self.zoom_by(1 / ZOOM_STEP))
        self.zoom_label = Gtk.Label()
        self.zoom_label.set_name("zoomlabel")
        self.zoom_in_btn = self._btn("+", "Zoom in (Ctrl +)", lambda: self.zoom_by(ZOOM_STEP))
        self.width_btn = self._btn("Width", "Fit the width (Ctrl+0)", lambda: self.set_mode("width"))
        self.whole_btn = self._btn("Page", "Fit the whole page", lambda: self.set_mode("page"))
        self.turn_btn = self._btn("⟳", "Turn the pages (Ctrl+R)", self.turn)
        self.find_btn = self._btn("Find", "Find (Ctrl+F)", self.show_find)
        self.more_btn = self._btn("More", "Dark mode, presentation, details, recent", lambda: self._pop(self.more_btn, self._more_menu()))
        for w in (self.zoom_out_btn, self.zoom_label, self.zoom_in_btn, self.width_btn, self.whole_btn, self.turn_btn, self.find_btn, self.more_btn):
            t.pack_start(w, False, False, 0)
        root.pack_start(t, False, False, 0)

        body = Gtk.Box()
        self.side_revealer = Gtk.Revealer()
        self.side_revealer.set_transition_type(Gtk.RevealerTransitionType.SLIDE_RIGHT)
        side = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        side.set_name("sidebar")
        side.set_border_width(8)
        side.set_size_request(200, -1)
        self.side_stack = Gtk.Stack()
        switcher = Gtk.StackSwitcher()
        switcher.set_stack(self.side_stack)
        switcher.set_halign(Gtk.Align.CENTER)
        side.pack_start(switcher, False, False, 0)
        tscroll = Gtk.ScrolledWindow()
        tscroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.thumb_list = Gtk.ListBox()
        self.thumb_list.connect("row-activated", lambda _l, row: self.go_to(row.page))
        tscroll.add(self.thumb_list)
        self.thumb_scroll = tscroll
        self.side_stack.add_titled(tscroll, "pages", "Pages")
        self.toc_store = Gtk.TreeStore(str, int)
        self.toc = Gtk.TreeView(model=self.toc_store)
        self.toc.set_headers_visible(False)
        col = Gtk.TreeViewColumn("", Gtk.CellRendererText(ellipsize=Pango.EllipsizeMode.END), text=0)
        col.set_expand(True)
        self.toc.append_column(col)
        num = Gtk.CellRendererText()
        num.set_property("xalign", 1.0)
        self.toc.append_column(Gtk.TreeViewColumn("", num, text=1))
        self.toc.connect("row-activated", lambda tv, p, _c: self.go_to(self.toc_store[p][1] - 1))
        cscroll = Gtk.ScrolledWindow()
        cscroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.toc_stack = Gtk.Stack()
        cscroll.add(self.toc)
        none = Gtk.Label(label="This PDF has no table of contents.")
        none.set_name("sideempty")
        none.set_line_wrap(True)
        self.toc_stack.add_named(cscroll, "toc")
        self.toc_stack.add_named(none, "none")
        self.side_stack.add_titled(self.toc_stack, "contents", "Contents")
        side.pack_start(self.side_stack, True, True, 0)
        self.side_revealer.add(side)
        body.pack_start(self.side_revealer, False, False, 0)

        self.scroller = Gtk.ScrolledWindow()
        self.canvas = Gtk.DrawingArea()
        # Never focused: GTK would scroll the view to its top. Keys go to the window.
        self.canvas.set_can_focus(False)
        self.canvas.connect("size-allocate", lambda *_a: self._apply_pending())
        self.canvas.add_events(Gdk.EventMask.SCROLL_MASK | Gdk.EventMask.SMOOTH_SCROLL_MASK | Gdk.EventMask.BUTTON_PRESS_MASK
                               | Gdk.EventMask.BUTTON_RELEASE_MASK | Gdk.EventMask.POINTER_MOTION_MASK)
        self.canvas.connect("draw", self._on_draw)
        self.canvas.connect("button-press-event", self._on_press)
        self.canvas.connect("button-release-event", self._on_release)
        self.canvas.connect("motion-notify-event", self._on_motion)
        self.scroller.connect("scroll-event", self._on_scroll)
        self.scroller.add(self.canvas)
        self.scroller.connect("size-allocate", self._on_resize)
        self.scroller.get_vadjustment().connect("value-changed", lambda *_a: self._on_scrolled())
        body.pack_start(self.scroller, True, True, 0)
        root.pack_start(body, True, True, 0)

        self.find_revealer = Gtk.Revealer()
        fb = Gtk.Box(spacing=8)
        fb.set_name("findbar")
        fb.set_border_width(8)
        self.find_entry = Gtk.SearchEntry()
        self.find_entry.set_placeholder_text("Find in the document")
        self.find_entry.set_width_chars(28)
        self.find_entry.connect("activate", lambda *_a: self.find_next(1) if self._search_key() == getattr(self, "_searched", None) else self.search())
        fb.pack_start(self.find_entry, False, False, 0)
        fb.pack_start(self._btn("‹", "Previous match (Shift+F3)", lambda: self.find_next(-1)), False, False, 0)
        fb.pack_start(self._btn("›", "Next match (F3 or Enter)", lambda: self.find_next(1)), False, False, 0)
        self.case_btn = self._btn("Aa", "Match upper and lower case", lambda: None, toggle=True)
        self.word_btn = self._btn("Word", "Whole words only", lambda: None, toggle=True)
        fb.pack_start(self.case_btn, False, False, 0)
        fb.pack_start(self.word_btn, False, False, 0)
        self.find_count = Gtk.Label()
        self.find_count.set_name("findcount")
        fb.pack_start(self.find_count, False, False, 4)
        fb.pack_end(self._btn("Close", "Close (Esc)", self.hide_find), False, False, 0)
        self.find_revealer.add(fb)
        root.pack_start(self.find_revealer, False, False, 0)
        self.side_btn.set_active(bool(self.settings.get("sidebar")))

    def _pop(self, widget, menu):
        menu.attach_to_widget(widget, None)
        menu.connect("deactivate", lambda m: GLib.idle_add(m.destroy))
        menu.popup_at_widget(widget, Gdk.Gravity.SOUTH_WEST, Gdk.Gravity.NORTH_WEST, None)

    def _more_menu(self):
        m = Gtk.Menu()
        have = self.doc is not None

        def item(label, fn, ok=True):
            it = Gtk.MenuItem(label=label)
            it.set_sensitive(ok)
            it.connect("activate", lambda *_a: fn())
            m.append(it)
        dark = Gtk.CheckMenuItem(label="Dark Reading Mode")
        dark.set_active(bool(self.settings.get("dark")))
        dark.connect("toggled", lambda w: self.set_dark(w.get_active()))
        m.append(dark)
        item("Presentation  (F5)", self.start_presentation, have)
        item("Full Screen  (F11)", self.toggle_fullscreen, have)
        m.append(Gtk.SeparatorMenuItem())
        item("Copy Selected Text  (Ctrl+C)", self.copy, bool(self.sel_text))
        item("Copy This Page's Text", self.copy_page_text, have)
        item("Document Details", self.show_details, have)
        m.append(Gtk.SeparatorMenuItem())
        recent = Gtk.MenuItem(label="Recent Documents")
        sub = Gtk.Menu()
        files = [p for p in self.settings.get("recent") or [] if isinstance(p, str)]
        for p in files:
            it = Gtk.MenuItem(label=p.replace(os.path.expanduser("~"), "~", 1))
            it.connect("activate", lambda _w, p=p: self.open_path(p))
            sub.append(it)
        if not files:
            empty = Gtk.MenuItem(label="Nothing opened yet")
            empty.set_sensitive(False)
            sub.append(empty)
        recent.set_submenu(sub)
        m.append(recent)
        m.show_all()
        return m

    # ---------- opening ----------

    def on_open(self):
        dialog = Gtk.FileChooserDialog(title="Open a PDF", transient_for=self, action=Gtk.FileChooserAction.OPEN)
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Open", Gtk.ResponseType.OK)
        f = Gtk.FileFilter()
        f.set_name("PDF documents")
        f.add_mime_type("application/pdf")
        f.add_pattern("*.pdf")
        f.add_pattern("*.PDF")
        dialog.add_filter(f)
        folder = os.path.dirname(self.path) if self.path else os.path.join(os.path.expanduser("~"), "Documents")
        if os.path.isdir(folder):
            dialog.set_current_folder(folder)
        path = dialog.get_filename() if dialog.run() == Gtk.ResponseType.OK else None
        dialog.destroy()
        if path:
            self.open_path(path)

    def ask_password(self, path, wrong):
        """The PDF's password, or None if the person gave up."""
        d = Gtk.Dialog(title="Locked PDF", transient_for=self, modal=True)
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Open", Gtk.ResponseType.OK)
        d.set_default_response(Gtk.ResponseType.OK)
        box = d.get_content_area()
        box.set_spacing(8)
        box.set_border_width(16)
        msg = Gtk.Label(label=("That password isn't right. Try again." if wrong else f"{os.path.basename(path)} is locked. Type its password."))
        msg.set_xalign(0)
        entry = Gtk.Entry()
        entry.set_visibility(False)
        entry.set_activates_default(True)
        box.pack_start(msg, False, False, 0)
        box.pack_start(entry, False, False, 0)
        d.show_all()
        r = d.run()
        text = entry.get_text()
        d.destroy()
        return text if r == Gtk.ResponseType.OK else None

    def open_path(self, path, password=None):
        self._remember()
        wrong = False
        while True:
            try:
                meta = engine.info(path, password)
                break
            except engine.EngineError as e:
                if e.code == "password":
                    password = self.ask_password(path, wrong)
                    wrong = True
                    if password is None:
                        return False
                    continue
                if e.code == "crashed":
                    self.error(f"{os.path.basename(path)} can't be shown", "It made the PDF engine stop: it may be damaged, or made to cause harm. It wasn't opened.")
                else:
                    self.error(f"{os.path.basename(path)} can't be shown", str(e))
                return False
        try:
            doc = Poppler.Document.new_from_file(Gio.File.new_for_path(path).get_uri(), password)
        except GLib.Error as e:
            self.error(f"{os.path.basename(path)} can't be shown", e.message)
            return False
        self._cancel_search()
        self.path, self.password, self.doc, self.meta = path, password, doc, meta
        self.sizes = [tuple(s) for s in meta["sizes"]] or [(612, 792)]
        self.cache.clear()
        self.links.clear()
        self.matches, self.match_index = [], -1
        self.sel, self.sel_text = None, ""
        self.find_count.set_text("")
        saved = (self.settings.get("docs") or {}).get(path) or {}
        self.mode = saved.get("mode") if saved.get("mode") in ("width", "page", "custom") else "width"
        self.scale = float(saved.get("scale") or 1.0)
        self.rotation = saved.get("rotation") if saved.get("rotation") in (0, 90, 180, 270) else 0
        title = meta.get("title") or os.path.basename(path)
        self.set_title(f"{title} — PDF")
        bar = self.get_titlebar()
        if isinstance(bar, Gtk.HeaderBar):
            bar.set_title(f"{title} — PDF")
        self._fill_toc(meta.get("outline") or [])
        self._fill_thumbs()
        self._relayout()
        page = int(saved.get("page") or 1) - 1
        GLib.idle_add(lambda: self.go_to(page if 0 <= page < len(self.sizes) else 0, smooth=False) and False)
        recent = [p for p in self.settings.get("recent") or [] if p != path]
        self.settings["recent"] = [path] + recent[:9]
        save_settings(self.settings)
        try:
            Gtk.RecentManager.get_default().add_item(Gio.File.new_for_path(path).get_uri())
        except GLib.Error:
            pass
        self._sync()
        return True

    def _remember(self):
        if not self.path:
            return
        docs = self.settings.setdefault("docs", {})
        docs[self.path] = {"page": self.current + 1, "mode": self.mode, "scale": round(self.scale, 3), "rotation": self.rotation}
        if len(docs) > 200:
            for k in list(docs)[:len(docs) - 200]:
                docs.pop(k, None)
        save_settings(self.settings)

    def _sync(self):
        have = self.doc is not None
        n = len(self.sizes) if have else 0
        for w in (self.zoom_out_btn, self.zoom_in_btn, self.width_btn, self.whole_btn, self.turn_btn, self.find_btn, self.page_entry):
            w.set_sensitive(have)
        self.prev_btn.set_sensitive(have and self.current > 0)
        self.next_btn.set_sensitive(have and self.current < n - 1)
        self.page_label.set_text(f"of {n}" if have else "")
        if have and not self.page_entry.has_focus():
            self.page_entry.set_text(str(self.current + 1))
        self.side_revealer.set_reveal_child(self.side_btn.get_active() and have and not self.present)

    # ---------- layout ----------

    def _page_size(self, i):
        w, h = self.sizes[i]
        return (h, w) if self.rotation in (90, 270) else (w, h)

    def _fit_scale(self, mode):
        vw = max(self.scroller.get_allocated_width() - 2 * MARGIN - 16, 50)
        vh = max(self.scroller.get_allocated_height() - 2 * MARGIN, 50)
        widest = max(self._page_size(i)[0] for i in range(len(self.sizes)))
        if mode == "width":
            return vw / widest
        w, h = self._page_size(self.current)
        return min(vw / w, vh / h)

    def _relayout(self, keep=True):
        if not self.doc:
            self.canvas.set_size_request(-1, -1)
            self.canvas.queue_draw()
            return
        anchor = self._anchor() if keep and self.rects else None
        if self.mode in ("width", "page"):
            self.scale = self._fit_scale(self.mode)
        self.scale = max(MIN_SCALE, min(MAX_SCALE, self.scale))
        widest = max(self._page_size(i)[0] for i in range(len(self.sizes))) * self.scale
        total_w = max(self.scroller.get_allocated_width() - 2, widest + 2 * MARGIN)
        y, rects = MARGIN, []
        for i in range(len(self.sizes)):
            w, h = self._page_size(i)
            pw, ph = w * self.scale, h * self.scale
            rects.append(((total_w - pw) / 2, y, pw, ph))
            y += ph + GAP
        self.rects = rects
        self.canvas.set_size_request(int(total_w), int(y - GAP + MARGIN))
        self.zoom_label.set_text(f"{round(self.scale * 100)}%")
        if anchor:
            GLib.idle_add(self._restore_anchor, anchor)
        self.canvas.queue_draw()

    def _anchor(self):
        """(page, how far down it) at the top of the view."""
        top = self.scroller.get_vadjustment().get_value()
        for i, (_x, y, _w, h) in enumerate(self.rects):
            if y + h + GAP > top:
                return i, max(0.0, (top - y) / h)
        return 0, 0.0

    def _restore_anchor(self, anchor):
        i, frac = anchor
        if i < len(self.rects):
            _x, y, _w, h = self.rects[i]
            self._scroll_to(y + frac * h)
        return False

    def _scroll_to(self, y):
        """Scroll so `y` is at the top; if the view hasn't grown to its new
        size yet, again once it has."""
        self._pending = y
        self._apply_pending()

    def _apply_pending(self):
        y = getattr(self, "_pending", None)
        if y is None:
            return
        adj = self.scroller.get_vadjustment()
        adj.set_value(y)
        if adj.get_upper() - adj.get_page_size() >= y - 1 or self.canvas.get_allocated_height() >= self.canvas.get_size_request()[1]:
            self._pending = None

    def _on_resize(self, *_a):
        size = (self.scroller.get_allocated_width(), self.scroller.get_allocated_height())
        if size != getattr(self, "_last_size", None):
            self._last_size = size
            if self.mode != "custom" or self.present:
                GLib.idle_add(lambda: self._relayout() and False)

    def set_mode(self, mode):
        if self.doc:
            self.mode = mode
            self._relayout()
            self._schedule_remember()

    def zoom_by(self, factor):
        if self.doc:
            self.mode = "custom"
            self.scale = max(MIN_SCALE, min(MAX_SCALE, self.scale * factor))
            self._relayout()
            self._schedule_remember()

    def turn(self):
        if self.doc:
            self.rotation = (self.rotation + 90) % 360
            self.cache.clear()
            self._relayout()
            self._schedule_remember()

    def set_dark(self, on):
        self.settings["dark"] = on
        save_settings(self.settings)
        self.cache.clear()
        self.canvas.queue_draw()

    # ---------- page <-> screen ----------

    def to_screen(self, i, x, y):
        w, h = self.sizes[i]
        r = self.rotation
        u, v = (x, y) if r == 0 else (h - y, x) if r == 90 else (w - x, h - y) if r == 180 else (y, w - x)
        rx, ry, _rw, _rh = self.rects[i]
        return rx + u * self.scale, ry + v * self.scale

    def to_page(self, i, sx, sy):
        rx, ry, _rw, _rh = self.rects[i]
        u, v = (sx - rx) / self.scale, (sy - ry) / self.scale
        w, h = self.sizes[i]
        r = self.rotation
        return (u, v) if r == 0 else (v, h - u) if r == 90 else (w - u, h - v) if r == 180 else (w - v, u)

    def page_at(self, sx, sy):
        for i, (x, y, w, h) in enumerate(self.rects):
            if x <= sx <= x + w and y <= sy <= y + h:
                return i
        return None

    # ---------- drawing ----------

    def _render(self, i):
        key = (i, round(self.scale, 3), self.rotation, bool(self.settings.get("dark")))
        if key in self.cache:
            self.cache.move_to_end(key)
            return self.cache[key]
        _x, _y, pw, ph = self.rects[i]
        surf = cairo.ImageSurface(cairo.FORMAT_RGB24, max(1, math.ceil(pw)), max(1, math.ceil(ph)))
        cr = cairo.Context(surf)
        cr.set_source_rgb(1, 1, 1)
        cr.paint()
        w, h = self.sizes[i]
        s = self.scale
        if self.rotation == 90:
            cr.translate(h * s, 0)
        elif self.rotation == 180:
            cr.translate(w * s, h * s)
        elif self.rotation == 270:
            cr.translate(0, w * s)
        cr.rotate(math.radians(self.rotation))
        cr.scale(s, s)
        page = self.doc.get_page(i)
        if page:
            page.render(cr)
        if self.settings.get("dark"):
            cr.identity_matrix()
            cr.set_operator(cairo.OPERATOR_DIFFERENCE)
            cr.set_source_rgb(0.92, 0.93, 0.96)
            cr.paint()
        self.cache[key] = surf
        while len(self.cache) > CACHE_PAGES:
            self.cache.popitem(last=False)
        return surf

    def _colour(self, name, fallback):
        ok, c = self.canvas.get_style_context().lookup_color(name)
        return c if ok else Gdk.RGBA(*fallback)

    def _on_draw(self, widget, cr):
        deep = self._colour("nexa_bg_deep", (0.03, 0.05, 0.12, 1))
        cr.set_source_rgba(*((0, 0, 0, 1) if self.present else (deep.red, deep.green, deep.blue, 1)))
        cr.paint()
        if not self.doc:
            self._draw_empty(widget, cr)
            return
        if self.present:
            self._draw_presentation(widget, cr)
            return
        clip = Gdk.cairo_get_clip_rectangle(cr)
        cx, cy, cw, ch = (clip[1].x, clip[1].y, clip[1].width, clip[1].height) if clip[0] else (0, 0, 1 << 20, 1 << 20)
        accent = self._colour("nexa_accent", (0.3, 0.64, 1, 1))
        for i, (x, y, w, h) in enumerate(self.rects):
            if y > cy + ch or y + h < cy or x > cx + cw or x + w < cx:
                continue
            cr.set_source_surface(self._render(i), x, y)
            cr.paint()
            for k, (page, rect) in enumerate(self.matches):
                if page != i:
                    continue
                strong = k == self.match_index
                cr.set_source_rgba(accent.red, accent.green, accent.blue, 0.55 if strong else 0.25)
                self._fill_page_rect(cr, i, rect)
                if strong:
                    cr.set_source_rgba(accent.red, accent.green, accent.blue, 1)
                    cr.set_line_width(2)
                    self._fill_page_rect(cr, i, rect, stroke=True)
            if self.sel and self.sel[0] == i:
                cr.set_source_rgba(accent.red, accent.green, accent.blue, 0.3)
                for r in self.sel_rects:
                    self._fill_page_rect(cr, i, r)

    def _fill_page_rect(self, cr, i, rect, stroke=False):
        x1, y1 = self.to_screen(i, rect[0], rect[1])
        x2, y2 = self.to_screen(i, rect[2], rect[3])
        cr.rectangle(min(x1, x2), min(y1, y2), abs(x2 - x1), abs(y2 - y1))
        cr.stroke() if stroke else cr.fill()

    def _draw_text(self, widget, cr, text, y, size, colour):
        w = widget.get_allocated_width()
        layout = widget.create_pango_layout(text)
        layout.set_font_description(Pango.FontDescription(f"Inter {size}"))
        width = min(w - 48, 640)
        layout.set_width(int(width * Pango.SCALE))
        layout.set_alignment(Pango.Alignment.CENTER)
        cr.move_to((w - width) / 2, y)
        cr.set_source_rgba(colour.red, colour.green, colour.blue, colour.alpha)
        PangoCairo.show_layout(cr, layout)

    def _draw_empty(self, widget, cr):
        h = self.scroller.get_allocated_height()
        self._draw_text(widget, cr, "No document open", h / 2 - 30, 15, self._colour("nexa_text", (0.85, 0.88, 0.96, 1)))
        self._draw_text(widget, cr, "Press Open (Ctrl+O), or drop a PDF here.", h / 2 + 4, 10, self._colour("nexa_muted", (0.56, 0.6, 0.77, 1)))

    def _draw_presentation(self, widget, cr):
        i = self.present_page
        W, H = widget.get_allocated_width(), widget.get_allocated_height()
        w, h = self._page_size(i)
        s = min(W / w, H / h)
        old = (self.scale, self.rects)
        self.scale = s
        self.rects = [(0, 0, w * s, h * s) if k == i else (0, 0, 1, 1) for k in range(len(self.sizes))]
        surf = self._render(i)
        self.scale, self.rects = old
        cr.set_source_surface(surf, (W - w * s) / 2, (H - h * s) / 2)
        cr.paint()

    # ---------- moving around ----------

    def _on_scrolled(self):
        # While a jump waits for the view to grow, the in-between positions don't count.
        if not self.rects or self.present or getattr(self, "_pending", None) is not None:
            return
        adj = self.scroller.get_vadjustment()
        line = adj.get_value() + adj.get_page_size() * 0.35
        cur = self.current
        for i, (_x, y, _w, h) in enumerate(self.rects):
            if y <= line <= y + h + GAP:
                cur = i
                break
        if cur != self.current:
            self.current = cur
            self._sync()
            self._select_thumb()
            self._schedule_remember()

    def go_to(self, i, smooth=True):
        if not self.doc:
            return False
        i = max(0, min(len(self.sizes) - 1, i))
        if self.present:
            self.present_page = i
            self.current = i
            self.canvas.queue_draw()
            self._sync()
            return True
        if not self.rects:
            self._relayout()
        self.current = i
        self._scroll_to(self.rects[i][1] - MARGIN / 2)
        self._sync()
        self._select_thumb()
        return True

    def _on_page_entry(self, *_a):
        try:
            n = int(self.page_entry.get_text().strip())
        except ValueError:
            n = self.current + 1
        self.go_to(n - 1)
        self.set_focus(None)

    def _schedule_remember(self):
        if self.save_timer:
            GLib.source_remove(self.save_timer)
        self.save_timer = GLib.timeout_add(800, lambda: (setattr(self, "save_timer", 0), self._remember(), False)[2])

    def _on_scroll(self, _w, ev):
        if not (ev.state & Gdk.ModifierType.CONTROL_MASK) or not self.doc:
            return False
        if ev.direction == Gdk.ScrollDirection.SMOOTH:
            dy = ev.get_scroll_deltas()[2]
            if dy:
                self.zoom_by(ZOOM_STEP ** (-dy))
        else:
            self.zoom_by(ZOOM_STEP if ev.direction == Gdk.ScrollDirection.UP else 1 / ZOOM_STEP)
        return True

    # ---------- links and selecting text ----------

    def _links_of(self, i):
        if i in self.links:
            return self.links[i]
        out = []
        page = self.doc.get_page(i)
        w, h = self.sizes[i]
        for m in (page.get_link_mapping() if page else []) or []:
            a = m.area
            # poppler's link areas are from the bottom left.
            out.append(((a.x1, h - a.y2, a.x2, h - a.y1), m.action))
        self.links[i] = out
        return out

    def _link_at(self, i, px, py):
        for (x1, y1, x2, y2), action in self._links_of(i):
            if x1 <= px <= x2 and y1 <= py <= y2:
                return action
        return None

    def _follow(self, action):
        kind = action.type
        if kind == Poppler.ActionType.GOTO_DEST:
            dest = action.goto_dest.dest
            if dest.type == Poppler.DestType.NAMED:
                dest = self.doc.find_dest(dest.named_dest) or dest
            if dest.page_num >= 1:
                self.go_to(dest.page_num - 1)
        elif kind == Poppler.ActionType.URI:
            uri = action.uri.uri or ""
            if uri.startswith(("http://", "https://", "mailto:")):
                d = Gtk.MessageDialog(transient_for=self, flags=0, message_type=Gtk.MessageType.QUESTION,
                                      buttons=Gtk.ButtonsType.NONE, text="Open this link?")
                d.format_secondary_text(uri)
                d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Open", Gtk.ResponseType.OK)
                ok = d.run() == Gtk.ResponseType.OK
                d.destroy()
                if ok:
                    Gtk.show_uri_on_window(self, uri, Gdk.CURRENT_TIME)

    def _on_press(self, _w, ev):
        self.set_focus(None)
        if not self.doc or ev.button != 1:
            return False
        if self.present:
            self.go_to(self.present_page + (-1 if ev.x < self.canvas.get_allocated_width() / 3 else 1))
            return True
        i = self.page_at(ev.x, ev.y)
        self.press = (i, ev.x, ev.y) if i is not None else None
        if self.sel:
            self.sel, self.sel_rects, self.sel_text = None, [], ""
            self.canvas.queue_draw()
        return True

    def _on_motion(self, _w, ev):
        if not self.doc or self.present:
            return
        if self.press and self.press[0] is not None:
            i, x0, y0 = self.press
            if abs(ev.x - x0) + abs(ev.y - y0) > 4:
                self._select(i, self.to_page(i, x0, y0), self.to_page(i, ev.x, ev.y))
            return
        i = self.page_at(ev.x, ev.y)
        link = self._link_at(i, *self.to_page(i, ev.x, ev.y)) if i is not None else None
        win = self.canvas.get_window()
        if win:
            win.set_cursor(Gdk.Cursor.new_from_name(self.canvas.get_display(), "pointer" if link else "text" if i is not None else "default"))
        self.canvas.set_tooltip_text(link.uri.uri if link is not None and link.type == Poppler.ActionType.URI else None)

    def _on_release(self, _w, ev):
        if not self.press:
            return
        i, x0, y0 = self.press
        self.press = None
        if i is not None and abs(ev.x - x0) + abs(ev.y - y0) <= 4:
            link = self._link_at(i, *self.to_page(i, ev.x, ev.y))
            if link is not None:
                self._follow(link)

    def _select(self, i, a, b):
        page = self.doc.get_page(i)
        if not page:
            return
        rect = Poppler.Rectangle()
        rect.x1, rect.y1, rect.x2, rect.y2 = a[0], a[1], b[0], b[1]
        self.sel = (i, a, b)
        self.sel_text = page.get_selected_text(Poppler.SelectionStyle.GLYPH, rect) or ""
        rects = []
        try:
            region = page.get_selected_region(1.0, Poppler.SelectionStyle.GLYPH, rect)
            for k in range(region.num_rectangles()):
                r = region.get_rectangle(k)
                rects.append((r.x, r.y, r.x + r.width, r.y + r.height))
        except (TypeError, AttributeError):
            rects = [(min(a[0], b[0]), min(a[1], b[1]), max(a[0], b[0]), max(a[1], b[1]))]
        self.sel_rects = rects
        self.canvas.queue_draw()

    def copy(self):
        if self.sel_text:
            Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text(self.sel_text, -1)

    def copy_page_text(self):
        try:
            text = engine.text(self.path, self.current + 1, self.password)
        except engine.EngineError as e:
            self.error("The page's text couldn't be read", str(e))
            return
        Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text(text, -1)

    # ---------- search ----------

    def show_find(self):
        if not self.doc:
            return
        self.find_revealer.set_reveal_child(True)
        self.find_entry.grab_focus()

    def hide_find(self):
        self.find_revealer.set_reveal_child(False)
        self._cancel_search()
        self.matches, self.match_index = [], -1
        self.find_count.set_text("")
        self.canvas.queue_draw()
        self.set_focus(None)

    def _search_key(self):
        return (self.find_entry.get_text(), self.case_btn.get_active(), self.word_btn.get_active())

    def _cancel_search(self):
        self.search_gen += 1
        if self.search_proc and self.search_proc.poll() is None:
            self.search_proc.terminate()
        self.search_proc = None

    def search(self):
        text, case, words = self._search_key()
        self._cancel_search()
        self.matches, self.match_index = [], -1
        self._searched = (text, case, words)
        if not text.strip() or not self.doc:
            self.find_count.set_text("")
            self.canvas.queue_draw()
            return
        gen = self.search_gen
        self._search_from = self.current
        self.find_count.get_style_context().remove_class("bad")
        self.find_count.set_text("Looking…")
        path, password = self.path, self.password

        def found(f):
            GLib.idle_add(self._found, gen, f)

        def started(proc):
            self.search_proc = proc

        def work():
            try:
                r = engine.search(path, text, case, words, password, on_found=found, started=started)
                GLib.idle_add(self._search_done, gen, r, None)
            except engine.EngineError as e:
                GLib.idle_add(self._search_done, gen, None, str(e))
        threading.Thread(target=work, daemon=True).start()

    def _found(self, gen, f):
        if gen != self.search_gen:
            return False
        self.matches.extend((f["page"] - 1, tuple(r)) for r in f["rects"])
        self.find_count.set_text(f"{len(self.matches)} found so far…")
        if self.match_index < 0 and f["page"] - 1 >= self._search_from:
            # The first match from the page being read.
            self.find_next(1, from_page=self._search_from)
        self.canvas.queue_draw()
        return False

    def _search_done(self, gen, result, error):
        if gen != self.search_gen:
            return False
        ctx = self.find_count.get_style_context()
        if error:
            ctx.add_class("bad")
            self.find_count.set_text(error)
        elif not self.matches:
            ctx.add_class("bad")
            self.find_count.set_text("Not found")
        else:
            if self.match_index < 0:
                self.find_next(1, from_page=0)  # none after the page being read: from the start
            self._count_label()
        return False

    def _count_label(self):
        if self.matches:
            self.find_count.set_text(f"{self.match_index + 1} of {len(self.matches)}")

    def find_next(self, step, from_page=None):
        if not self.matches:
            if self.find_entry.get_text():
                self.search()
            return
        if from_page is not None:
            later = [k for k, (p, _r) in enumerate(self.matches) if p >= from_page]
            self.match_index = later[0] if later else 0
        else:
            self.match_index = (self.match_index + step) % len(self.matches)
        page, rect = self.matches[self.match_index]
        if self.present:
            self.go_to(page)
        else:
            _x, y1 = self.to_screen(page, rect[0], rect[1])
            adj = self.scroller.get_vadjustment()
            if not (adj.get_value() + 40 <= y1 <= adj.get_value() + adj.get_page_size() - 60):
                adj.set_value(max(0, y1 - adj.get_page_size() / 3))
        self._count_label()
        self.canvas.queue_draw()

    # ---------- sidebar ----------

    def _toggle_sidebar(self):
        self.settings["sidebar"] = self.side_btn.get_active()
        save_settings(self.settings)
        self._sync()
        if self.side_btn.get_active():
            self._load_thumbs()

    def _fill_toc(self, outline):
        self.toc_store.clear()

        def add(parent, items, depth):
            for it in items[:5000]:
                row = self.toc_store.append(parent, [it.get("title") or "", int(it.get("page") or 0)])
                if depth < 16:
                    add(row, it.get("children") or [], depth + 1)
        add(None, outline, 0)
        if outline:
            self.toc.expand_all()
        self.toc_stack.set_visible_child_name("toc" if outline else "none")
        self.toc_stack.show_all()

    def _fill_thumbs(self):
        for c in self.thumb_list.get_children():
            c.destroy()
        self.thumb_rows = []
        for i in range(len(self.sizes)):
            row = Gtk.ListBoxRow()
            row.page = i
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            box.set_border_width(4)
            img = Gtk.Image()
            w, h = self.sizes[i]
            img.set_size_request(THUMB_WIDTH, int(THUMB_WIDTH * h / max(w, 1)))
            num = Gtk.Label(label=str(i + 1))
            num.get_style_context().add_class("pnum")
            box.pack_start(img, False, False, 0)
            box.pack_start(num, False, False, 0)
            row.add(box)
            row.img = img
            self.thumb_list.add(row)
            self.thumb_rows.append(row)
        self.thumb_list.show_all()
        self._load_thumbs()

    def _load_thumbs(self):
        if not self.doc or not self.side_btn.get_active():
            return
        self.thumb_gen += 1
        gen, path, password = self.thumb_gen, self.path, self.password
        want = [r.page + 1 for r in self.thumb_rows if not getattr(r, "done", False)][:400]
        outdir = cache_dir(path)

        def got(t):
            GLib.idle_add(self._got_thumb, gen, t)

        def work():
            for k in range(0, len(want), 20):
                if gen != self.thumb_gen:
                    return
                try:
                    engine.thumbs(path, outdir, THUMB_WIDTH, want[k:k + 20], password, on_thumb=got)
                except engine.EngineError:
                    return
        threading.Thread(target=work, daemon=True).start()

    def _got_thumb(self, gen, t):
        if gen != self.thumb_gen:
            return False
        i = t["page"] - 1
        if 0 <= i < len(self.thumb_rows):
            row = self.thumb_rows[i]
            row.img.set_from_file(t["png"])
            row.done = True
        return False

    def _select_thumb(self):
        rows = getattr(self, "thumb_rows", [])
        if 0 <= self.current < len(rows):
            self.thumb_list.select_row(rows[self.current])

    # ---------- details ----------

    def show_details(self):
        m = self.meta
        w, h = self.sizes[self.current]
        rows = [
            ("Title", m.get("title")), ("Author", m.get("author")), ("Subject", m.get("subject")),
            ("Pages", str(len(self.sizes))), ("Page size", f"{round(w * 25.4 / 72)} × {round(h * 25.4 / 72)} mm"),
            ("Made with", m.get("creator")), ("Saved by", m.get("producer")),
            ("Created", time.strftime("%d %B %Y, %H:%M", time.localtime(m["created"])) if m.get("created") else None),
            ("Changed", time.strftime("%d %B %Y, %H:%M", time.localtime(m["modified"])) if m.get("modified") else None),
            ("File", self.path),
        ]
        d = Gtk.Dialog(title="Document Details", transient_for=self, modal=True)
        d.add_button("Close", Gtk.ResponseType.CLOSE)
        grid = Gtk.Grid(row_spacing=8, column_spacing=16)
        grid.set_border_width(16)
        r = 0
        for key, value in rows:
            if not value:
                continue
            k = Gtk.Label(label=key)
            k.set_xalign(1)
            v = Gtk.Label(label=value)
            v.set_xalign(0)
            v.set_selectable(True)
            v.set_line_wrap(True)
            v.set_max_width_chars(48)
            grid.attach(k, 0, r, 1, 1)
            grid.attach(v, 1, r, 1, 1)
            r += 1
        d.get_content_area().pack_start(grid, True, True, 0)
        d.show_all()
        d.run()
        d.destroy()

    # ---------- full screen and presentation ----------

    def toggle_fullscreen(self, on=None):
        self._full = not getattr(self, "_full", False) if on is None else on
        (self.fullscreen if self._full else self.unfullscreen)()
        if not self._full and self.present:
            self.stop_presentation()

    def start_presentation(self):
        if not self.doc:
            return
        self.present = True
        self.present_page = self.current
        self.toolbar.hide()
        self.find_revealer.set_reveal_child(False)
        if self.get_titlebar():
            self.get_titlebar().hide()
        self.toggle_fullscreen(True)
        self.scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.NEVER)
        self.canvas.set_size_request(-1, -1)
        self._sync()
        self.canvas.queue_draw()

    def stop_presentation(self):
        if not self.present:
            return
        self.present = False
        self.toolbar.show()
        if self.get_titlebar():
            self.get_titlebar().show()
        self.scroller.set_policy(Gtk.PolicyType.AUTOMATIC, Gtk.PolicyType.AUTOMATIC)
        self.toggle_fullscreen(False)
        self.cache.clear()
        self._relayout(keep=False)
        GLib.idle_add(lambda: self.go_to(self.current) and False)

    # ---------- keys and drops ----------

    def _on_key(self, _w, ev):
        ctrl = ev.state & Gdk.ModifierType.CONTROL_MASK
        shift = ev.state & Gdk.ModifierType.SHIFT_MASK
        name = Gdk.keyval_name(ev.keyval) or ""
        if name == "Escape":
            if self.present:
                self.stop_presentation()
            elif getattr(self, "_full", False):
                self.toggle_fullscreen(False)
            elif self.find_revealer.get_reveal_child():
                self.hide_find()
            return True
        if self.present and name in ("Right", "Down", "Page_Down", "space", "Return", "n"):
            self.go_to(self.present_page + 1)
            return True
        if self.present and name in ("Left", "Up", "Page_Up", "BackSpace", "p"):
            self.go_to(self.present_page - 1)
            return True
        if ctrl:
            acts = {"o": self.on_open, "f": self.show_find, "g": lambda: (self.page_entry.grab_focus(), self.page_entry.select_region(0, -1)),
                    "plus": lambda: self.zoom_by(ZOOM_STEP), "equal": lambda: self.zoom_by(ZOOM_STEP),
                    "minus": lambda: self.zoom_by(1 / ZOOM_STEP), "0": lambda: self.set_mode("width"),
                    "r": self.turn, "c": self.copy, "q": self.close, "w": self.close}
            if name.lower() in acts:
                acts[name.lower()]()
                return True
            if name == "Home":
                self.go_to(0)
                return True
            if name == "End":
                self.go_to(len(self.sizes) - 1)
                return True
            return False
        if name == "F3":
            self.find_next(-1 if shift else 1)
            return True
        if name == "F5":
            self.start_presentation()
            return True
        if name == "F11":
            self.toggle_fullscreen()
            return True
        if name == "F9":
            self.side_btn.set_active(not self.side_btn.get_active())
            return True
        return False

    def _on_drop(self, _w, ctx, _x, _y, data, _info, time_):
        for uri in data.get_uris() or []:
            path = Gio.File.new_for_uri(uri).get_path()
            if path and os.path.isfile(path):
                self.open_path(path)
                break
        Gtk.drag_finish(ctx, True, False, time_)

    def error(self, text, secondary=None):
        d = Gtk.MessageDialog(transient_for=self, flags=0, message_type=Gtk.MessageType.ERROR, buttons=Gtk.ButtonsType.OK, text=text)
        if secondary:
            d.format_secondary_text(secondary)
        d.run()
        d.destroy()


def main():
    Gtk.init()
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    win = PdfWindow(args[0] if args else None)
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    win._sync()
    Gtk.main()
