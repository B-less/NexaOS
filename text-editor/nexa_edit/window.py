"""The Editor's window: tabs, find and replace, go to line, view options,
encodings, recent files, drafts of unsaved changes and the open tabs coming
back. Opening, saving and drafts are done by nexa-textio (engine.py)."""
import json
import os
import threading
import time
import uuid

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkSource", "4")
from gi.repository import Gtk, Gdk, Gio, GLib, GtkSource

from nexa_common import themes
from nexa_common.titlebar import attach_titlebar

from . import engine

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar, #findbar, #gotobar { background-image: none; background-color: @nexa_surface; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 10px; padding: 4px 12px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#toolbtn:checked { background-color: @nexa_selected; color: @nexa_selected_text; }
#statusbar { background-image: none; background-color: @nexa_surface; }
#statusbar label { color: @nexa_muted; font-size: 9pt; }
#statusbtn { background-image: none; background-color: transparent; border: none; color: @nexa_muted; font-size: 9pt; padding: 0 8px; border-radius: 6px; }
#statusbtn:hover { background-color: @nexa_hover; color: @nexa_text; }
#findcount { color: @nexa_muted; font-size: 9pt; }
#findcount.bad { color: @nexa_crit; }
#toolbtn, #findbar button, infobar button { border: none; box-shadow: none; }
notebook > header { background-image: none; background-color: @nexa_surface; border: none; box-shadow: none; }
notebook > header > tabs > tab { background-image: none; background-color: transparent; border: none; box-shadow: none; padding: 4px 8px; }
notebook > header > tabs > tab label { color: @nexa_muted; }
notebook > header > tabs > tab:hover { background-color: @nexa_hover; border-radius: 6px 6px 0 0; }
notebook > header > tabs > tab:checked { box-shadow: inset 0 -2px @nexa_accent; }
notebook > header > tabs > tab:checked label { color: @nexa_text_strong; }
notebook > stack { background-color: @nexa_bg; }
infobar > revealer > box { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border: none; }
infobar.warning > revealer > box { box-shadow: inset 4px 0 @nexa_warn; }
infobar.info > revealer > box { box-shadow: inset 4px 0 @nexa_accent; }
infobar label { color: @nexa_text; }
infobar button { background-image: none; background-color: @nexa_surface; color: @nexa_text; border-radius: 10px; padding: 4px 12px; }
infobar button:hover { background-color: @nexa_hover; }
#findbar entry, #gotobar entry { background-image: none; background-color: @nexa_field; color: @nexa_text; border: 1px solid @nexa_border; border-radius: 6px; }
#gotobar label { color: @nexa_text; }
#tabclose { background-image: none; background-color: transparent; border: none; padding: 0 4px; min-height: 0; min-width: 0; color: @nexa_muted; }
#tabclose:hover { color: @nexa_text; background-color: @nexa_hover; border-radius: 6px; }
"""

# "nexa" is written by nexa-theme in the current theme's colours; the rest
# are fallbacks if it hasn't been generated yet.
SCHEMES = ("nexa", "oblivion", "classic")
DEFAULTS = {"wrap": False, "numbers": True, "spaces": False, "highlight": True, "size": 11,
            "recent": [], "session": [], "session_index": 0}
MAX_RECENT = 12


def config_path():
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "nexa-edit", "settings.json")


def load_settings():
    s = dict(DEFAULTS)
    try:
        with open(config_path()) as f:
            s.update({k: v for k, v in json.load(f).items() if k in DEFAULTS})
    except (OSError, ValueError):
        pass
    return s


def save_settings(s):
    p = config_path()
    try:
        os.makedirs(os.path.dirname(p), exist_ok=True)
        tmp = p + ".tmp"
        with open(tmp, "w") as f:
            json.dump(s, f)
        os.replace(tmp, p)
    except OSError:
        pass


def _alive_editor(pid):
    """Is process `pid` another Editor still running (its drafts are its own)?"""
    try:
        with open(f"/proc/{int(pid)}/cmdline", "rb") as f:
            return int(pid) != os.getpid() and b"nexa_edit" in f.read()
    except (OSError, ValueError):
        return False


class Doc:
    """One open file (or an untitled one) in a tab."""

    def __init__(self, win):
        self.win = win
        self.path = None
        self.untitled = 0
        self.encoding = "utf-8"
        self.line_ending = "lf"
        self.stamp = None  # (mtime_ns, size) when opened or saved
        self.writable = True
        self.draft_id = f"{os.getpid()}-{uuid.uuid4().hex[:12]}"
        self.draft_timer = 0
        self.monitor = None

        self.buffer = GtkSource.Buffer()
        self.view = GtkSource.View(buffer=self.buffer)
        self.view.get_style_context().add_class("nexa-code")
        self.view.set_tab_width(4)
        self.view.set_insert_spaces_instead_of_tabs(True)
        self.view.set_auto_indent(True)
        self.view.set_monospace(True)
        self.view.set_left_margin(4)
        self.search = GtkSource.SearchContext.new(self.buffer, win.search_settings)
        self.search.set_highlight(False)

        self.infobar = Gtk.InfoBar()
        self.infobar.set_no_show_all(True)
        self.info_label = Gtk.Label()
        self.info_label.set_line_wrap(True)
        self.info_label.set_xalign(0)
        self.infobar.get_content_area().pack_start(self.info_label, True, True, 0)
        self.infobar.connect("response", self._on_info_response)
        self._info_actions = {}

        scroller = Gtk.ScrolledWindow()
        scroller.add(self.view)
        self.page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.page.pack_start(self.infobar, False, False, 0)
        self.page.pack_start(scroller, True, True, 0)

        self.tab_label = Gtk.Label()
        close = Gtk.Button(label="×")
        close.set_name("tabclose")
        close.set_relief(Gtk.ReliefStyle.NONE)
        close.set_tooltip_text("Close tab (Ctrl+W)")
        close.connect("clicked", lambda *_a: win.close_doc(self))
        self.tab = Gtk.Box(spacing=4)
        self.tab.pack_start(self.tab_label, False, False, 0)
        self.tab.pack_start(close, False, False, 0)
        self.tab.show_all()

    def title(self):
        if self.path:
            return os.path.basename(self.path)
        return "Untitled" if self.untitled <= 1 else f"Untitled {self.untitled}"

    @property
    def modified(self):
        return self.buffer.get_modified()

    def text(self):
        return self.buffer.get_text(*self.buffer.get_bounds(), True)

    def set_text(self, text):
        self.buffer.begin_not_undoable_action()
        self.buffer.set_text(text)
        self.buffer.end_not_undoable_action()
        self.buffer.place_cursor(self.buffer.get_start_iter())
        self.buffer.set_modified(False)

    def is_blank(self):
        return not self.path and not self.modified and self.buffer.get_char_count() == 0

    # --- the notice above the text ---

    def notice(self, text, kind=Gtk.MessageType.INFO, actions=()):
        for child in self.infobar.get_action_area().get_children():
            child.destroy()
        self._info_actions = {}
        for i, (label, fn) in enumerate(actions):
            self.infobar.add_button(label, i + 1)
            self._info_actions[i + 1] = fn
        self.infobar.add_button("Dismiss", Gtk.ResponseType.CLOSE)
        self.infobar.set_message_type(kind)
        self.info_label.set_text(text)
        self.info_label.show()
        self.infobar.show()
        self.infobar.get_action_area().show_all()

    def hide_notice(self):
        self.infobar.hide()

    def _on_info_response(self, _bar, response):
        self.infobar.hide()
        fn = self._info_actions.get(response)
        if fn:
            fn()

    # --- watching the file for other programs' changes ---

    def watch(self):
        if self.monitor:
            self.monitor.cancel()
            self.monitor = None
        if self.path:
            try:
                self.monitor = Gio.File.new_for_path(self.path).monitor_file(Gio.FileMonitorFlags.WATCH_MOVES, None)
                self.monitor.connect("changed", lambda *_a: GLib.timeout_add(250, self.win.check_disk, self))
            except GLib.Error:
                self.monitor = None

    def disk_stamp(self):
        try:
            st = os.stat(self.path)
        except OSError:
            return None
        return (str(st.st_mtime_ns), st.st_size)


class EditorWindow(Gtk.Window):
    def __init__(self, paths=()):
        super().__init__(title="Editor")
        self.set_icon_name("nexa-edit")
        attach_titlebar(self, "Editor")
        self.set_default_size(900, 640)
        self.settings = load_settings()
        self.search_settings = GtkSource.SearchSettings()
        self.search_settings.set_wrap_around(True)
        self._untitled = 0
        self._closing = False
        self.lang_manager = GtkSource.LanguageManager.get_default()

        self._css()
        self._build_ui()
        self._apply_view_settings()
        self._theme_monitor = Gio.File.new_for_path(themes.settings_path()).monitor_file(Gio.FileMonitorFlags.NONE, None)
        self._theme_monitor.connect("changed", lambda *_a: GLib.timeout_add(300, self._apply_scheme))

        opened = False
        for p in paths:
            opened = self.open_path(os.path.abspath(p)) or opened
        if not paths:
            for p in self.settings.get("session") or []:
                if isinstance(p, str) and os.path.isfile(p):
                    opened = self.open_path(p, quiet=True) or opened
            if opened:
                idx = self.settings.get("session_index") or 0
                if 0 <= idx < self.notebook.get_n_pages():
                    self.notebook.set_current_page(idx)
        recovered = self.recover_drafts()
        if not opened and not recovered:
            self.new_doc()
        self.connect("delete-event", self._on_delete)
        self.drag_dest_set(Gtk.DestDefaults.ALL, [], Gdk.DragAction.COPY)
        self.drag_dest_add_uri_targets()
        self.connect("drag-data-received", self._on_drop)

    # ---------- building ----------

    def _css(self):
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.font_css = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), self.font_css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 1)

    def _button(self, label, tip, handler, toggle=False):
        b = Gtk.ToggleButton(label=label) if toggle else Gtk.Button(label=label)
        b.set_name("toolbtn")
        b.set_tooltip_text(tip)
        b.connect("toggled" if toggle else "clicked", handler)
        return b

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.add(root)

        bar = Gtk.Box(spacing=8)
        bar.set_name("toolbar")
        bar.set_border_width(8)
        bar.pack_start(self._button("New", "New tab (Ctrl+N or Ctrl+T)", lambda *_a: self.new_doc()), False, False, 0)
        bar.pack_start(self._button("Open", "Open files (Ctrl+O)", lambda *_a: self.on_open()), False, False, 0)
        bar.pack_start(self._button("Save", "Save (Ctrl+S)", lambda *_a: self.save(self.doc())), False, False, 0)
        bar.pack_start(self._button("Find", "Find (Ctrl+F); replace: Ctrl+H", lambda *_a: self.show_find(False)), False, False, 0)
        self.menu_btn = self._button("More", "Save As, Recent, Go to Line, View", lambda b: self._pop(b, self._main_menu()))
        bar.pack_start(self.menu_btn, False, False, 0)
        root.pack_start(bar, False, False, 0)

        self.notebook = Gtk.Notebook()
        self.notebook.set_scrollable(True)
        self.notebook.set_show_border(False)
        self.notebook.connect("switch-page", lambda _n, page, _i: GLib.idle_add(self._on_switched))
        root.pack_start(self.notebook, True, True, 0)

        root.pack_start(self._build_find(), False, False, 0)
        root.pack_start(self._build_goto(), False, False, 0)

        status = Gtk.Box(spacing=4)
        status.set_name("statusbar")
        status.set_border_width(2)
        self.status_path = Gtk.Label()
        self.status_path.set_xalign(0)
        self.status_path.set_ellipsize(3)  # Pango.EllipsizeMode.END
        self.status_path.set_margin_start(10)
        status.pack_start(self.status_path, True, True, 0)
        self.status_pos = Gtk.Label()
        status.pack_start(self.status_pos, False, False, 8)
        self.lang_btn = self._status_button("Language", self._language_menu)
        self.enc_btn = self._status_button("Encoding used when saving", self._encoding_menu)
        self.eol_btn = self._status_button("Line endings used when saving", self._eol_menu)
        for b in (self.lang_btn, self.enc_btn, self.eol_btn):
            status.pack_end(b, False, False, 0)
        root.pack_start(status, False, False, 0)

        accels = Gtk.AccelGroup()
        self.add_accel_group(accels)
        ctrl, shift = Gdk.ModifierType.CONTROL_MASK, Gdk.ModifierType.SHIFT_MASK
        for key, mods, fn in (
            ("n", ctrl, lambda: self.new_doc()), ("t", ctrl, lambda: self.new_doc()),
            ("o", ctrl, self.on_open), ("s", ctrl, lambda: self.save(self.doc())),
            ("s", ctrl | shift, lambda: self.save_as(self.doc())), ("w", ctrl, lambda: self.close_doc(self.doc())),
            ("f", ctrl, lambda: self.show_find(False)), ("h", ctrl, lambda: self.show_find(True)),
            ("l", ctrl, self.show_goto), ("q", ctrl, self.close),
            ("plus", ctrl, lambda: self.zoom(1)), ("equal", ctrl, lambda: self.zoom(1)),
            ("minus", ctrl, lambda: self.zoom(-1)), ("0", ctrl, lambda: self.zoom(0)),
            ("F3", 0, lambda: self.find_step(True)), ("F3", shift, lambda: self.find_step(False)),
        ):
            accels.connect(Gdk.keyval_from_name(key), mods, 0, lambda *_a, f=fn: f() or True)
        self.connect("key-press-event", self._on_key)

    def _status_button(self, tip, menu_fn):
        b = Gtk.Button()
        b.set_name("statusbtn")
        b.set_tooltip_text(tip)
        b.set_relief(Gtk.ReliefStyle.NONE)
        b.connect("clicked", lambda btn: self._pop(btn, menu_fn()))
        return b

    def _pop(self, widget, menu):
        """A menu built now (it shows what's true now), under `widget`."""
        menu.attach_to_widget(widget, None)
        menu.connect("deactivate", lambda m: GLib.idle_add(m.destroy))
        menu.popup_at_widget(widget, Gdk.Gravity.SOUTH_WEST, Gdk.Gravity.NORTH_WEST, None)

    def _build_find(self):
        self.find_revealer = Gtk.Revealer()
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        box.set_name("findbar")
        box.set_border_width(8)
        row = Gtk.Box(spacing=8)
        self.find_entry = Gtk.SearchEntry()
        self.find_entry.set_placeholder_text("Find")
        self.find_entry.set_width_chars(28)
        self.find_entry.connect("search-changed", self._on_find_changed)
        self.find_entry.connect("activate", lambda *_a: self.find_step(True))
        row.pack_start(self.find_entry, False, False, 0)
        row.pack_start(self._button("‹", "Previous match (Shift+F3)", lambda *_a: self.find_step(False)), False, False, 0)
        row.pack_start(self._button("›", "Next match (F3 or Enter)", lambda *_a: self.find_step(True)), False, False, 0)
        self.case_btn = self._button("Aa", "Match upper and lower case", self._on_find_options, toggle=True)
        self.word_btn = self._button("Word", "Whole words only", self._on_find_options, toggle=True)
        self.regex_btn = self._button(".*", "Regular expression", self._on_find_options, toggle=True)
        for b in (self.case_btn, self.word_btn, self.regex_btn):
            row.pack_start(b, False, False, 0)
        self.find_count = Gtk.Label()
        self.find_count.set_name("findcount")
        row.pack_start(self.find_count, False, False, 4)
        row.pack_end(self._button("Close", "Close (Esc)", lambda *_a: self.hide_find()), False, False, 0)
        box.pack_start(row, False, False, 0)
        self.replace_row = Gtk.Box(spacing=8)
        self.replace_entry = Gtk.Entry()
        self.replace_entry.set_placeholder_text("Replace with")
        self.replace_entry.set_width_chars(28)
        self.replace_entry.connect("activate", lambda *_a: self.replace_one())
        self.replace_row.pack_start(self.replace_entry, False, False, 0)
        self.replace_row.pack_start(self._button("Replace", "Replace this match and find the next", lambda *_a: self.replace_one()), False, False, 0)
        self.replace_row.pack_start(self._button("Replace All", "Replace every match", lambda *_a: self.replace_all()), False, False, 0)
        for child in self.replace_row.get_children():
            child.show()
        self.replace_row.set_no_show_all(True)
        box.pack_start(self.replace_row, False, False, 0)
        self.find_revealer.add(box)
        return self.find_revealer

    def _build_goto(self):
        self.goto_revealer = Gtk.Revealer()
        row = Gtk.Box(spacing=8)
        row.set_name("gotobar")
        row.set_border_width(8)
        row.pack_start(Gtk.Label(label="Go to line"), False, False, 0)
        self.goto_entry = Gtk.Entry()
        self.goto_entry.set_width_chars(10)
        self.goto_entry.set_input_purpose(Gtk.InputPurpose.DIGITS)
        self.goto_entry.connect("activate", lambda *_a: self.goto_line())
        row.pack_start(self.goto_entry, False, False, 0)
        self.goto_hint = Gtk.Label()
        self.goto_hint.set_name("findcount")
        row.pack_start(self.goto_hint, False, False, 0)
        row.pack_end(self._button("Close", "Close (Esc)", lambda *_a: self.goto_revealer.set_reveal_child(False)), False, False, 0)
        self.goto_revealer.add(row)
        return self.goto_revealer

    # ---------- menus ----------

    def _item(self, menu, label, fn, sensitive=True):
        it = Gtk.MenuItem(label=label)
        it.set_sensitive(sensitive)
        it.connect("activate", lambda *_a: fn())
        menu.append(it)
        return it

    def _check(self, menu, label, key):
        it = Gtk.CheckMenuItem(label=label)
        it.set_active(bool(self.settings.get(key)))
        it.connect("toggled", lambda w: self._set_option(key, w.get_active()))
        menu.append(it)

    def _main_menu(self):
        m = Gtk.Menu()
        doc = self.doc()
        self._item(m, "Save As…  (Ctrl+Shift+S)", lambda: self.save_as(self.doc()))
        self._item(m, "Save All", self.save_all)
        recent = Gtk.MenuItem(label="Recent Files")
        sub = Gtk.Menu()
        files = [p for p in self.settings.get("recent") or [] if isinstance(p, str)]
        for p in files:
            self._item(sub, p.replace(os.path.expanduser("~"), "~", 1), lambda p=p: self.open_path(p))
        if files:
            sub.append(Gtk.SeparatorMenuItem())
            self._item(sub, "Clear the list", self._clear_recent)
        else:
            self._item(sub, "Nothing opened yet", lambda: None, sensitive=False)
        recent.set_submenu(sub)
        m.append(recent)
        m.append(Gtk.SeparatorMenuItem())
        self._item(m, "Replace…  (Ctrl+H)", lambda: self.show_find(True))
        self._item(m, "Go to Line…  (Ctrl+L)", self.show_goto)
        m.append(Gtk.SeparatorMenuItem())
        self._check(m, "Wrap long lines", "wrap")
        self._check(m, "Line numbers", "numbers")
        self._check(m, "Show spaces and tabs", "spaces")
        self._check(m, "Highlight the current line", "highlight")
        self._item(m, "Bigger text  (Ctrl +)", lambda: self.zoom(1))
        self._item(m, "Smaller text  (Ctrl −)", lambda: self.zoom(-1))
        self._item(m, "Normal size  (Ctrl+0)", lambda: self.zoom(0))
        m.append(Gtk.SeparatorMenuItem())
        self._item(m, "Close Tab  (Ctrl+W)", lambda: self.close_doc(self.doc()), sensitive=doc is not None)
        self._item(m, "Quit  (Ctrl+Q)", self.close)
        m.show_all()
        return m

    def _language_menu(self):
        m = Gtk.Menu()
        doc = self.doc()
        self._item(m, "Plain text", lambda: self._set_language(None))
        sections = {}
        for lid in self.lang_manager.get_language_ids() or []:
            lang = self.lang_manager.get_language(lid)
            if lang and not lang.get_hidden():
                sections.setdefault(lang.get_section() or "Other", []).append(lang)
        for section in sorted(sections):
            it = Gtk.MenuItem(label=section)
            sub = Gtk.Menu()
            for lang in sorted(sections[section], key=lambda l: l.get_name().lower()):
                self._item(sub, lang.get_name(), lambda l=lang: self._set_language(l))
            it.set_submenu(sub)
            m.append(it)
        m.set_sensitive(doc is not None)
        m.show_all()
        return m

    def _encoding_menu(self):
        m = Gtk.Menu()
        for enc, name in engine.ENCODINGS.items():
            self._item(m, name, lambda e=enc: self._set_encoding(e))
        m.show_all()
        return m

    def _eol_menu(self):
        m = Gtk.Menu()
        for eol, name in (("lf", "LF (Linux and most programs)"), ("crlf", "CRLF (Windows)"), ("cr", "CR (very old programs)")):
            self._item(m, name, lambda e=eol: self._set_eol(e))
        m.show_all()
        return m

    # ---------- documents ----------

    def doc(self):
        page = self.notebook.get_nth_page(self.notebook.get_current_page()) if self.notebook.get_n_pages() else None
        return self._doc_of(page)

    def _doc_of(self, page):
        return getattr(page, "nexa_doc", None) if page else None

    def docs(self):
        return [self._doc_of(self.notebook.get_nth_page(i)) for i in range(self.notebook.get_n_pages())]

    def _add(self, doc):
        doc.page.nexa_doc = doc
        self._apply_scheme_to(doc)
        self._apply_view_to(doc)
        doc.buffer.connect("modified-changed", lambda *_a: self._refresh_doc(doc))
        doc.buffer.connect("changed", lambda *_a: self._schedule_draft(doc))
        doc.buffer.connect("notify::cursor-position", lambda *_a: self._update_status())
        doc.buffer.connect("mark-set", lambda *_a: self._update_status())
        doc.search.connect("notify::occurrences-count", lambda *_a: self._update_find_count())
        doc.view.drag_dest_add_uri_targets()
        doc.view.connect("drag-data-received", self._on_view_drop)
        doc.page.show_all()
        i = self.notebook.append_page(doc.page, doc.tab)
        self.notebook.set_tab_reorderable(doc.page, True)
        self.notebook.set_show_tabs(self.notebook.get_n_pages() > 1)
        self.notebook.set_current_page(i)
        self._refresh_doc(doc)
        doc.view.grab_focus()
        return doc

    def new_doc(self):
        doc = Doc(self)
        self._untitled += 1
        doc.untitled = self._untitled
        return self._add(doc)

    def find_doc(self, path):
        real = os.path.realpath(path)
        return next((d for d in self.docs() if d.path and os.path.realpath(d.path) == real), None)

    def open_path(self, path, quiet=False):
        existing = self.find_doc(path)
        if existing:
            self.notebook.set_current_page(self.notebook.page_num(existing.page))
            return True
        try:
            r = engine.run("open", path=path)
        except engine.EngineError as e:
            if not quiet:
                self.error(f"Couldn't open {os.path.basename(path)}", str(e))
            return False
        current = self.doc()
        doc = Doc(self)
        doc.path = path
        doc.encoding, doc.line_ending = r["encoding"], r["line_ending"]
        doc.stamp = (r["mtime_ns"], r["size"])
        doc.writable = r["writable"]
        doc.set_text(r["text"])
        doc.buffer.set_language(self._guess_language(path))
        self._add(doc)
        if current and current.is_blank():
            self._remove(current)
        doc.watch()
        notes = []
        if r.get("replaced"):
            notes.append("Some characters in this file weren't valid and are shown as �.")
        if r.get("mixed"):
            notes.append(f"This file mixed different line endings; it will be saved with {engine.LINE_ENDINGS[doc.line_ending]} only.")
        if not doc.writable:
            notes.append("You can read this file but not save changes to it. Use Save As to keep a copy.")
        if notes:
            doc.notice(" ".join(notes), Gtk.MessageType.WARNING if not doc.writable else Gtk.MessageType.INFO)
        self._add_recent(path)
        return True

    def _remove(self, doc):
        if doc.monitor:
            doc.monitor.cancel()
        if doc.draft_timer:
            GLib.source_remove(doc.draft_timer)
            doc.draft_timer = 0
        self.notebook.remove_page(self.notebook.page_num(doc.page))
        self.notebook.set_show_tabs(self.notebook.get_n_pages() > 1)

    def close_doc(self, doc):
        if doc is None:
            return False
        if doc.modified:
            answer = self.ask_unsaved([doc])
            if answer == "cancel":
                return False
            if answer == "save" and not self.save(doc):
                return False
        self._drop_draft(doc)
        self._remove(doc)
        if not self.notebook.get_n_pages() and not self._closing:
            self.new_doc()
        return True

    def _refresh_doc(self, doc):
        doc.tab_label.set_text(("● " if doc.modified else "") + doc.title())
        doc.tab_label.set_tooltip_text(doc.path or "Not saved yet")
        if doc is self.doc():
            self._update_title()
            self._update_status()

    def _update_title(self):
        doc = self.doc()
        if not doc:
            return
        text = f"{'● ' if doc.modified else ''}{doc.title()} — Editor"
        self.set_title(text)
        bar = self.get_titlebar()
        if isinstance(bar, Gtk.HeaderBar):
            bar.set_title(text)

    def _update_status(self):
        doc = self.doc()
        if not doc:
            return
        buf = doc.buffer
        it = buf.get_iter_at_mark(buf.get_insert())
        pos = f"Ln {it.get_line() + 1}, Col {it.get_line_offset() + 1}"
        sel = buf.get_selection_bounds()
        if sel:
            a, b = sel
            pos += f"  ({abs(b.get_offset() - a.get_offset())} selected)"
        self.status_pos.set_text(pos)
        self.status_path.set_text(doc.path.replace(os.path.expanduser("~"), "~", 1) if doc.path else "Not saved yet")
        lang = buf.get_language()
        self.lang_btn.set_label(lang.get_name() if lang else "Plain text")
        self.enc_btn.set_label(engine.ENCODINGS.get(doc.encoding, doc.encoding))
        self.eol_btn.set_label(engine.LINE_ENDINGS.get(doc.line_ending, doc.line_ending))

    def _on_switched(self):
        doc = self.doc()
        if doc:
            self._update_title()
            self._update_status()
            for d in self.docs():
                d.search.set_highlight(d is doc and self.find_revealer.get_reveal_child())
            self._update_find_count()
        return False

    def _guess_language(self, path):
        content_type, _ = Gio.content_type_guess(path, None)
        return self.lang_manager.guess_language(path, content_type)

    def _set_language(self, lang):
        doc = self.doc()
        if doc:
            doc.buffer.set_language(lang)
            self._update_status()

    def _set_encoding(self, enc):
        doc = self.doc()
        if doc and doc.encoding != enc:
            doc.encoding = enc
            doc.buffer.set_modified(True)
            self._update_status()

    def _set_eol(self, eol):
        doc = self.doc()
        if doc and doc.line_ending != eol:
            doc.line_ending = eol
            doc.buffer.set_modified(True)
            self._update_status()

    # ---------- open and save ----------

    def on_open(self):
        dialog = Gtk.FileChooserDialog(title="Open", transient_for=self, action=Gtk.FileChooserAction.OPEN)
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Open", Gtk.ResponseType.OK)
        dialog.set_select_multiple(True)
        doc = self.doc()
        if doc and doc.path:
            dialog.set_current_folder(os.path.dirname(doc.path))
        paths = dialog.get_filenames() if dialog.run() == Gtk.ResponseType.OK else []
        dialog.destroy()
        for p in paths:
            self.open_path(p)

    def save(self, doc, path=None, force=False):
        """Save `doc` (to `path` for Save As). True when it was saved."""
        if doc is None:
            return False
        if not path and not doc.path:
            return self.save_as(doc)
        target = path or doc.path
        same = doc.path and os.path.realpath(target) == os.path.realpath(doc.path)
        try:
            r = engine.run("save", path=target, text=doc.text(), encoding=doc.encoding, line_ending=doc.line_ending,
                           expect=({"mtime_ns": doc.stamp[0], "size": doc.stamp[1]} if same and doc.stamp else None), force=force)
        except engine.EngineError as e:
            if e.code in ("changed", "gone"):
                choice = self.choose(f"{doc.title()}: {e}", "Save anyway to replace it with your version, or keep theirs and lose your changes.",
                                     [("Cancel", "cancel"), ("Keep Theirs", "reload"), ("Save Anyway", "force")])
                if choice == "force":
                    return self.save(doc, path, force=True)
                if choice == "reload":
                    self.reload(doc)
                return False
            self.error(f"{doc.title()} wasn't saved", str(e))
            return False
        new_path = not same
        doc.path = target
        doc.stamp = (r["mtime_ns"], r["size"])
        doc.writable = True
        doc.buffer.set_modified(False)
        if new_path:
            doc.buffer.set_language(self._guess_language(target))
            doc.watch()
        doc.hide_notice()
        self._drop_draft(doc)
        self._add_recent(target)
        self._refresh_doc(doc)
        return True

    def save_as(self, doc):
        if doc is None:
            return False
        dialog = Gtk.FileChooserDialog(title="Save As", transient_for=self, action=Gtk.FileChooserAction.SAVE)
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Save", Gtk.ResponseType.OK)
        dialog.set_do_overwrite_confirmation(True)
        if doc.path:
            dialog.set_current_folder(os.path.dirname(doc.path))
            dialog.set_current_name(os.path.basename(doc.path))
        else:
            docs_dir = os.path.join(os.path.expanduser("~"), "Documents")
            dialog.set_current_folder(docs_dir if os.path.isdir(docs_dir) else os.path.expanduser("~"))
            dialog.set_current_name("untitled.txt")
        path = dialog.get_filename() if dialog.run() == Gtk.ResponseType.OK else None
        dialog.destroy()
        return bool(path) and self.save(doc, path, force=True)

    def save_all(self):
        for d in self.docs():
            if d.modified and not self.save(d):
                return False
        return True

    def reload(self, doc):
        try:
            r = engine.run("open", path=doc.path)
        except engine.EngineError as e:
            self.error(f"Couldn't reload {doc.title()}", str(e))
            return
        line = doc.buffer.get_iter_at_mark(doc.buffer.get_insert()).get_line()
        doc.encoding, doc.line_ending = r["encoding"], r["line_ending"]
        doc.stamp = (r["mtime_ns"], r["size"])
        doc.writable = r["writable"]
        doc.set_text(r["text"])
        it = doc.buffer.get_iter_at_line(min(line, doc.buffer.get_line_count() - 1))
        doc.buffer.place_cursor(it)
        GLib.idle_add(lambda: doc.view.scroll_to_mark(doc.buffer.get_insert(), 0.1, False, 0, 0) and False)
        doc.hide_notice()
        self._drop_draft(doc)
        self._refresh_doc(doc)

    def check_disk(self, doc):
        """Another program may have changed or removed doc's file."""
        if not doc.path or doc.page.get_parent() is None:
            return False
        now = doc.disk_stamp()
        if now == doc.stamp:
            return False
        if now is None:
            doc.notice(f"{doc.title()} was deleted or moved by another program. Save to keep your copy.",
                       Gtk.MessageType.WARNING, [("Save", lambda: self.save(doc, force=True))])
            doc.stamp = None
        elif not doc.modified:
            self.reload(doc)
            doc.notice("Reloaded: another program changed this file.")
        else:
            doc.notice(f"Another program changed {doc.title()}. Reload it (your changes here are lost) or keep yours (saving replaces theirs).",
                       Gtk.MessageType.WARNING, [("Reload", lambda: self.reload(doc)), ("Keep Mine", lambda: setattr(doc, "stamp", doc.disk_stamp()))])
        return False

    # ---------- drafts (unsaved changes kept safe) ----------

    def _schedule_draft(self, doc):
        if doc.draft_timer:
            GLib.source_remove(doc.draft_timer)
        doc.draft_timer = GLib.timeout_add(1500, self._write_draft, doc)

    def _write_draft(self, doc):
        doc.draft_timer = 0
        if not doc.modified:
            return False
        args = dict(id=doc.draft_id, path=doc.path, title=doc.title(), text=doc.text(), encoding=doc.encoding, line_ending=doc.line_ending)

        def work():
            try:
                engine.run("draft_put", **args)
            except engine.EngineError:
                pass
        threading.Thread(target=work, daemon=True).start()
        return False

    def _drop_draft(self, doc):
        if doc.draft_timer:
            GLib.source_remove(doc.draft_timer)
            doc.draft_timer = 0
        try:
            engine.run("draft_drop", id=doc.draft_id)
        except engine.EngineError:
            pass

    def recover_drafts(self):
        """Unsaved changes left by an Editor that crashed or lost power."""
        try:
            drafts = engine.run("draft_list")
        except engine.EngineError:
            return 0
        n = 0
        for d in drafts:
            if _alive_editor(str(d.get("id", "")).split("-")[0]):
                continue
            try:
                full = engine.run("draft_get", id=d["id"])
            except engine.EngineError:
                continue
            doc = Doc(self)
            doc.path = full.get("path") or None
            if not doc.path:
                self._untitled += 1
                doc.untitled = self._untitled
            doc.encoding = full.get("encoding") or "utf-8"
            doc.line_ending = full.get("line_ending") or "lf"
            doc.set_text(full.get("text") or "")
            doc.buffer.set_modified(True)
            if doc.path:
                doc.buffer.set_language(self._guess_language(doc.path))
                doc.stamp = doc.disk_stamp()
            existing = doc.path and self.find_doc(doc.path)
            if existing and not existing.modified:
                self._remove(existing)
            current = self.doc()
            self._add(doc)
            if current and current.is_blank():
                self._remove(current)
            doc.watch()
            when = time.strftime("%d %b %H:%M", time.localtime(full.get("saved") or 0))
            doc.notice(f"Unsaved changes recovered (from {when}). Save to keep them, or close the tab to throw them away.",
                       Gtk.MessageType.WARNING)
            # Kept under this Editor's name from now on.
            self._write_draft(doc)
            try:
                engine.run("draft_drop", id=d["id"])
            except engine.EngineError:
                pass
            n += 1
        return n

    # ---------- find, replace, go to line ----------

    def show_find(self, replace):
        self.goto_revealer.set_reveal_child(False)
        self.replace_row.set_visible(replace)
        self.find_revealer.set_reveal_child(True)
        doc = self.doc()
        sel = doc.buffer.get_selection_bounds() if doc else ()
        if sel:
            a, b = sel
            text = doc.buffer.get_text(a, b, False)
            if "\n" not in text and len(text) < 200:
                self.find_entry.set_text(text)
        self.find_entry.grab_focus()
        self._on_find_changed()

    def hide_find(self):
        self.find_revealer.set_reveal_child(False)
        for d in self.docs():
            d.search.set_highlight(False)
        doc = self.doc()
        if doc:
            doc.view.grab_focus()

    def _on_find_options(self, *_a):
        self.search_settings.set_case_sensitive(self.case_btn.get_active())
        self.search_settings.set_at_word_boundaries(self.word_btn.get_active())
        self.search_settings.set_regex_enabled(self.regex_btn.get_active())
        self._update_find_count()

    def _on_find_changed(self, *_a):
        self.search_settings.set_search_text(self.find_entry.get_text() or None)
        doc = self.doc()
        if doc:
            doc.search.set_highlight(True)
        self._update_find_count()

    def _update_find_count(self):
        doc = self.doc()
        ctx = self.find_count.get_style_context()
        ctx.remove_class("bad")
        if not doc or not self.find_entry.get_text():
            self.find_count.set_text("")
            return
        err = doc.search.get_regex_error()
        if err:
            ctx.add_class("bad")
            self.find_count.set_text("That pattern isn't valid")
            return
        n = doc.search.get_occurrences_count()
        if n < 0:
            self.find_count.set_text("…")
            return
        if n == 0:
            ctx.add_class("bad")
            self.find_count.set_text("Not found")
            return
        pos = -1
        sel = doc.buffer.get_selection_bounds()
        if sel:
            a, b = sel
            pos = doc.search.get_occurrence_position(a, b)
        self.find_count.set_text(f"{pos} of {n}" if pos > 0 else f"{n} found")

    def find_step(self, forward):
        doc = self.doc()
        if not doc or not self.find_entry.get_text():
            self.show_find(False)
            return
        buf = doc.buffer
        sel = buf.get_selection_bounds()
        if sel:
            a, b = sel
        else:
            a = b = buf.get_iter_at_mark(buf.get_insert())
        found, start, end, _wrapped = doc.search.forward(b) if forward else doc.search.backward(a)
        if found:
            buf.select_range(start, end)
            doc.view.scroll_to_mark(buf.get_insert(), 0.2, False, 0, 0)
        self._update_find_count()

    def replace_one(self):
        doc = self.doc()
        if not doc or not self.find_entry.get_text():
            return
        buf = doc.buffer
        sel = buf.get_selection_bounds()
        if sel:
            a, b = sel
            if doc.search.get_occurrence_position(a, b) > 0:
                try:
                    doc.search.replace(a, b, self.replace_entry.get_text(), -1)
                except GLib.Error as e:
                    self.find_count.set_text(e.message)
                    return
        self.find_step(True)

    def replace_all(self):
        doc = self.doc()
        if not doc or not self.find_entry.get_text():
            return 0
        try:
            n = doc.search.replace_all(self.replace_entry.get_text(), -1)
        except GLib.Error as e:
            self.find_count.set_text(e.message)
            return 0
        self.find_count.get_style_context().remove_class("bad")
        self.find_count.set_text(f"Replaced {n}" if n else "Not found")
        return n

    def show_goto(self):
        doc = self.doc()
        if not doc:
            return
        self.find_revealer.set_reveal_child(False)
        self.goto_hint.set_text(f"1 to {doc.buffer.get_line_count()}")
        self.goto_entry.set_text("")
        self.goto_revealer.set_reveal_child(True)
        self.goto_entry.grab_focus()

    def goto_line(self, line=None):
        doc = self.doc()
        if not doc:
            return
        try:
            n = int(line if line is not None else self.goto_entry.get_text().strip())
        except ValueError:
            self.goto_hint.set_text(f"Type a number from 1 to {doc.buffer.get_line_count()}")
            return
        n = max(1, min(n, doc.buffer.get_line_count()))
        it = doc.buffer.get_iter_at_line(n - 1)
        doc.buffer.place_cursor(it)
        doc.view.scroll_to_mark(doc.buffer.get_insert(), 0.2, True, 0, 0.3)
        self.goto_revealer.set_reveal_child(False)
        doc.view.grab_focus()

    # ---------- view ----------

    def _set_option(self, key, value):
        self.settings[key] = value
        save_settings(self.settings)
        self._apply_view_settings()

    def zoom(self, step):
        size = self.settings.get("size") or 11
        size = 11 if step == 0 else max(6, min(48, size + step))
        self.settings["size"] = size
        save_settings(self.settings)
        self._apply_view_settings()

    def _apply_view_settings(self):
        size = self.settings.get("size") or 11
        self.font_css.load_from_data(f"textview.nexa-code {{ font-family: 'JetBrains Mono', monospace; font-size: {size}pt; }}".encode())
        for d in self.docs():
            self._apply_view_to(d)

    def _apply_view_to(self, doc):
        s = self.settings
        doc.view.set_wrap_mode(Gtk.WrapMode.WORD_CHAR if s.get("wrap") else Gtk.WrapMode.NONE)
        doc.view.set_show_line_numbers(bool(s.get("numbers")))
        doc.view.set_highlight_current_line(bool(s.get("highlight")))
        drawer = doc.view.get_space_drawer()
        drawer.set_types_for_locations(GtkSource.SpaceLocationFlags.ALL,
                                       GtkSource.SpaceTypeFlags.SPACE | GtkSource.SpaceTypeFlags.TAB if s.get("spaces") else GtkSource.SpaceTypeFlags.NONE)
        drawer.set_enable_matrix(bool(s.get("spaces")))

    def _apply_scheme_to(self, doc):
        manager = GtkSource.StyleSchemeManager.get_default()
        for name in SCHEMES:
            scheme = manager.get_scheme(name)
            if scheme:
                doc.buffer.set_style_scheme(scheme)
                break

    def _apply_scheme(self):
        GtkSource.StyleSchemeManager.get_default().force_rescan()
        for d in self.docs():
            self._apply_scheme_to(d)
        return False

    # ---------- recent and session ----------

    def _add_recent(self, path):
        recent = [p for p in self.settings.get("recent") or [] if isinstance(p, str) and p != path]
        self.settings["recent"] = [path] + recent[:MAX_RECENT - 1]
        save_settings(self.settings)
        try:
            Gtk.RecentManager.get_default().add_item(Gio.File.new_for_path(path).get_uri())
        except GLib.Error:
            pass

    def _clear_recent(self):
        self.settings["recent"] = []
        save_settings(self.settings)

    def _save_session(self):
        docs = self.docs()
        self.settings["session"] = [d.path for d in docs if d.path]
        current = self.doc()
        with_path = [d for d in docs if d.path]
        self.settings["session_index"] = with_path.index(current) if current in with_path else 0
        save_settings(self.settings)

    # ---------- closing ----------

    def ask_unsaved(self, docs):
        names = ", ".join(d.title() for d in docs[:4]) + (f" and {len(docs) - 4} more" if len(docs) > 4 else "")
        text = f"Save the changes to {names}?" if len(docs) == 1 else f"{len(docs)} files have unsaved changes: {names}."
        return self.choose(text, "If you don't save, your changes are lost.",
                           [("Cancel", "cancel"), ("Don't Save", "discard"), ("Save" if len(docs) == 1 else "Save All", "save")])

    def _on_delete(self, *_a):
        unsaved = [d for d in self.docs() if d.modified]
        if unsaved:
            answer = self.ask_unsaved(unsaved)
            if answer == "cancel":
                return True
            if answer == "save":
                for d in unsaved:
                    self.notebook.set_current_page(self.notebook.page_num(d.page))
                    if not self.save(d):
                        return True
        self._save_session()
        self._closing = True
        for d in self.docs():
            self._drop_draft(d)
            if d.monitor:
                d.monitor.cancel()
        return False

    # ---------- keys and drops ----------

    def _on_key(self, _w, event):
        ctrl = event.state & Gdk.ModifierType.CONTROL_MASK
        shift = event.state & Gdk.ModifierType.SHIFT_MASK
        name = Gdk.keyval_name(event.keyval)
        if ctrl and name in ("Tab", "ISO_Left_Tab", "Page_Down", "Page_Up"):
            n = self.notebook.get_n_pages()
            step = -1 if (shift or name in ("ISO_Left_Tab", "Page_Up")) else 1
            self.notebook.set_current_page((self.notebook.get_current_page() + step) % n)
            return True
        if name == "Escape" and (self.find_revealer.get_reveal_child() or self.goto_revealer.get_reveal_child()):
            self.goto_revealer.set_reveal_child(False)
            self.hide_find()
            return True
        return False

    def _open_uris(self, uris):
        opened = False
        for uri in uris or []:
            path = Gio.File.new_for_uri(uri).get_path()
            if path and os.path.isfile(path):
                opened = self.open_path(path) or opened
        return opened

    def _on_drop(self, _w, ctx, _x, _y, data, _info, time_):
        self._open_uris(data.get_uris())
        Gtk.drag_finish(ctx, True, False, time_)

    def _on_view_drop(self, view, ctx, _x, _y, data, _info, time_):
        uris = data.get_uris()
        if uris:
            view.stop_emission_by_name("drag-data-received")
            self._open_uris(uris)
            Gtk.drag_finish(ctx, True, False, time_)

    # ---------- dialogs ----------

    def choose(self, text, secondary, buttons):
        """Ask; buttons are (label, answer), the last one the main choice."""
        dialog = Gtk.MessageDialog(transient_for=self, flags=0, message_type=Gtk.MessageType.QUESTION,
                                   buttons=Gtk.ButtonsType.NONE, text=text)
        dialog.format_secondary_text(secondary)
        for i, (label, _a) in enumerate(buttons):
            dialog.add_button(label, i + 1)
        dialog.set_default_response(len(buttons))
        r = dialog.run()
        dialog.destroy()
        return buttons[r - 1][1] if 1 <= r <= len(buttons) else "cancel"

    def error(self, text, secondary=None):
        dialog = Gtk.MessageDialog(transient_for=self, flags=0, message_type=Gtk.MessageType.ERROR,
                                   buttons=Gtk.ButtonsType.OK, text=text)
        if secondary:
            dialog.format_secondary_text(secondary)
        dialog.run()
        dialog.destroy()


def main(argv):
    Gtk.init()
    win = EditorWindow([a for a in argv if not a.startswith("-")])
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    for d in win.docs():
        if not d.infobar.get_visible():
            d.infobar.hide()
    Gtk.main()
