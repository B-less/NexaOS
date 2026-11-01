"""Music: the library (Songs, Albums with covers, Artists), search, Up Next
(play next, add, reorder, remove), shuffle and repeat, playlists, gapless
playback, media keys (MPRIS), and the queue and position kept for next
time. Tags, covers, the index and playlists are nexa-musiclib's work."""
import json
import os
import random
import threading

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Gst", "1.0")
from gi.repository import Gtk, Gdk, GdkPixbuf, Gio, GLib, Gst, Pango

from nexa_common import magic
from nexa_common.titlebar import attach_titlebar

from . import library
from .mpris import Mpris
from .player import Player

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar, #playerbar { background-image: none; background-color: @nexa_surface; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 10px; padding: 4px 12px; border: none; box-shadow: none; }
#toolbtn:hover { background-color: @nexa_hover; }
#toolbtn:checked { background-color: @nexa_selected; color: @nexa_selected_text; }
#transport { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 10px; border: none; box-shadow: none; padding: 4px 10px; }
#transport:hover { background-color: @nexa_hover; }
#transport:checked { background-color: @nexa_selected; color: @nexa_selected_text; }
#nowtitle { color: @nexa_text_strong; font-weight: bold; }
#nowartist, #timelabel, #status { color: @nexa_muted; font-size: 9pt; }
#timelabel { font-family: "JetBrains Mono", monospace; }
#nav { background-image: none; background-color: @nexa_surface; }
#nav row { padding: 8px 16px; color: @nexa_text; border-radius: 10px; }
#nav row:selected { background-color: @nexa_selected; color: @nexa_selected_text; }
treeview { background-image: none; background-color: @nexa_bg; color: @nexa_text; }
treeview:selected { background-color: @nexa_selected; color: @nexa_selected_text; }
treeview header button { background-image: none; background-color: @nexa_surface; color: @nexa_muted; border: none; box-shadow: none; }
flowbox, flowboxchild { background-color: transparent; }
flowboxchild { border-radius: 16px; padding: 8px; }
flowboxchild:hover { background-color: @nexa_hover; }
.albumtitle { color: @nexa_text; font-weight: bold; }
.albumartist { color: @nexa_muted; font-size: 9pt; }
.heading { color: @nexa_text_strong; font-size: 15pt; font-weight: bold; }
.subhead { color: @nexa_muted; }
.empty { color: @nexa_muted; font-size: 11pt; }
.artistrow label { color: @nexa_text; }
.primary { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; font-weight: bold; border-radius: 10px; border: none; box-shadow: none; padding: 6px 16px; }
.primary:hover { background-color: @nexa_accent_hover; }
#hexplay { background: none; border: none; box-shadow: none; padding: 0; }
"""

AUDIO_EXTS = (".mp3", ".wav", ".ogg", ".oga", ".flac", ".m4a", ".mp4", ".aac", ".opus")
COVER = 150
SAVE_EVERY = 5


def data_dir():
    return os.path.join(os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share"), "nexa-music")


def config_path():
    return os.path.join(os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config"), "nexa-music", "settings.json")


def _load(path, default):
    try:
        with open(path) as f:
            v = json.load(f)
            return v if isinstance(v, dict) else default
    except (OSError, ValueError):
        return default


def _save(path, value):
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path + ".tmp", "w") as f:
            json.dump(value, f)
        os.replace(path + ".tmp", path)
    except OSError:
        pass


def fmt_time(s):
    if s is None or s < 0:
        return "–:––"
    s = int(s + 0.5)
    return f"{s // 3600}:{s % 3600 // 60:02d}:{s % 60:02d}" if s >= 3600 else f"{s // 60}:{s % 60:02d}"


class HexButton(Gtk.Button):
    """The main action: play/pause in the NexaOS hexagon."""

    def __init__(self, on_click):
        super().__init__()
        self.set_name("hexplay")
        self.playing = False
        area = Gtk.DrawingArea()
        area.set_size_request(48, 52)
        area.connect("draw", self._draw)
        self.add(area)
        self.connect("clicked", lambda *_a: on_click())

    def set_playing(self, playing):
        self.playing = playing
        self.set_tooltip_text("Pause (Space)" if playing else "Play (Space)")
        self.queue_draw()

    def _draw(self, area, cr):
        import math
        w, h = area.get_allocated_width(), area.get_allocated_height()
        ctx = area.get_style_context()
        ok, accent = ctx.lookup_color("nexa_accent")
        ok2, on = ctx.lookup_color("nexa_on_accent")
        accent = accent if ok else Gdk.RGBA(0.3, 0.64, 1, 1)
        on = on if ok2 else Gdk.RGBA(0.03, 0.05, 0.12, 1)
        r = min(w, h) / 2 - 1
        cx, cy = w / 2, h / 2
        cr.move_to(cx + r * math.cos(math.radians(-90)), cy + r * math.sin(math.radians(-90)))
        for k in range(1, 6):
            a = math.radians(-90 + 60 * k)
            cr.line_to(cx + r * math.cos(a), cy + r * math.sin(a))
        cr.close_path()
        cr.set_source_rgba(accent.red, accent.green, accent.blue, 1)
        cr.fill()
        cr.set_source_rgba(on.red, on.green, on.blue, 1)
        s = r * 0.42
        if self.playing:
            cr.rectangle(cx - s * 0.75, cy - s, s * 0.5, s * 2)
            cr.rectangle(cx + s * 0.25, cy - s, s * 0.5, s * 2)
        else:
            cr.move_to(cx - s * 0.6, cy - s)
            cr.line_to(cx + s, cy)
            cr.line_to(cx - s * 0.6, cy + s)
            cr.close_path()
        cr.fill()


class SkipButton(Gtk.Button):
    """Previous or next, drawn (not a font character, which a font may lack)."""

    def __init__(self, forward, tip, on_click):
        super().__init__()
        self.set_name("transport")
        self.set_tooltip_text(tip)
        self.forward = forward
        area = Gtk.DrawingArea()
        area.set_size_request(22, 22)
        area.connect("draw", self._draw)
        self.add(area)
        self.connect("clicked", lambda *_a: on_click())

    def _draw(self, area, cr):
        w, h = area.get_allocated_width(), area.get_allocated_height()
        ok, c = self.get_style_context().lookup_color("nexa_text")
        c = c if ok else Gdk.RGBA(0.85, 0.88, 0.96, 1)
        cr.set_source_rgba(c.red, c.green, c.blue, 1)
        cx, cy, s = w / 2, h / 2, min(w, h) / 2.6
        if not self.forward:
            cr.translate(w, 0)
            cr.scale(-1, 1)
        for dx in (-s, 0):
            cr.move_to(cx + dx, cy - s)
            cr.line_to(cx + dx + s, cy)
            cr.line_to(cx + dx, cy + s)
            cr.close_path()
        cr.rectangle(cx + s, cy - s, s * 0.3, s * 2)
        cr.fill()


class MusicWindow(Gtk.Window):
    def __init__(self, paths=None):
        super().__init__(title="Music")
        self.set_icon_name("nexa-music")
        attach_titlebar(self, "Music")
        self.set_default_size(1000, 700)
        self.settings = _load(config_path(), {"dirs": []})
        self.tracks, self.by_path = [], {}
        self.queue, self.pos = [], -1
        self.unshuffled = None
        self.repeat = "off"
        self.state_saved = 0
        self._seeking = False
        self._playing = False
        self._resume_at = None
        self.view_history = []
        self.player = Player(on_state_changed=self._on_player_state, on_eos=self._on_eos,
                             on_error=self._on_player_error, on_next_started=self._on_next_started)
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self._build_ui()
        self.mpris = Mpris(self)
        self.connect("delete-event", lambda *_a: self._save_state() or False)
        self.connect("destroy", lambda *_a: self._closed())
        self.connect("key-press-event", self._on_key)
        self.drag_dest_set(Gtk.DestDefaults.ALL, [], Gdk.DragAction.COPY)
        self.drag_dest_add_uri_targets()
        self.connect("drag-data-received", self._on_drop)
        self.tick_id = GLib.timeout_add(500, self._tick)
        self._restore_state()
        if paths:
            GLib.idle_add(lambda: self.open_paths(paths) and False)
        self.rescan()

    # ---------- building ----------

    def _btn(self, label, tip, fn, name="toolbtn", toggle=False):
        b = Gtk.ToggleButton(label=label) if toggle else Gtk.Button(label=label)
        b.set_name(name)
        b.set_tooltip_text(tip)
        b.connect("toggled" if toggle else "clicked", lambda *_a: fn())
        return b

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.add(root)
        bar = Gtk.Box(spacing=8)
        bar.set_name("toolbar")
        bar.set_border_width(8)
        self.back_btn = self._btn("‹", "Back (Alt+Left)", self.go_back)
        bar.pack_start(self.back_btn, False, False, 0)
        self.search = Gtk.SearchEntry()
        self.search.set_placeholder_text("Search songs, albums, artists")
        self.search.set_width_chars(30)
        self.search.connect("search-changed", lambda *_a: self._apply_search())
        bar.pack_start(self.search, False, False, 0)
        self.status_label = Gtk.Label()
        self.status_label.set_name("status")
        bar.pack_start(self.status_label, True, True, 0)
        bar.pack_end(self._btn("More", "Library folders, read the library again", lambda: self._pop(self.more_btn, self._more_menu())), False, False, 0)
        self.more_btn = bar.get_children()[-1]
        bar.pack_end(self._btn("Open", "Play files or a folder (Ctrl+O)", self.on_open), False, False, 0)
        root.pack_start(bar, False, False, 0)

        body = Gtk.Box()
        self.nav = Gtk.ListBox()
        self.nav.set_name("nav")
        self.nav.set_size_request(170, -1)
        for key, label in (("songs", "Songs"), ("albums", "Albums"), ("artists", "Artists"), ("playlists", "Playlists"), ("queue", "Up Next")):
            row = Gtk.ListBoxRow()
            row.key = key
            l = Gtk.Label(label=label)
            l.set_xalign(0)
            row.add(l)
            self.nav.add(row)
        self.nav.connect("row-selected", lambda _l, row: row and self.show_view(row.key))
        body.pack_start(self.nav, False, False, 0)
        self.stack = Gtk.Stack()
        self.stack.set_transition_type(Gtk.StackTransitionType.CROSSFADE)
        self.stack.set_transition_duration(160)
        body.pack_start(self.stack, True, True, 0)
        root.pack_start(body, True, True, 0)

        self._build_songs()
        self._build_albums()
        self._build_artists()
        self._build_playlists()
        self._build_queue()
        self._build_detail()
        self.empty = self._empty("No music yet", "Put songs in your Music folder, or press Open to play some.")
        self.stack.add_named(self.empty, "empty")
        root.pack_start(self._build_player(), False, False, 0)

    def _empty(self, title, text):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        box.set_valign(Gtk.Align.CENTER)
        t = Gtk.Label(label=title)
        t.get_style_context().add_class("heading")
        s = Gtk.Label(label=text)
        s.get_style_context().add_class("empty")
        s.set_line_wrap(True)
        box.pack_start(t, False, False, 0)
        box.pack_start(s, False, False, 0)
        return box

    def _song_view(self, store, columns=("title", "artist", "album", "length")):
        view = Gtk.TreeView(model=store)
        view.set_enable_search(False)
        view.get_selection().set_mode(Gtk.SelectionMode.MULTIPLE)
        names = {"num": ("#", 0), "title": ("Title", 1), "artist": ("Artist", 2), "album": ("Album", 3), "length": ("Length", 4)}
        for key in columns:
            title, col = names[key]
            r = Gtk.CellRendererText()
            r.set_property("ellipsize", Pango.EllipsizeMode.END)
            c = Gtk.TreeViewColumn(title, r, text=col, weight=6)
            c.set_resizable(True)
            c.set_expand(key in ("title", "artist", "album"))
            if key in ("length", "num"):
                r.set_property("xalign", 1.0)
                r.set_property("family", "JetBrains Mono")
            view.append_column(c)
        view.connect("row-activated", lambda v, p, _c: self._play_from_view(v, p))
        view.connect("button-press-event", self._on_list_click)
        return view

    def _store(self):
        # num, title, artist, album, length, path, weight (bold = playing)
        return Gtk.ListStore(str, str, str, str, str, str, int)

    def _row(self, t, num=""):
        bold = 700 if self.current() and self.current()["path"] == t["path"] else 400
        return [str(num), t["title"], t.get("artist") or "Unknown artist", t.get("album") or "", fmt_time(t.get("duration")), t["path"], bold]

    def _scrolled(self, child):
        s = Gtk.ScrolledWindow()
        s.add(child)
        return s

    def _build_songs(self):
        self.songs_store = self._store()
        self.songs_filter = self.songs_store.filter_new()
        self.songs_filter.set_visible_func(self._song_visible)
        self.songs_view = self._song_view(Gtk.TreeModelSort(model=self.songs_filter))
        for i, c in enumerate(self.songs_view.get_columns()):
            c.set_sort_column_id(i + 1)
        self.stack.add_named(self._scrolled(self.songs_view), "songs")

    def _build_albums(self):
        self.albums_box = Gtk.FlowBox()
        self.albums_box.set_valign(Gtk.Align.START)
        self.albums_box.set_max_children_per_line(12)
        self.albums_box.set_selection_mode(Gtk.SelectionMode.NONE)
        self.albums_box.set_homogeneous(True)
        self.albums_box.set_border_width(16)
        self.albums_box.set_filter_func(self._album_visible)
        self.albums_box.connect("child-activated", lambda _b, child: self.show_album(child.key))
        self.stack.add_named(self._scrolled(self.albums_box), "albums")

    def _build_artists(self):
        self.artists_list = Gtk.ListBox()
        self.artists_list.set_filter_func(self._artist_visible)
        self.artists_list.connect("row-activated", lambda _l, row: self.show_artist(row.key))
        self.stack.add_named(self._scrolled(self.artists_list), "artists")

    def _build_playlists(self):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        box.set_border_width(16)
        top = Gtk.Box(spacing=8)
        h = Gtk.Label(label="Playlists")
        h.get_style_context().add_class("heading")
        top.pack_start(h, False, False, 0)
        new = Gtk.Button(label="New Playlist")
        new.get_style_context().add_class("primary")
        new.connect("clicked", lambda *_a: self.new_playlist())
        top.pack_end(new, False, False, 0)
        box.pack_start(top, False, False, 0)
        self.playlists_list = Gtk.ListBox()
        self.playlists_list.connect("row-activated", lambda _l, row: self.show_playlist(row.key))
        box.pack_start(self._scrolled(self.playlists_list), True, True, 0)
        self.stack.add_named(box, "playlists")

    def _build_queue(self):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        box.set_border_width(16)
        top = Gtk.Box(spacing=8)
        h = Gtk.Label(label="Up Next")
        h.get_style_context().add_class("heading")
        top.pack_start(h, False, False, 0)
        top.pack_end(self._btn("Save as Playlist", "Keep these songs as a playlist", self.queue_to_playlist), False, False, 0)
        top.pack_end(self._btn("Clear", "Empty Up Next (the song playing stays)", self.clear_queue), False, False, 0)
        box.pack_start(top, False, False, 0)
        self.queue_store = self._store()
        self.queue_view = self._song_view(self.queue_store, ("num", "title", "artist", "length"))
        self.queue_view.set_reorderable(True)
        self.queue_store.connect("row-deleted", lambda *_a: GLib.idle_add(self._queue_reordered))
        box.pack_start(self._scrolled(self.queue_view), True, True, 0)
        self.stack.add_named(box, "queue")

    def _build_detail(self):
        """One album, artist or playlist: a header, actions and its songs."""
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        box.set_border_width(16)
        head = Gtk.Box(spacing=16)
        self.detail_cover = Gtk.Image()
        head.pack_start(self.detail_cover, False, False, 0)
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        text.set_valign(Gtk.Align.END)
        self.detail_title = Gtk.Label()
        self.detail_title.get_style_context().add_class("heading")
        self.detail_title.set_xalign(0)
        self.detail_title.set_line_wrap(True)
        self.detail_sub = Gtk.Label()
        self.detail_sub.get_style_context().add_class("subhead")
        self.detail_sub.set_xalign(0)
        text.pack_start(self.detail_title, False, False, 0)
        text.pack_start(self.detail_sub, False, False, 0)
        self.detail_actions = Gtk.Box(spacing=8)
        text.pack_start(self.detail_actions, False, False, 8)
        head.pack_start(text, True, True, 0)
        box.pack_start(head, False, False, 0)
        self.detail_store = self._store()
        self.detail_view = self._song_view(self.detail_store, ("num", "title", "artist", "length"))
        box.pack_start(self._scrolled(self.detail_view), True, True, 0)
        self.stack.add_named(box, "detail")

    def _build_player(self):
        bar = Gtk.Box(spacing=12)
        bar.set_name("playerbar")
        bar.set_border_width(10)
        self.now_cover = Gtk.Image()
        self.now_cover.set_size_request(52, 52)
        bar.pack_start(self.now_cover, False, False, 0)
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        text.set_valign(Gtk.Align.CENTER)
        text.set_size_request(200, -1)
        self.now_title = Gtk.Label(label="Nothing playing")
        self.now_title.set_name("nowtitle")
        self.now_title.set_xalign(0)
        self.now_title.set_ellipsize(Pango.EllipsizeMode.END)
        self.now_artist = Gtk.Label()
        self.now_artist.set_name("nowartist")
        self.now_artist.set_xalign(0)
        self.now_artist.set_ellipsize(Pango.EllipsizeMode.END)
        text.pack_start(self.now_title, False, False, 0)
        text.pack_start(self.now_artist, False, False, 0)
        bar.pack_start(text, False, False, 0)
        self.prev_btn = SkipButton(False, "Previous (Ctrl+Left)", self.previous)
        self.play_btn = HexButton(self.play_pause)
        self.play_btn.set_playing(False)
        self.next_btn = SkipButton(True, "Next (Ctrl+Right)", self.next)
        for w in (self.prev_btn, self.play_btn, self.next_btn):
            w.set_valign(Gtk.Align.CENTER)
            bar.pack_start(w, False, False, 0)
        self.elapsed = Gtk.Label(label="0:00")
        self.elapsed.set_name("timelabel")
        self.seek = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 1, 1)
        self.seek.set_draw_value(False)
        self.seek.connect("button-press-event", lambda *_a: setattr(self, "_seeking", True))
        self.seek.connect("button-release-event", self._on_seek_release)
        self.total = Gtk.Label(label="0:00")
        self.total.set_name("timelabel")
        bar.pack_start(self.elapsed, False, False, 0)
        bar.pack_start(self.seek, True, True, 0)
        bar.pack_start(self.total, False, False, 0)
        self.shuffle_btn = self._btn("Shuffle", "Shuffle (S)", lambda: self.set_shuffle(self.shuffle_btn.get_active(), from_button=True), name="transport", toggle=True)
        self.repeat_btn = self._btn("Repeat: Off", "Repeat (R): off, all, one", self.cycle_repeat, name="transport")
        bar.pack_start(self.shuffle_btn, False, False, 0)
        bar.pack_start(self.repeat_btn, False, False, 0)
        self.volume_scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 1, 0.05)
        self.volume_scale.set_draw_value(False)
        self.volume_scale.set_size_request(90, -1)
        self.volume_scale.set_tooltip_text("Volume")
        self.volume_scale.set_value(0.8)
        self.volume_scale.connect("value-changed", lambda s: self.set_volume(s.get_value(), from_scale=True))
        bar.pack_start(self.volume_scale, False, False, 0)
        return bar

    def _pop(self, widget, menu):
        menu.attach_to_widget(widget, None)
        menu.connect("deactivate", lambda m: GLib.idle_add(m.destroy))
        menu.popup_at_widget(widget, Gdk.Gravity.SOUTH_WEST, Gdk.Gravity.NORTH_WEST, None)

    def _item(self, menu, label, fn, ok=True):
        it = Gtk.MenuItem(label=label)
        it.set_sensitive(ok)
        it.connect("activate", lambda *_a: fn())
        menu.append(it)
        return it

    def _more_menu(self):
        m = Gtk.Menu()
        self._item(m, "Add a Folder to the Library…", self.add_library_folder)
        dirs = self.settings.get("dirs") or []
        for d in dirs:
            self._item(m, f"Stop Using {d.replace(os.path.expanduser('~'), '~', 1)}", lambda d=d: self.remove_library_folder(d))
        m.append(Gtk.SeparatorMenuItem())
        self._item(m, "Read the Library Again", self.rescan)
        m.show_all()
        return m

    # ---------- the library ----------

    def library_dirs(self):
        music = os.path.join(os.path.expanduser("~"), "Music")
        return [music] + [d for d in self.settings.get("dirs") or [] if d != music]

    def rescan(self):
        self.status_label.set_text("Reading your music…")
        dirs = self.library_dirs()

        def progress(p):
            GLib.idle_add(lambda: self.status_label.set_text(f"Reading your music… {p['done']} of {p['total']}") and False)

        def work():
            try:
                r = library.scan(dirs, progress)
                GLib.idle_add(self._scanned, r["tracks"], None)
            except library.EngineError as e:
                GLib.idle_add(self._scanned, None, str(e))
        threading.Thread(target=work, daemon=True).start()

    def _scanned(self, tracks, error):
        if error:
            self.status_label.set_text(error)
            return False
        self.tracks = tracks
        known = {t["path"]: t for t in tracks}
        for p, t in self.by_path.items():
            known.setdefault(p, t)  # songs opened from elsewhere stay known
        self.by_path = known
        n_albums = len({(t["album_artist"].lower(), t["album"].lower()) for t in tracks if t["album"]})
        self.status_label.set_text(f"{len(tracks)} songs, {n_albums} albums" if tracks else "")
        self._fill_library()
        if not self.nav.get_selected_row():
            self.nav.select_row(self.nav.get_row_at_index(1 if tracks else 4 if self.queue else 0))
        elif self.stack.get_visible_child_name() == "empty" and tracks:
            self.show_view(self.nav.get_selected_row().key)
        return False

    def _fill_library(self):
        self.songs_store.clear()
        for t in sorted(self.tracks, key=lambda t: (t["title"].lower(), t["artist"].lower())):
            self.songs_store.append(self._row(t))
        for c in self.albums_box.get_children():
            c.destroy()
        self.albums = {}
        for t in self.tracks:
            key = (t["album_artist"] or "Unknown artist", t["album"] or "Other songs")
            self.albums.setdefault(key, []).append(t)
        for key in sorted(self.albums, key=lambda k: (k[1].lower(), k[0].lower())):
            songs = self.albums[key]
            child = Gtk.FlowBoxChild()
            child.key = key
            v = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
            v.pack_start(self._cover_image(next((s["cover"] for s in songs if s.get("cover")), None), COVER), False, False, 0)
            for text, cls in ((key[1], "albumtitle"), (key[0], "albumartist")):
                l = Gtk.Label(label=text)
                l.get_style_context().add_class(cls)
                l.set_ellipsize(Pango.EllipsizeMode.END)
                l.set_max_width_chars(18)
                l.set_xalign(0)
                v.pack_start(l, False, False, 0)
            child.add(v)
            child.set_tooltip_text(f"{key[1]} — {key[0]}")
            self.albums_box.add(child)
        self.albums_box.show_all()
        for c in self.artists_list.get_children():
            c.destroy()
        artists = {}
        for t in self.tracks:
            artists.setdefault(t["artist"] or "Unknown artist", []).append(t)
        for name in sorted(artists, key=str.lower):
            row = Gtk.ListBoxRow()
            row.key = name
            row.get_style_context().add_class("artistrow")
            h = Gtk.Box(spacing=12)
            h.set_border_width(8)
            l = Gtk.Label(label=name)
            l.set_xalign(0)
            n = Gtk.Label(label=f"{len(artists[name])} songs")
            n.get_style_context().add_class("albumartist")
            h.pack_start(l, True, True, 0)
            h.pack_end(n, False, False, 0)
            row.add(h)
            self.artists_list.add(row)
        self.artists_list.show_all()
        self._fill_playlists()

    def _cover_image(self, path, size):
        img = Gtk.Image()
        pix = None
        if path:
            try:
                pix = GdkPixbuf.Pixbuf.new_from_file_at_scale(path, size, size, True)
            except GLib.Error:
                pix = None
        if pix:
            img.set_from_pixbuf(pix)
        else:
            img.set_from_icon_name("audio-x-generic", Gtk.IconSize.DIALOG)
            img.set_pixel_size(size // 2)
        img.set_size_request(size, size)
        return img

    # ---------- search ----------

    def _q(self):
        return self.search.get_text().strip().lower()

    def _matches(self, *fields):
        q = self._q()
        return not q or all(any(w in (f or "").lower() for f in fields) for w in q.split())

    def _song_visible(self, model, it, _data=None):
        return self._matches(model[it][1], model[it][2], model[it][3])

    def _album_visible(self, child):
        return self._matches(child.key[0], child.key[1])

    def _artist_visible(self, row):
        return self._matches(row.key)

    def _apply_search(self):
        self.songs_filter.refilter()
        self.albums_box.invalidate_filter()
        self.artists_list.invalidate_filter()
        if self._q() and self.stack.get_visible_child_name() not in ("songs", "albums", "artists"):
            self.nav.select_row(self.nav.get_row_at_index(0))

    # ---------- views ----------

    def show_view(self, key):
        if not self.tracks and key in ("songs", "albums", "artists"):
            self.stack.set_visible_child_name("empty")
            return
        if key == "queue":
            self._fill_queue()
        if key == "playlists":
            self._fill_playlists()
        self.stack.set_visible_child_name(key)
        self.view_history = []

    def go_back(self):
        if self.view_history:
            self.stack.set_visible_child_name(self.view_history.pop())

    def _detail(self, title, sub, cover, songs, actions, numbered=True):
        if self.stack.get_visible_child_name() != "detail":
            self.view_history.append(self.stack.get_visible_child_name())
        self.detail_title.set_text(title)
        self.detail_sub.set_text(sub)
        new = self._cover_image(cover, COVER)
        self.detail_cover.set_from_pixbuf(new.get_pixbuf()) if new.get_pixbuf() else self.detail_cover.set_from_icon_name("audio-x-generic", Gtk.IconSize.DIALOG)
        self.detail_cover.set_pixel_size(COVER // 2)
        for c in self.detail_actions.get_children():
            c.destroy()
        for i, (label, fn) in enumerate(actions):
            b = Gtk.Button(label=label)
            if i == 0:
                b.get_style_context().add_class("primary")
            else:
                b.set_name("toolbtn")
            b.connect("clicked", lambda *_a, f=fn: f())
            self.detail_actions.pack_start(b, False, False, 0)
        self.detail_actions.show_all()
        self.detail_store.clear()
        for i, t in enumerate(songs):
            self.detail_store.append(self._row(t, (t.get("track") or i + 1) if numbered else i + 1))
        self.detail_songs = songs
        self.stack.set_visible_child_name("detail")

    def _length(self, songs):
        total = sum(t.get("duration") or 0 for t in songs)
        return f"{len(songs)} songs, {fmt_time(total)}"

    def show_album(self, key):
        songs = self.albums.get(key, [])
        years = sorted({t["year"] for t in songs if t.get("year")})
        sub = f"{key[0]}{' · ' + str(years[0]) if years else ''} · {self._length(songs)}"
        self.current_detail = ("album", key)
        self._detail(key[1], sub, next((s["cover"] for s in songs if s.get("cover")), None), songs,
                     [("Play", lambda: self.play_list(songs, 0)), ("Shuffle", lambda: self.play_list(songs, None, shuffle=True)),
                      ("Add to Up Next", lambda: self.add_to_queue(songs))])

    def show_artist(self, name):
        songs = sorted([t for t in self.tracks if (t["artist"] or "Unknown artist") == name],
                       key=lambda t: ((t["album"] or "").lower(), t.get("disc") or 1, t.get("track") or 0))
        self.current_detail = ("artist", name)
        self._detail(name, self._length(songs), next((s["cover"] for s in songs if s.get("cover")), None), songs,
                     [("Play", lambda: self.play_list(songs, 0)), ("Shuffle", lambda: self.play_list(songs, None, shuffle=True))], numbered=False)

    # ---------- playlists ----------

    def _fill_playlists(self):
        try:
            lists = library.run("playlists")
        except library.EngineError as e:
            self.status_label.set_text(str(e))
            lists = []
        for c in self.playlists_list.get_children():
            c.destroy()
        if not lists:
            row = Gtk.ListBoxRow()
            row.key = None
            row.set_activatable(False)
            l = Gtk.Label(label="No playlists yet. Press New Playlist, or Save as Playlist in Up Next.")
            l.get_style_context().add_class("empty")
            l.set_line_wrap(True)
            row.add(l)
            self.playlists_list.add(row)
        for p in lists:
            row = Gtk.ListBoxRow()
            row.key = p["name"]
            h = Gtk.Box(spacing=12)
            h.set_border_width(10)
            l = Gtk.Label(label=p["name"])
            l.set_xalign(0)
            n = Gtk.Label(label=f"{p['count']} songs")
            n.get_style_context().add_class("albumartist")
            h.pack_start(l, True, True, 0)
            h.pack_end(n, False, False, 0)
            row.add(h)
            self.playlists_list.add(row)
        self.playlists_list.show_all()
        self.playlist_names = [p["name"] for p in lists]

    def _playlist_songs(self, name):
        paths = library.run("playlist_get", name=name)
        missing = [p for p in paths if p not in self.by_path and os.path.isfile(p)]
        if missing:
            for t in library.tags(missing):
                self.by_path[t["path"]] = t
        return [self.by_path[p] for p in paths if p in self.by_path]

    def show_playlist(self, name):
        if not name:
            return
        try:
            songs = self._playlist_songs(name)
        except library.EngineError as e:
            self.notify(str(e))
            return
        self.current_detail = ("playlist", name)
        self._detail(name, self._length(songs), next((s["cover"] for s in songs if s.get("cover")), None), songs,
                     [("Play", lambda: self.play_list(songs, 0)), ("Shuffle", lambda: self.play_list(songs, None, shuffle=True)),
                      ("Rename", lambda: self.rename_playlist(name)), ("Delete", lambda: self.delete_playlist(name))], numbered=False)

    def ask_name(self, title, default=""):
        d = Gtk.Dialog(title=title, transient_for=self, modal=True)
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Save", Gtk.ResponseType.OK)
        d.set_default_response(Gtk.ResponseType.OK)
        e = Gtk.Entry()
        e.set_text(default)
        e.set_activates_default(True)
        box = d.get_content_area()
        box.set_border_width(16)
        box.pack_start(e, False, False, 0)
        d.show_all()
        r = d.run()
        text = e.get_text().strip()
        d.destroy()
        return text if r == Gtk.ResponseType.OK and text else None

    def save_playlist(self, name, songs):
        try:
            library.run("playlist_put", name=name, paths=[t["path"] for t in songs])
        except library.EngineError as e:
            self.notify(str(e))
            return False
        self._fill_playlists()
        return True

    def new_playlist(self, songs=()):
        name = self.ask_name("New Playlist", "My Playlist")
        if name and self.save_playlist(name, list(songs)):
            self.notify(f"Made the playlist {name}.")
            return name
        return None

    def add_to_playlist(self, name, songs):
        try:
            current = self._playlist_songs(name)
        except library.EngineError:
            current = []
        if self.save_playlist(name, current + list(songs)):
            self.notify(f"Added {len(songs)} to {name}.")

    def rename_playlist(self, name):
        to = self.ask_name("Rename Playlist", name)
        if to and to != name:
            try:
                library.run("playlist_rename", name=name, to=to)
            except library.EngineError as e:
                self.notify(str(e))
                return
            self._fill_playlists()
            self.show_playlist(to)

    def delete_playlist(self, name):
        d = Gtk.MessageDialog(transient_for=self, flags=0, message_type=Gtk.MessageType.QUESTION, buttons=Gtk.ButtonsType.NONE,
                              text=f"Delete the playlist {name}?")
        d.format_secondary_text("The songs themselves stay in your library.")
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Delete", Gtk.ResponseType.OK)
        ok = d.run() == Gtk.ResponseType.OK
        d.destroy()
        if ok:
            try:
                library.run("playlist_delete", name=name)
            except library.EngineError as e:
                self.notify(str(e))
                return
            self.nav.select_row(self.nav.get_row_at_index(3))
            self.show_view("playlists")

    def queue_to_playlist(self):
        if self.queue:
            self.new_playlist([self.by_path[p] for p in self.queue if p in self.by_path])

    # ---------- right click on songs ----------

    def _selected_songs(self, view):
        model, paths = view.get_selection().get_selected_rows()
        return [self.by_path[model[p][5]] for p in paths if model[p][5] in self.by_path]

    def _on_list_click(self, view, ev):
        if ev.button != 3:
            return False
        hit = view.get_path_at_pos(int(ev.x), int(ev.y))
        if hit and not view.get_selection().path_is_selected(hit[0]):
            view.get_selection().unselect_all()
            view.get_selection().select_path(hit[0])
        songs = self._selected_songs(view)
        if not songs:
            return True
        m = Gtk.Menu()
        if view is self.queue_view:
            self._item(m, "Play", lambda: self.play_index(self.queue.index(songs[0]["path"])))
            self._item(m, "Remove from Up Next", lambda: self.remove_from_queue(songs))
            self._item(m, "Move Up", lambda: self.move_in_queue(songs[0]["path"], -1))
            self._item(m, "Move Down", lambda: self.move_in_queue(songs[0]["path"], 1))
        else:
            self._item(m, "Play", lambda: self.play_list(songs, 0))
            self._item(m, "Play Next", lambda: self.play_next(songs))
            self._item(m, "Add to Up Next", lambda: self.add_to_queue(songs))
        sub = Gtk.Menu()
        self._item(sub, "New Playlist…", lambda: self.new_playlist(songs))
        names = getattr(self, "playlist_names", [])
        if names:
            sub.append(Gtk.SeparatorMenuItem())
        for n in names:
            self._item(sub, n, lambda n=n: self.add_to_playlist(n, songs))
        pl = Gtk.MenuItem(label="Add to Playlist")
        pl.set_submenu(sub)
        m.append(pl)
        d = getattr(self, "current_detail", None)
        if view is self.detail_view and d and d[0] == "playlist":
            self._item(m, "Remove from This Playlist", lambda: self._remove_from_playlist(d[1], songs))
        t = songs[0]
        if t.get("album") and view is not self.detail_view:
            self._item(m, "Go to Album", lambda: self.show_album((t["album_artist"] or "Unknown artist", t["album"])))
        if view is not self.detail_view:
            self._item(m, "Go to Artist", lambda: self.show_artist(t["artist"] or "Unknown artist"))
        m.show_all()
        m.popup_at_pointer(ev)
        return True

    def _remove_from_playlist(self, name, songs):
        drop = {t["path"] for t in songs}
        keep = [t for t in self._playlist_songs(name) if t["path"] not in drop]
        if self.save_playlist(name, keep):
            self.show_playlist(name)

    # ---------- the queue ----------

    def current(self):
        if 0 <= self.pos < len(self.queue):
            return self.by_path.get(self.queue[self.pos]) or {"path": self.queue[self.pos], "title": os.path.basename(self.queue[self.pos])}
        return None

    def _play_from_view(self, view, path):
        model = view.get_model()
        if view is self.queue_view:
            self.play_index(path.get_indices()[0])
            return
        songs = [self.by_path[r[5]] for r in model if r[5] in self.by_path]
        idx = next((i for i, t in enumerate(songs) if t["path"] == model[path][5]), 0)
        self.play_list(songs, idx)

    def play_list(self, songs, start=0, shuffle=False):
        if not songs:
            return
        paths = [t["path"] for t in songs]
        for t in songs:
            self.by_path.setdefault(t["path"], t)
        if shuffle or self.shuffle():
            first = paths[start] if start is not None else random.choice(paths)
            rest = [p for p in paths if p != first]
            random.shuffle(rest)
            self.unshuffled = paths
            self.queue = [first] + rest
            self._set_shuffle_button(True)
            self.play_index(0)
        else:
            self.unshuffled = None
            self.queue = paths
            self.play_index(start or 0)

    def play_index(self, i):
        if not (0 <= i < len(self.queue)):
            return
        self.pos = i
        self._resume_at = None
        self.player.load(self.queue[i])
        self.player.play()
        self._now_playing_changed()

    def _next_index(self):
        if not self.queue:
            return None
        if self.repeat == "one":
            return self.pos
        if self.pos + 1 < len(self.queue):
            return self.pos + 1
        return 0 if self.repeat == "all" else None

    def _update_next(self):
        n = self._next_index()
        self.player.set_next(self.queue[n] if n is not None else None)

    def next(self):
        if not self.queue:
            return
        n = self.pos + 1 if self.pos + 1 < len(self.queue) else (0 if self.repeat != "off" else None)
        if n is None:
            self.stop()
            return
        self.play_index(n)

    def previous(self):
        if not self.queue:
            return
        if (self.player.get_position() or 0) > 3:
            self.seek_to(0)
            return
        self.play_index(self.pos - 1 if self.pos > 0 else (len(self.queue) - 1 if self.repeat != "off" else 0))

    def play_next(self, songs):
        paths = [t["path"] for t in songs]
        for t in songs:
            self.by_path.setdefault(t["path"], t)
        if not self.queue:
            self.play_list(songs, 0)
            return
        self.queue[self.pos + 1:self.pos + 1] = paths
        self._queue_changed()
        self.notify(f"{songs[0]['title']} plays next." if len(songs) == 1 else f"{len(songs)} songs play next.")

    def add_to_queue(self, songs):
        for t in songs:
            self.by_path.setdefault(t["path"], t)
        if not self.queue:
            self.play_list(songs, 0)
            return
        self.queue.extend(t["path"] for t in songs)
        self._queue_changed()
        self.notify(f"Added {len(songs)} to Up Next.")

    def remove_from_queue(self, songs):
        drop = {t["path"] for t in songs}
        cur = self.current()
        playing_removed = cur and cur["path"] in drop
        keep = [p for p in self.queue if p not in drop]
        if playing_removed:
            self.player.stop()
            self.queue, self.pos = keep, min(self.pos, len(keep) - 1)
            if self.queue:
                self.play_index(max(self.pos, 0))
            else:
                self._now_playing_changed()
        else:
            self.queue = keep
            self.pos = keep.index(cur["path"]) if cur else -1
        self._queue_changed()

    def move_in_queue(self, path, step):
        i = self.queue.index(path)
        j = i + step
        if 0 <= j < len(self.queue):
            cur = self.current()
            self.queue[i], self.queue[j] = self.queue[j], self.queue[i]
            self.pos = self.queue.index(cur["path"]) if cur else -1
            self._queue_changed()

    def clear_queue(self):
        cur = self.current()
        self.queue = [cur["path"]] if cur else []
        self.pos = 0 if cur else -1
        self.unshuffled = None
        self._queue_changed()

    def _queue_reordered(self):
        """After a drag in Up Next: the list's order is the queue's."""
        order = [r[5] for r in self.queue_store]
        if sorted(order) == sorted(self.queue) and order != self.queue:
            cur = self.current()
            self.queue = order
            self.pos = order.index(cur["path"]) if cur else -1
            self._update_next()
            self._fill_queue()
        return False

    def _queue_changed(self):
        self._update_next()
        if self.stack.get_visible_child_name() == "queue":
            self._fill_queue()
        self.mpris.changed("CanGoNext", "CanGoPrevious")

    def _fill_queue(self):
        self.queue_store.clear()
        for i, p in enumerate(self.queue):
            t = self.by_path.get(p) or {"path": p, "title": os.path.basename(p)}
            row = self._row(t, i + 1)
            row[6] = 700 if i == self.pos else 400
            self.queue_store.append(row)

    # ---------- shuffle, repeat, volume ----------

    def shuffle(self):
        return self.unshuffled is not None

    def _set_shuffle_button(self, on):
        self._in_set = True
        self.shuffle_btn.set_active(on)
        self._in_set = False

    def set_shuffle(self, on, from_button=False):
        if from_button and getattr(self, "_in_set", False):
            return
        if on == self.shuffle():
            self._set_shuffle_button(on)
            return
        cur = self.current()
        if on:
            self.unshuffled = list(self.queue)
            rest = [p for i, p in enumerate(self.queue) if i != self.pos]
            random.shuffle(rest)
            self.queue = ([cur["path"]] if cur else []) + rest
            self.pos = 0 if cur else -1
        else:
            self.queue = self.unshuffled or self.queue
            self.unshuffled = None
            self.pos = self.queue.index(cur["path"]) if cur and cur["path"] in self.queue else self.pos
        self._set_shuffle_button(on)
        self._queue_changed()
        self.mpris.changed("Shuffle")

    def loop(self):
        return {"off": "None", "all": "Playlist", "one": "Track"}[self.repeat]

    def set_loop(self, status):
        self.set_repeat({"None": "off", "Playlist": "all", "Track": "one"}[status])

    def set_repeat(self, mode):
        self.repeat = mode
        self.repeat_btn.set_label({"off": "Repeat: Off", "all": "Repeat: All", "one": "Repeat: One"}[mode])
        self._update_next()
        self.mpris.changed("LoopStatus")

    def cycle_repeat(self):
        self.set_repeat({"off": "all", "all": "one", "one": "off"}[self.repeat])

    def volume(self):
        return self.player.get_volume()

    def set_volume(self, v, from_scale=False):
        self.player.set_volume(v)
        if not from_scale:
            self.volume_scale.set_value(v)
        self.mpris.changed("Volume")

    # ---------- playing ----------

    def play(self):
        if self.pos < 0 and self.queue:
            self.play_index(0)
            return
        if self.current():
            self.player.play()

    def pause(self):
        self.player.pause()

    def play_pause(self):
        if self._playing:
            self.pause()
        else:
            self.play()

    def stop(self):
        self.player.stop()
        self._show_playing(False)
        self.mpris.changed("PlaybackStatus")

    def status(self):
        if self._playing:
            return "Playing"
        return "Paused" if self.current() and self.player.state() == Gst.State.PAUSED else "Stopped"

    def position(self):
        return self.player.get_position()

    def seek_to(self, seconds):
        self.player.seek(seconds)
        self.mpris.seeked(seconds)

    def _closed(self):
        GLib.source_remove(self.tick_id)
        self.player.stop()
        self.mpris.close()

    def present(self):
        super().present()

    def quit(self):
        self._save_state()
        self.destroy()

    def _now_playing_changed(self):
        t = self.current()
        self.now_title.set_text(t["title"] if t else "Nothing playing")
        self.now_artist.set_text(" · ".join(x for x in ((t or {}).get("artist"), (t or {}).get("album")) if x))
        cover = self._cover_image((t or {}).get("cover"), 52)
        if cover.get_pixbuf():
            self.now_cover.set_from_pixbuf(cover.get_pixbuf())
        else:
            self.now_cover.set_from_icon_name("audio-x-generic", Gtk.IconSize.DIALOG)
        self.set_title(f"{t['title']} — Music" if t else "Music")
        dur = (t or {}).get("duration")
        self.seek.set_range(0, max(dur or 1, 1))
        self.total.set_text(fmt_time(dur) if dur else "0:00")
        self._update_next()
        self._refresh_bold()
        if self.stack.get_visible_child_name() == "queue":
            self._fill_queue()
        self.mpris.changed("Metadata", "PlaybackStatus", "CanGoNext", "CanGoPrevious", "CanPlay", "CanPause", "CanSeek")

    def _refresh_bold(self):
        cur = (self.current() or {}).get("path")
        for store in (self.songs_store, self.detail_store):
            for row in store:
                row[6] = 700 if row[5] == cur else 400

    def _on_next_started(self):
        """Gapless: playbin went on to the next song by itself."""
        n = self._next_index()
        if n is not None:
            self.pos = n
        self._now_playing_changed()

    def _on_eos(self):
        n = self._next_index()
        if n is None:
            self.player.stop()
            self._show_playing(False)
            self.mpris.changed("PlaybackStatus")
        else:
            self.play_index(n)

    def _on_player_state(self, state):
        playing = state == Gst.State.PLAYING
        if playing != self._playing:
            self._show_playing(playing)
            self.mpris.changed("PlaybackStatus")
        if state == Gst.State.PAUSED and self._resume_at:
            at, self._resume_at = self._resume_at, None
            GLib.idle_add(lambda: self.player.seek(at) and False)

    def _on_player_error(self, message):
        t = self.current()
        name = t["title"] if t else "This song"
        self.notify(f"{name} can't be played ({message}). Going on to the next one.")
        n = self._next_index()
        if n is not None and n != self.pos and self.repeat != "one":
            self.play_index(n)
        else:
            self.stop()

    def _show_playing(self, playing):
        self._playing = playing
        magic.level("nexa-music", 0.7 if playing else 0.0)
        self.play_btn.set_playing(playing)

    def _tick(self):
        if self._playing:
            magic.level("nexa-music", 0.7)  # its background moves with the music
        if not self._seeking and self.current():
            pos = self.player.get_position()
            dur = self.player.get_duration() or (self.current() or {}).get("duration")
            if dur:
                self.seek.set_range(0, max(dur, 1))
                self.total.set_text(fmt_time(dur))
            if pos is not None:
                self.seek.set_value(pos)
                self.elapsed.set_text(fmt_time(pos))
        self.state_saved += 1
        if self.state_saved >= SAVE_EVERY * 2:
            self.state_saved = 0
            self._save_state()
        return True

    def _on_seek_release(self, scale, _ev):
        self._seeking = False
        self.seek_to(scale.get_value())
        return False

    # ---------- remembering ----------

    def _state_path(self):
        return os.path.join(data_dir(), "state.json")

    def _save_state(self):
        _save(self._state_path(), {
            "queue": self.queue[:5000], "pos": self.pos, "position": self.player.get_position() or 0,
            "unshuffled": self.unshuffled[:5000] if self.unshuffled else None, "repeat": self.repeat,
            "volume": self.player.get_volume(),
            "songs": {p: self.by_path[p] for p in self.queue[:5000] if p in self.by_path},
        })

    def _restore_state(self):
        s = _load(self._state_path(), {})
        queue = [p for p in s.get("queue") or [] if isinstance(p, str) and os.path.isfile(p)]
        for p, t in (s.get("songs") or {}).items():
            if isinstance(t, dict) and p in queue:
                self.by_path.setdefault(p, t)
        self.set_volume(float(s.get("volume") or 0.8))
        self.set_repeat(s.get("repeat") if s.get("repeat") in ("off", "all", "one") else "off")
        if not queue:
            return
        self.queue = queue
        u = s.get("unshuffled")
        self.unshuffled = [p for p in u if p in queue] if isinstance(u, list) else None
        self._set_shuffle_button(self.unshuffled is not None)
        pos = s.get("pos")
        self.pos = pos if isinstance(pos, int) and 0 <= pos < len(queue) else 0
        self.player.load(self.queue[self.pos])
        self._resume_at = float(s.get("position") or 0) or None
        self.player.pause()  # ready where it was left, not playing
        self._now_playing_changed()

    # ---------- opening ----------

    def on_open(self):
        d = Gtk.FileChooserDialog(title="Play Files or a Folder", transient_for=self, action=Gtk.FileChooserAction.OPEN)
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Play", Gtk.ResponseType.OK)
        d.set_select_multiple(True)
        f = Gtk.FileFilter()
        f.set_name("Songs and playlists")
        for ext in AUDIO_EXTS + (".m3u", ".m3u8"):
            f.add_pattern(f"*{ext}")
            f.add_pattern(f"*{ext.upper()}")
        d.add_filter(f)
        music = os.path.join(os.path.expanduser("~"), "Music")
        if os.path.isdir(music):
            d.set_current_folder(music)
        paths = d.get_filenames() if d.run() == Gtk.ResponseType.OK else []
        d.destroy()
        if paths:
            self.open_paths(paths)

    def open_paths(self, paths):
        files = []
        for p in paths:
            p = os.path.abspath(p)
            if os.path.isdir(p):
                for base, dirs, names in os.walk(p):
                    dirs[:] = sorted(d for d in dirs if not d.startswith("."))
                    files += [os.path.join(base, n) for n in sorted(names) if n.lower().endswith(AUDIO_EXTS)]
            elif p.lower().endswith((".m3u", ".m3u8")):
                try:
                    files += [x for x in library.run("read_m3u", path=p) if os.path.isfile(x)]
                except library.EngineError as e:
                    self.notify(str(e))
            elif os.path.isfile(p):
                files.append(p)
        if not files:
            self.notify("No songs there.")
            return False
        try:
            songs = library.tags(files[:5000])
        except library.EngineError as e:
            self.notify(str(e))
            return False
        self.play_list(songs, 0)
        self.nav.select_row(self.nav.get_row_at_index(4))
        return True

    def add_library_folder(self):
        d = Gtk.FileChooserDialog(title="Add a Folder to the Library", transient_for=self, action=Gtk.FileChooserAction.SELECT_FOLDER)
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Add", Gtk.ResponseType.OK)
        path = d.get_filename() if d.run() == Gtk.ResponseType.OK else None
        d.destroy()
        if path:
            dirs = [x for x in self.settings.get("dirs") or [] if x != path] + [path]
            self.settings["dirs"] = dirs
            _save(config_path(), self.settings)
            self.rescan()

    def remove_library_folder(self, path):
        self.settings["dirs"] = [x for x in self.settings.get("dirs") or [] if x != path]
        _save(config_path(), self.settings)
        self.rescan()

    # ---------- messages, keys, drops ----------

    def notify(self, text):
        self.status_label.set_text(text)

    def _on_key(self, _w, ev):
        name = Gdk.keyval_name(ev.keyval) or ""
        ctrl = ev.state & Gdk.ModifierType.CONTROL_MASK
        alt = ev.state & Gdk.ModifierType.MOD1_MASK
        if isinstance(self.get_focus(), Gtk.Entry):
            if name == "Escape":
                self.search.set_text("")
                self.set_focus(None)
                return True
            return False
        if ctrl and name.lower() == "o":
            self.on_open()
        elif ctrl and name.lower() == "f":
            self.search.grab_focus()
        elif ctrl and name.lower() in ("q", "w"):
            self.close()
        elif ctrl and name == "Right":
            self.next()
        elif ctrl and name == "Left":
            self.previous()
        elif alt and name == "Left":
            self.go_back()
        elif name == "space" and not isinstance(self.get_focus(), Gtk.TreeView):
            self.play_pause()
        elif name.lower() == "s" and not ctrl:
            self.set_shuffle(not self.shuffle())
        elif name.lower() == "r" and not ctrl:
            self.cycle_repeat()
        else:
            return False
        return True

    def _on_drop(self, _w, ctx, _x, _y, data, _info, time_):
        paths = [Gio.File.new_for_uri(u).get_path() for u in data.get_uris() or []]
        self.open_paths([p for p in paths if p])
        Gtk.drag_finish(ctx, True, False, time_)


def main(argv):
    Gtk.init()
    win = MusicWindow([a for a in argv if not a.startswith("-")])
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
