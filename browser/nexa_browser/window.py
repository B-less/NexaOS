"""Nexa Browser: WebKitGTK with tabs, ad and tracker blocking (compiled by
nexa-webfilter, Rust), HTTPS-only mode, site permissions that ask first,
find in page, zoom per site, bookmarks, history, downloads, session
restore, private and Tor windows, developer tools, printing.

The engine is WebKit (C++); this file is the window around it."""
import os
import sys
import threading

# WebKit's GStreamer backend plays HLS (.m3u8: news, sports, social and live
# sites) only when asked. Without it those videos never start: the browser
# looks like Safari, so sites hand it HLS directly. Set before any WebView.
os.environ.setdefault("WEBKIT_GST_ENABLE_HLS_SUPPORT", "1")

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gtk, Gdk, Gio, GLib, Pango, WebKit2  # noqa: E402

from nexa_common.titlebar import attach_titlebar  # noqa: E402

from . import blocker, downloads, newtab, session, tor  # noqa: E402
from .bookmarks import Bookmarks  # noqa: E402
from .history import History  # noqa: E402
from .prefs import PERMISSIONS, Prefs, data_dir, host_of, site_of  # noqa: E402
from .urls import to_uri as _to_uri  # noqa: E402

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#urlentry { background-image: none; background-color: @nexa_field; color: @nexa_text_strong; border-radius: 14px; padding: 6px 12px; border: 1px solid @nexa_border; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#toolbtn:disabled { color: @nexa_disabled; }
#shield-on { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 6px; padding: 4px 10px; }
#shield-off { background-image: none; background-color: @nexa_surface_hi; color: @nexa_muted; border-radius: 6px; padding: 4px 10px; }
#private-badge { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 10px; padding: 2px 10px; font-weight: bold; }
popover { background-image: none; background-color: @nexa_surface; color: @nexa_text; border: 1px solid @nexa_border; }
popover list, popover row { background-image: none; background-color: transparent; color: @nexa_text; }
popover row:hover { background-color: @nexa_hover; }
popover progressbar progress { background-image: none; background-color: @nexa_accent; }
#menuitem { background-image: none; background-color: transparent; color: @nexa_text; border: none; padding: 6px 10px; }
#menuitem:hover { background-color: @nexa_hover; }
#panel-title { color: @nexa_text_strong; font-weight: bold; }
#panel-muted { color: @nexa_muted; font-size: 9pt; }
#panel-row { padding: 6px 4px; }
notebook > header { background-image: none; background-color: @nexa_surface; border: none; }
notebook > header tab { padding: 2px 6px; color: @nexa_muted; border: none; background-image: none; }
notebook > header tab:checked { color: @nexa_text_strong; background-color: @nexa_bg; border-radius: 8px 8px 0 0; }
#tabclose, #tabaudio { background-image: none; background-color: transparent; border: none; padding: 0 4px; min-height: 0; min-width: 0; color: @nexa_muted; }
#tabclose:hover, #tabaudio:hover { color: @nexa_text_strong; }
#findbar { background-image: none; background-color: @nexa_surface; padding: 6px; }
#findcount { color: @nexa_muted; }
infobar, infobar > revealer > box { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; }
"""

TOR_HOME_URL = "https://check.torproject.org/"  # first page says whether Tor is working
TOR_NOTE = ("Traffic from this window goes through the Tor network and nothing is saved. "
            "It hides your IP address from websites and your network, but it is not Tor Browser: "
            "sites can still fingerprint the browser, and anything you sign in to knows it is you.")
NEWTAB = "about:blank"
LOCAL_HOSTS = ("localhost", "127.0.0.1", "::1")

PREFS = Prefs()
BOOKMARKS = Bookmarks()
_history = None
_http_ok = set()  # sites you chose to visit without https (this run only)


def history():
    global _history
    if _history is None:
        _history = History()
    return _history


def _is_local(host):
    return (host in LOCAL_HOSTS or host.endswith(".local") or host.startswith(("10.", "192.168.", "169.254."))
            or any(host.startswith(f"172.{n}.") for n in range(16, 32)))


def _make_context(private, use_tor=False):
    """A private window keeps nothing: no cookies, cache, or history on disk.
    A Tor window is private and sends everything, DNS lookups included,
    through Tor's SOCKS port. A normal one keeps cookies (so sign-ins survive
    a restart) but refuses third-party cookies, which are mostly tracking."""
    if use_tor:
        manager = WebKit2.WebsiteDataManager.new_ephemeral()
        manager.set_network_proxy_settings(WebKit2.NetworkProxyMode.CUSTOM,
                                           WebKit2.NetworkProxySettings.new(tor.PROXY_URI, None))
        context = WebKit2.WebContext.new_with_website_data_manager(manager)
    elif private:
        context = WebKit2.WebContext.new_ephemeral()
    else:
        context = WebKit2.WebContext.get_default()
        if not getattr(context, "_nexa_ready", False):
            cookies = context.get_cookie_manager()
            os.makedirs(data_dir(), exist_ok=True)
            cookies.set_persistent_storage(os.path.join(data_dir(), "cookies.sqlite"),
                                           WebKit2.CookiePersistentStorage.SQLITE)
            cookies.set_accept_policy(WebKit2.CookieAcceptPolicy.NO_THIRD_PARTY)
            try:
                context.set_favicon_database_directory(os.path.join(data_dir(), "favicons"))
            except GLib.Error:
                pass
    if not getattr(context, "_nexa_ready", False):
        context._nexa_ready = True
        context.connect("download-started", _on_download_started)
        context.set_spell_checking_enabled(PREFS["spellcheck"] and not use_tor)
        langs = [l for l in GLib.get_language_names() if "_" in l and "." not in l and "@" not in l][:1] or ["en_US"]
        context.set_spell_checking_languages(langs)
    return context


def _make_settings(private, use_tor):
    s = WebKit2.Settings()
    want = {
        "enable-developer-extras": True,          # F12: the inspector
        "enable-mediasource": True,               # streamed video (YouTube and most sites)
        "enable-media-stream": not use_tor,       # camera/microphone, always asked first
        "enable-webrtc": not private,             # video calls; off in private and Tor (it can reveal your address)
        "enable-encrypted-media": False,          # no DRM module exists for WebKitGTK; saying so beats a broken player
        "enable-dns-prefetching": False,          # can leak where you're about to go
        "javascript-can-open-windows-automatically": False,  # pop-ups only when you click
        "enable-smooth-scrolling": True,
        "enable-back-forward-cache": True,
        "enable-page-cache": True,
    }
    for prop, value in want.items():
        if s.find_property(prop):
            s.set_property(prop, value)
    return s


# ---- blocking: one compiled rule set, shared by every window ----------------

_ucms = []
_filter_store = None
_current_filter = None


def _blocking_dir():
    return os.path.join(data_dir(), "webkit-filters")


def refresh_blocking(done=None):
    """Compile (in a thread, cached) and apply to every open window."""
    def work():
        try:
            ident, rules, _summary = blocker.compile_rules(PREFS)
            GLib.idle_add(apply, ident, rules)
        except blocker.BlockerError as e:
            print(f"nexa-browser: blocking unavailable: {e}", file=sys.stderr)
            GLib.idle_add(lambda: done and done(False))

    def apply(ident, rules):
        global _filter_store
        if _filter_store is None:
            os.makedirs(_blocking_dir(), exist_ok=True)
            _filter_store = WebKit2.UserContentFilterStore.new(_blocking_dir())

        def saved(store, res):
            global _current_filter
            try:
                _current_filter = store.save_finish(res)
            except GLib.Error as e:
                print(f"nexa-browser: WebKit refused the rules: {e.message}", file=sys.stderr)
                if done:
                    done(False)
                return
            for ucm in _ucms:
                _apply_to(ucm)
            if done:
                done(True)
        _filter_store.save(ident, GLib.Bytes.new(rules), None, saved)
        return False

    threading.Thread(target=work, daemon=True).start()


def _apply_to(ucm):
    ucm.remove_all_filters()
    if PREFS["block_ads"] and _current_filter is not None:
        ucm.add_filter(_current_filter)


# ---- one tab -----------------------------------------------------------------

class Tab:
    def __init__(self, win, view):
        self.win, self.view = win, view
        self.newtab = False
        self.upgraded = {}  # https url -> the http url it replaced
        self.page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.infobar = Gtk.InfoBar(show_close_button=True)
        self.infobar.set_no_show_all(True)
        self.infobar.connect("response", lambda bar, _r: bar.hide())
        self.info_label = Gtk.Label(xalign=0, wrap=True)
        self.infobar.get_content_area().pack_start(self.info_label, True, True, 0)
        self.page.pack_start(self.infobar, False, False, 0)
        self.page.pack_start(view, True, True, 0)

        head = Gtk.Box(spacing=4)
        self.icon = Gtk.Image.new_from_icon_name("text-html-symbolic", Gtk.IconSize.MENU)
        self.spinner = Gtk.Spinner()
        self.title = Gtk.Label(label="New tab", ellipsize=Pango.EllipsizeMode.END, width_chars=14, max_width_chars=18, xalign=0)
        self.audio = Gtk.Button(label="🔊")
        self.audio.set_name("tabaudio")
        self.audio.set_no_show_all(True)
        self.audio.set_tooltip_text("Mute this tab")
        self.audio.connect("clicked", lambda *_: self.toggle_mute())
        close = Gtk.Button(label="✕")
        close.set_name("tabclose")
        close.set_tooltip_text("Close tab (Ctrl+W)")
        close.connect("clicked", lambda *_: win.close_tab(self))
        for w in (self.icon, self.spinner, self.title, self.audio, close):
            head.pack_start(w, False, False, 0)
        head.set_child_packing(self.title, True, True, 0, Gtk.PackType.START)
        self.head = Gtk.EventBox()
        self.head.add(head)
        self.head.connect("button-press-event", self._on_head_click)
        self.head.show_all()
        self.spinner.hide()

        for sig, fn in (("notify::uri", win._on_uri_changed), ("notify::title", self._on_title),
                        ("notify::estimated-load-progress", win._on_progress), ("notify::is-loading", self._on_loading),
                        ("load-changed", win._on_load_changed), ("load-failed", self._on_load_failed),
                        ("decide-policy", win._on_decide_policy), ("create", win._on_create),
                        ("permission-request", self._on_permission), ("enter-fullscreen", win._on_fullscreen, ),
                        ("leave-fullscreen", win._on_unfullscreen), ("notify::is-playing-audio", self._on_audio),
                        ("notify::favicon", self._on_favicon), ("close", lambda *_: win.close_tab(self)),
                        ("button-press-event", self._on_mouse)):
            view.connect(sig, fn)

    # head (the tab itself)
    def _on_head_click(self, _w, event):
        if event.button == 2:
            self.win.close_tab(self)
            return True
        return False

    def _on_mouse(self, _view, event):
        if event.button == 8 and self.view.can_go_back():
            self.view.go_back()
            return True
        if event.button == 9 and self.view.can_go_forward():
            self.view.go_forward()
            return True
        return False

    def _on_title(self, view, _p):
        t = view.get_title() or ""
        self.title.set_text(t or ("New tab" if self.newtab else site_of(view.get_uri() or "") or "Page"))
        self.head.set_tooltip_text(t)
        if self.win.current() is self:
            self.win._update_title()
        if self.win.history is not None and t and not self.newtab:
            self.win.history.set_title(view.get_uri(), t)

    def _on_loading(self, view, _p):
        loading = view.is_loading()
        self.spinner.set_visible(loading)
        (self.spinner.start if loading else self.spinner.stop)()
        self.icon.set_visible(not loading)
        if self.win.current() is self:
            self.win.reload_btn.set_label("✕" if loading else "⟳")

    def _on_audio(self, view, _p):
        self.audio.set_visible(view.is_playing_audio() or view.get_is_muted())
        self.audio.set_label("🔇" if view.get_is_muted() else "🔊")

    def toggle_mute(self):
        self.view.set_is_muted(not self.view.get_is_muted())
        self.audio.set_label("🔇" if self.view.get_is_muted() else "🔊")
        self.audio.set_tooltip_text("Unmute this tab" if self.view.get_is_muted() else "Mute this tab")

    def _on_favicon(self, view, _p):
        surface = view.get_favicon()
        if surface is None:
            self.icon.set_from_icon_name("text-html-symbolic", Gtk.IconSize.MENU)
            return
        try:
            pb = Gdk.pixbuf_get_from_surface(surface, 0, 0, surface.get_width(), surface.get_height())
            if pb:
                self.icon.set_from_pixbuf(pb.scale_simple(16, 16, 2))
        except (TypeError, AttributeError):
            pass

    # HTTPS-only: a site with no secure version gets a choice, not a silent fallback
    def _on_load_failed(self, view, _event, uri, error):
        http = self.upgraded.pop(uri, None)
        if not http or error.matches(WebKit2.network_error_quark(), WebKit2.NetworkError.CANCELLED):
            return False
        site = host_of(http)
        self.show_info(f"<b>{GLib.markup_escape_text(site)}</b> has no secure (https) version, or it didn't answer. "
                       "On an insecure page anyone on your network can see and change what you send and receive.",
                       [("Go to the insecure page", lambda: self._continue_http(http, site))])
        html = (f"<html><body style='font-family:sans-serif;background:#111521;color:#e6e9f2;display:flex;align-items:center;"
                f"justify-content:center;height:90vh'><div style='max-width:560px'><h2>No secure connection</h2>"
                f"<p>{GLib.markup_escape_text(site)} didn't answer over https. HTTPS-only mode is on in Settings.</p>"
                f"</div></body></html>")
        view.load_alternate_html(html, uri, None)
        return True

    def _continue_http(self, http, site):
        _http_ok.add(site)
        self.infobar.hide()
        self.view.load_uri(http)

    def show_info(self, markup, buttons=(), kind=Gtk.MessageType.INFO):
        for child in self.infobar.get_action_area().get_children():
            child.destroy()
        self.infobar.set_message_type(kind)
        self.info_label.set_markup(markup)
        for label, fn in buttons:
            b = Gtk.Button(label=label)
            b.connect("clicked", lambda _b, f=fn: (self.infobar.hide(), f()))
            self.infobar.get_action_area().pack_start(b, False, False, 0)
        # (no-show-all keeps it hidden when the window opens; it also makes
        # show_all() skip it, so show it and its contents explicitly)
        self.infobar.get_content_area().show_all()
        self.infobar.get_action_area().show_all()
        self.infobar.show()

    # site permissions: asked once, remembered per site (never in private windows)
    def _on_permission(self, view, request):
        uri = view.get_uri() or ""
        site = site_of(uri)
        if isinstance(request, WebKit2.MediaKeySystemPermissionRequest):
            request.deny()
            self.show_info("This site wants to play <b>copy-protected (DRM)</b> video, as Netflix, Spotify, Disney+ "
                           "and Prime Video do. WebKit has no DRM module on Linux, so those can't play in this browser.",
                           kind=Gtk.MessageType.WARNING)
            return True
        kinds = []
        if isinstance(request, WebKit2.GeolocationPermissionRequest):
            kinds = ["location"]
        elif isinstance(request, WebKit2.NotificationPermissionRequest):
            kinds = ["notifications"]
        elif isinstance(request, WebKit2.UserMediaPermissionRequest):
            if WebKit2.user_media_permission_is_for_video_device(request):
                kinds.append("camera")
            if WebKit2.user_media_permission_is_for_audio_device(request):
                kinds.append("microphone")
        elif isinstance(request, WebKit2.DeviceInfoPermissionRequest):
            # Device names for a site you already let use your camera or microphone.
            ok = PREFS.permission(uri, "camera") or PREFS.permission(uri, "microphone")
            (request.allow if ok and not self.win.private else request.deny)()
            return True
        if not kinds or (self.win.use_tor and kinds != ["notifications"]):
            request.deny()
            return True
        if not self.win.private:
            remembered = [PREFS.permission(uri, k) for k in kinds]
            if all(r is True for r in remembered):
                request.allow()
                return True
            if any(r is False for r in remembered):
                request.deny()
                return True
        what = " and ".join(PERMISSIONS[k] for k in kinds)

        def answer(allowed):
            (request.allow if allowed else request.deny)()
            if not self.win.private:
                for k in kinds:
                    PREFS.set_permission(uri, k, allowed)
        self.show_info(f"<b>{GLib.markup_escape_text(site)}</b> wants to {what}." +
                       ("" if self.win.private else " Your answer is remembered for this site (Settings can change it)."),
                       [("Allow", lambda: answer(True)), ("Block", lambda: answer(False))], kind=Gtk.MessageType.QUESTION)
        return True


# ---- the window ----------------------------------------------------------------

class BrowserWindow(Gtk.Window):
    def __init__(self, start_url=None, private=False, use_tor=False, restore=False):
        private = private or use_tor
        title = "Nexa Browser — Tor" if use_tor else "Nexa Browser — Private" if private else "Nexa Browser"
        super().__init__(title=title)
        self.set_icon_name("nexa-browser")
        attach_titlebar(self, title)
        self.set_default_size(1100, 760)

        self.private, self.use_tor = private, use_tor
        self.history = None if private else history()
        self.context = _make_context(private, use_tor)
        self.settings = _make_settings(private, use_tor)
        self.ucm = WebKit2.UserContentManager()
        _ucms.append(self.ucm)
        _apply_to(self.ucm)
        self.tabs = []
        self.closed = []  # reopen with Ctrl+Shift+T
        self.downloads = []  # dicts: download, name, path, meter, widgets, state
        self._session_pending = 0
        self._fullscreen = False

        self._build_ui()
        self.connect("destroy", self._on_destroy)
        if use_tor:
            tab = self.new_tab(None)
            self._connect_tor(tab, start_url or TOR_HOME_URL)
        else:
            urls, active = (session.load() if restore else ([], 0))
            if start_url:
                self.new_tab(start_url)
            elif urls:
                for u in urls:
                    self.new_tab(u, background=True)
                self.notebook.set_current_page(active)
            else:
                self.new_tab(PREFS["home"] or None)
        GLib.timeout_add_seconds(1, self._tick_downloads)

    # ---- layout
    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        self.toolbar = toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        root.pack_start(toolbar, False, False, 0)

        def btn(label, tip, fn):
            b = Gtk.Button(label=label)
            b.set_name("toolbtn")
            b.set_tooltip_text(tip)
            b.connect("clicked", lambda *_: fn())
            return b
        self.back_btn = btn("←", "Back (Alt+Left)", lambda: self.view().go_back())
        self.fwd_btn = btn("→", "Forward (Alt+Right)", lambda: self.view().go_forward())
        self.reload_btn = btn("⟳", "Reload (Ctrl+R)", self._reload_stop)
        home_btn = btn("⌂", "Home", lambda: self.go_home())
        for b in (self.back_btn, self.fwd_btn, self.reload_btn, home_btn):
            toolbar.pack_start(b, False, False, 0)

        if self.private:
            badge = Gtk.Label(label="Tor" if self.use_tor else "Private")
            badge.set_name("private-badge")
            badge.set_tooltip_text(TOR_NOTE if self.use_tor else
                                   "Nothing from this window is saved: no history, cookies or cache")
            toolbar.pack_start(badge, False, False, 0)

        self.url_entry = Gtk.Entry()
        self.url_entry.set_name("urlentry")
        self.url_entry.set_hexpand(True)
        self.url_entry.set_placeholder_text("Search or type an address")
        self.url_entry.connect("activate", self._on_url_activate)
        toolbar.pack_start(self.url_entry, True, True, 0)
        if self.history is not None:
            self._attach_completion()

        self.star_btn = btn("☆", "Bookmark this page (Ctrl+D)", self.toggle_bookmark)
        toolbar.pack_start(self.star_btn, False, False, 0)
        self.shield_btn = Gtk.Button(label="🛡")
        self.shield_btn.connect("clicked", lambda *_: self.toggle_site_blocking())
        toolbar.pack_start(self.shield_btn, False, False, 0)

        self.downloads_btn = Gtk.MenuButton()
        self.downloads_btn.set_name("toolbtn")
        self.downloads_btn.set_tooltip_text("Downloads")
        self.downloads_label = Gtk.Label(label="⬇")
        self.downloads_btn.add(self.downloads_label)
        self.downloads_btn.set_popover(self._downloads_popover())
        toolbar.pack_end(self.downloads_btn, False, False, 0)

        self.menu_btn = Gtk.MenuButton()
        self.menu_btn.set_name("toolbtn")
        self.menu_btn.add(Gtk.Label(label="☰"))
        self.menu_btn.set_tooltip_text("Menu")
        self.menu_btn.set_popover(self._menu_popover())
        self.menu_btn.connect("toggled", lambda b: b.get_active() and self._update_zoom_label())
        toolbar.pack_end(self.menu_btn, False, False, 0)
        if self.history is not None:
            self.history_popover = self._history_popover()
            self.history_popover.set_relative_to(self.menu_btn)
        self.bookmarks_popover = self._bookmarks_popover()
        self.bookmarks_popover.set_relative_to(self.star_btn)

        self.notebook = Gtk.Notebook()
        self.notebook.set_scrollable(True)
        self.notebook.set_show_border(False)
        newtab_btn = btn("+", "New tab (Ctrl+T)", lambda: self.new_tab(None))
        newtab_btn.show()
        self.notebook.set_action_widget(newtab_btn, Gtk.PackType.END)
        self.notebook.connect("switch-page", self._on_switch_page)
        self.notebook.connect("page-reordered", lambda *_: self._session_soon())
        root.pack_start(self.notebook, True, True, 0)

        root.pack_start(self._findbar(), False, False, 0)
        self.connect("key-press-event", self._on_key)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider,
                                                 Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    def _menu_popover(self):
        pop = Gtk.Popover()
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        box.set_border_width(6)

        def item(label, accel, fn):
            b = Gtk.Button()
            b.set_name("menuitem")
            row = Gtk.Box(spacing=24)
            row.pack_start(Gtk.Label(label=label, xalign=0), True, True, 0)
            if accel:
                a = Gtk.Label(label=accel)
                a.set_name("panel-muted")
                row.pack_end(a, False, False, 0)
            b.add(row)
            b.connect("clicked", lambda *_: (pop.popdown(), fn()))
            box.pack_start(b, False, False, 0)
        item("New tab", "Ctrl+T", lambda: self.new_tab(None))
        item("New private window", "Ctrl+Shift+N", _open_private_window)
        item("New Tor window", "", lambda: _track(BrowserWindow(use_tor=True)))
        box.pack_start(Gtk.Separator(), False, False, 4)
        item("Bookmarks", "Ctrl+Shift+O", self.show_bookmarks)
        if self.history is not None:
            item("History", "Ctrl+H", self.show_history)
        item("Downloads", "", lambda: self.downloads_btn.set_active(True))
        item("Reopen closed tab", "Ctrl+Shift+T", self.reopen_closed)
        box.pack_start(Gtk.Separator(), False, False, 4)
        item("Find in page", "Ctrl+F", self.show_find)
        zoom = Gtk.Box(spacing=6)
        zoom.pack_start(Gtk.Label(label="Zoom", xalign=0), True, True, 6)
        plus = Gtk.Button(label="+")
        minus = Gtk.Button(label="−")
        self.zoom_label = Gtk.Button(label="100%")
        self.zoom_label.set_tooltip_text("Back to 100% (Ctrl+0)")
        for b, d in ((minus, -0.1), (self.zoom_label, None), (plus, 0.1)):
            b.set_name("toolbtn")
            b.connect("clicked", lambda _b, d=d: self.zoom(d))
            zoom.pack_start(b, False, False, 0)
        box.pack_start(zoom, False, False, 2)
        item("Full screen", "F11", self.toggle_fullscreen)
        item("Print…", "Ctrl+P", self.print_page)
        item("View page source", "Ctrl+U", self.view_source)
        item("Developer tools", "F12", lambda: self.view().get_inspector().show())
        box.pack_start(Gtk.Separator(), False, False, 4)
        item("Settings", "", self.show_settings)
        box.show_all()
        pop.add(box)
        return pop

    def _findbar(self):
        self.find_revealer = Gtk.Revealer()
        bar = Gtk.Box(spacing=6)
        bar.set_name("findbar")
        self.find_entry = Gtk.SearchEntry(placeholder_text="Find in page")
        self.find_entry.connect("search-changed", lambda *_: self._find(0))
        self.find_entry.connect("activate", lambda *_: self._find(1))
        self.find_entry.connect("next-match", lambda *_: self._find(1))
        self.find_entry.connect("previous-match", lambda *_: self._find(-1))
        self.find_entry.connect("stop-search", lambda *_: self.hide_find())
        bar.pack_start(self.find_entry, False, False, 0)
        for label, d in (("↑", -1), ("↓", 1)):
            b = Gtk.Button(label=label)
            b.set_name("toolbtn")
            b.connect("clicked", lambda _b, d=d: self._find(d))
            bar.pack_start(b, False, False, 0)
        self.find_count = Gtk.Label(label="")
        self.find_count.set_name("findcount")
        bar.pack_start(self.find_count, False, False, 0)
        close = Gtk.Button(label="✕")
        close.set_name("toolbtn")
        close.connect("clicked", lambda *_: self.hide_find())
        bar.pack_end(close, False, False, 0)
        self.find_revealer.add(bar)
        return self.find_revealer

    # ---- tabs
    def new_view(self, related=None):
        if related is not None:
            view = WebKit2.WebView(related_view=related)
            view.set_settings(self.settings)
        else:
            view = WebKit2.WebView(web_context=self.context, user_content_manager=self.ucm, settings=self.settings)
        return view

    def add_tab(self, view, background=False, after_current=False):
        tab = Tab(self, view)
        self.tabs.append(tab)
        pos = self.notebook.get_current_page() + 1 if after_current and self.tabs else -1
        n = self.notebook.insert_page(tab.page, tab.head, pos)
        self.notebook.set_tab_reorderable(tab.page, True)
        tab.page.show_all()
        tab.infobar.hide()
        if not background:
            self.notebook.set_current_page(n)
        self._session_soon()
        return tab

    def new_tab(self, uri, background=False, after_current=False):
        tab = self.add_tab(self.new_view(), background, after_current)
        if uri:
            tab.view.load_uri(uri)
        elif not self.use_tor:
            self.show_newtab(tab)
        return tab

    def show_newtab(self, tab):
        tab.newtab = True
        top = self.history.top(8) if self.history is not None else []
        colors = None
        try:
            from nexa_common import themes
            pal = themes.app_palette(themes.THEMES[themes.load_settings()["theme"]])
            colors = {k: pal[v] for k, v in (("bg", "bg"), ("surface", "surface"), ("text", "text"), ("muted", "muted"),
                                              ("accent", "accent")) if str(pal.get(v, "")).startswith("#")}
        except Exception:
            pass
        tab.view.load_html(newtab.page(PREFS["search"], top, BOOKMARKS.items[:8], colors), NEWTAB)
        tab.title.set_text("New tab")
        if tab is self.current():
            self.url_entry.set_text("")
            self.url_entry.grab_focus()

    def current(self):
        page = self.notebook.get_nth_page(self.notebook.get_current_page())
        return next((t for t in self.tabs if t.page is page), None)

    def view(self):
        tab = self.current()
        return tab.view if tab else None

    def tab_of(self, view):
        return next((t for t in self.tabs if t.view is view), None)

    def close_tab(self, tab):
        if tab not in self.tabs:
            return
        uri = tab.view.get_uri()
        if uri and not tab.newtab:
            self.closed.append(uri)
            del self.closed[:-20]
        self.tabs.remove(tab)
        self.notebook.remove_page(self.notebook.page_num(tab.page))
        tab.page.destroy()
        if not self.tabs:
            self.destroy()
            return
        self._session_soon()

    def reopen_closed(self):
        if self.closed:
            self.new_tab(self.closed.pop())

    def _on_switch_page(self, _nb, page, _n):
        tab = next((t for t in self.tabs if t.page is page), None)
        if tab is None:
            return
        uri = tab.view.get_uri() or ""
        self.url_entry.set_text("" if tab.newtab else uri)
        GLib.idle_add(self._sync_buttons)
        self._update_title(tab)
        if self.find_revealer.get_reveal_child():
            self._find(0)

    def _update_title(self, tab=None):
        tab = tab or self.current()
        t = tab.view.get_title() if tab and not tab.newtab else ""
        mark = " — Tor" if self.use_tor else " — Private" if self.private else ""
        self.set_title((t or "Nexa Browser") + mark)

    def _sync_buttons(self):
        view = self.view()
        if view is None:
            return False
        tab = self.current()
        self.back_btn.set_sensitive(view.can_go_back())
        self.fwd_btn.set_sensitive(view.can_go_forward())
        self.reload_btn.set_label("✕" if view.is_loading() else "⟳")
        uri = view.get_uri() or ""
        real = uri and not tab.newtab and not uri.startswith(("about:", "data:"))
        self.star_btn.set_label("★" if real and BOOKMARKS.has(uri) else "☆")
        self.star_btn.set_sensitive(bool(real))
        on = PREFS.blocking_on(uri) if real else PREFS["block_ads"]
        self.shield_btn.set_name("shield-on" if on else "shield-off")
        self.shield_btn.set_tooltip_text(
            ("Blocking ads and trackers here. Click to allow them on " + site_of(uri)) if on and real else
            ("Ads and trackers are allowed on " + site_of(uri) + ". Click to block them again") if real else
            "Ad and tracker blocking")
        self.shield_btn.set_sensitive(bool(real) and PREFS["block_ads"])
        return False

    # ---- view signals (any tab; act on the visible one)
    def _on_uri_changed(self, view, _p):
        tab = self.tab_of(view)
        if tab and view.get_uri() not in (None, NEWTAB):
            tab.newtab = False
        if tab is self.current() and not self.url_entry.has_focus():
            self.url_entry.set_text("" if tab.newtab else (view.get_uri() or ""))
        self._sync_buttons()

    def _on_progress(self, view, _p):
        if self.tab_of(view) is self.current():
            self.url_entry.set_progress_fraction(view.get_estimated_load_progress() if view.is_loading() else 0)

    def _on_load_changed(self, view, event):
        tab = self.tab_of(view)
        if event == WebKit2.LoadEvent.COMMITTED and tab and not tab.newtab:
            uri = view.get_uri()
            view.set_zoom_level(PREFS.zoom_for(uri))
            if self.history is not None:
                self.history.add(uri, view.get_title() or "")
            self._session_soon()
        if tab is self.current():
            self._sync_buttons()
        if event == WebKit2.LoadEvent.FINISHED:
            self.url_entry.set_progress_fraction(0)

    def _on_decide_policy(self, view, decision, decision_type):
        if decision_type == WebKit2.PolicyDecisionType.NAVIGATION_ACTION:
            nav = decision.get_navigation_action()
            uri = nav.get_request().get_uri() or ""
            # Middle-click or Ctrl+click on a link: a new tab in the background.
            if nav.get_navigation_type() == WebKit2.NavigationType.LINK_CLICKED and (
                    nav.get_mouse_button() == 2 or nav.get_modifiers() & Gdk.ModifierType.CONTROL_MASK):
                decision.ignore()
                self.new_tab(uri, background=True, after_current=True)
                return True
            # HTTPS-only: ask for the secure version first.
            if uri.startswith("http://") and PREFS["https_only"] and not self.use_tor:
                host = host_of(uri)
                if host and not _is_local(host) and site_of(host) not in _http_ok:
                    https = "https://" + uri[len("http://"):]
                    tab = self.tab_of(view)
                    if tab:
                        tab.upgraded[https] = uri
                    decision.ignore()
                    view.load_uri(https)
                    return True
        # Anything the browser can't show (zip, iso, ...) or that the site
        # marks as an attachment is downloaded instead of failing to load.
        if decision_type == WebKit2.PolicyDecisionType.RESPONSE:
            response = decision.get_response()
            disposition = ""
            try:
                disposition = response.get_http_headers().get_one("Content-Disposition") or ""
            except (AttributeError, TypeError):
                pass
            if not decision.is_mime_type_supported() or disposition.lower().startswith("attachment"):
                decision.download()
                return True
        return False

    def _on_create(self, view, _nav):
        """window.open / target=_blank / "Open link in new window": a new tab."""
        new = self.new_view(related=view)

        def ready(v):
            self.add_tab(v, after_current=True)
        new.connect("ready-to-show", ready)
        return new

    def _on_fullscreen(self, _view):
        self.toolbar.hide()
        self.notebook.set_show_tabs(False)
        return False

    def _on_unfullscreen(self, _view):
        self.toolbar.show()
        self.notebook.set_show_tabs(True)
        return False

    def toggle_fullscreen(self):
        self._fullscreen = not self._fullscreen
        (self.fullscreen if self._fullscreen else self.unfullscreen)()

    # ---- actions
    def _on_url_activate(self, entry):
        self.load(_to_uri(entry.get_text(), PREFS.search_url()))

    def load(self, uri):
        view = self.view()
        if view is None:
            self.new_tab(uri)
            return
        view.load_uri(uri)
        view.grab_focus()

    def go_home(self):
        if PREFS["home"]:
            self.load(PREFS["home"])
        else:
            self.show_newtab(self.current())

    def _reload_stop(self):
        view = self.view()
        if view.is_loading():
            view.stop_loading()
        elif self.current().newtab:
            self.show_newtab(self.current())
        else:
            view.reload()

    def zoom(self, delta):
        view = self.view()
        level = 1.0 if delta is None else view.get_zoom_level() + delta
        level = max(0.3, min(3.0, level))
        view.set_zoom_level(level)
        if not self.private and view.get_uri():
            PREFS.set_zoom(view.get_uri(), level)
        self._update_zoom_label()

    def _update_zoom_label(self):
        view = self.view()
        if view is not None:
            self.zoom_label.set_label(f"{view.get_zoom_level() * 100:.0f}%")

    def print_page(self):
        WebKit2.PrintOperation.new(self.view()).run_dialog(self)

    def view_source(self):
        view = self.view()
        res = view.get_main_resource()
        uri = view.get_uri() or ""
        if res is None or self.current().newtab:
            return

        def got(r, result):
            try:
                data = r.get_data_finish(result)
            except GLib.Error:
                return
            tab = self.new_tab(None, after_current=True)
            tab.newtab = False
            tab.view.load_plain_text(bytes(data).decode("utf-8", errors="replace"))
            tab.title.set_text("Source: " + site_of(uri))
        res.get_data(None, got)

    def toggle_bookmark(self):
        view = self.view()
        uri = view.get_uri() or ""
        if uri and not self.current().newtab:
            BOOKMARKS.toggle(uri, view.get_title() or "")
            self._sync_buttons()

    def toggle_site_blocking(self):
        uri = self.view().get_uri() or ""
        if not uri or self.current().newtab:
            return
        PREFS.set_blocking(uri, not PREFS.blocking_on(uri))
        self.shield_btn.set_sensitive(False)

        def done(_ok):
            self._sync_buttons()
            self.view().reload()
        refresh_blocking(done)

    def show_find(self):
        self.find_revealer.set_reveal_child(True)
        self.find_entry.grab_focus()
        self._find(0)

    def hide_find(self):
        self.find_revealer.set_reveal_child(False)
        view = self.view()
        if view:
            view.get_find_controller().search_finish()
            view.grab_focus()

    def _find(self, step):
        view = self.view()
        text = self.find_entry.get_text()
        fc = view.get_find_controller()
        if not text:
            fc.search_finish()
            self.find_count.set_text("")
            return
        if not getattr(fc, "_nexa_hooked", False):
            fc._nexa_hooked = True
            fc.connect("counted-matches", lambda _f, n: self.find_count.set_text(
                f"{n} match{'es' if n != 1 else ''}" if n else "Not found"))
            fc.connect("failed-to-find-text", lambda _f: self.find_count.set_text("Not found"))
        opts = WebKit2.FindOptions.CASE_INSENSITIVE | WebKit2.FindOptions.WRAP_AROUND
        if step == 0 or fc.get_search_text() != text:
            fc.search(text, opts, 1000)
            fc.count_matches(text, opts, 1000)
        elif step > 0:
            fc.search_next()
        else:
            fc.search_previous()

    def show_settings(self):
        from .settings_window import SettingsWindow
        SettingsWindow(self).show_all()

    def _on_key(self, _w, event):
        ctrl = bool(event.state & Gdk.ModifierType.CONTROL_MASK)
        shift = bool(event.state & Gdk.ModifierType.SHIFT_MASK)
        alt = bool(event.state & Gdk.ModifierType.MOD1_MASK)
        k = Gdk.keyval_to_lower(event.keyval)
        view = self.view()
        actions = {
            (True, True, Gdk.KEY_n): _open_private_window,
            (True, False, Gdk.KEY_t): lambda: self.new_tab(None),
            (True, True, Gdk.KEY_t): self.reopen_closed,
            (True, False, Gdk.KEY_w): lambda: self.close_tab(self.current()),
            (True, False, Gdk.KEY_l): lambda: (self.url_entry.grab_focus(), self.url_entry.select_region(0, -1)),
            (True, False, Gdk.KEY_f): self.show_find,
            (True, False, Gdk.KEY_r): lambda: view.reload(),
            (True, True, Gdk.KEY_r): lambda: view.reload_bypass_cache(),
            (True, False, Gdk.KEY_d): self.toggle_bookmark,
            (True, False, Gdk.KEY_p): self.print_page,
            (True, False, Gdk.KEY_u): self.view_source,
            (True, True, Gdk.KEY_i): lambda: view.get_inspector().show(),
            (True, True, Gdk.KEY_o): self.show_bookmarks,
            (True, False, Gdk.KEY_h): self.show_history,
            (True, False, Gdk.KEY_plus): lambda: self.zoom(0.1),
            (True, False, Gdk.KEY_equal): lambda: self.zoom(0.1),
            (True, True, Gdk.KEY_plus): lambda: self.zoom(0.1),
            (True, False, Gdk.KEY_minus): lambda: self.zoom(-0.1),
            (True, False, Gdk.KEY_0): lambda: self.zoom(None),
            (True, False, Gdk.KEY_Page_Down): lambda: self.notebook.next_page(),
            (True, False, Gdk.KEY_Page_Up): lambda: self.notebook.prev_page(),
        }
        if (ctrl, shift, k) in actions and not alt:
            actions[(ctrl, shift, k)]()
            return True
        if ctrl and event.keyval in (Gdk.KEY_Tab, Gdk.KEY_ISO_Left_Tab):
            n, cur = self.notebook.get_n_pages(), self.notebook.get_current_page()
            self.notebook.set_current_page((cur + (-1 if shift else 1)) % n)
            return True
        if ctrl and Gdk.KEY_1 <= event.keyval <= Gdk.KEY_9:
            i = event.keyval - Gdk.KEY_1
            self.notebook.set_current_page(self.notebook.get_n_pages() - 1 if i == 8 else i)
            return True
        if alt and event.keyval == Gdk.KEY_Left and view.can_go_back():
            view.go_back()
            return True
        if alt and event.keyval == Gdk.KEY_Right and view.can_go_forward():
            view.go_forward()
            return True
        if event.keyval == Gdk.KEY_F5:
            view.reload()
            return True
        if event.keyval == Gdk.KEY_F11:
            self.toggle_fullscreen()
            return True
        if event.keyval == Gdk.KEY_F12:
            view.get_inspector().show()
            return True
        if event.keyval == Gdk.KEY_Escape and self.find_revealer.get_reveal_child():
            self.hide_find()
            return True
        return False

    # ---- session (normal windows only)
    def _session_soon(self):
        if self.private or self._session_pending:
            return
        self._session_pending = GLib.timeout_add_seconds(2, self._save_session)

    def _save_session(self):
        self._session_pending = 0
        if PREFS["restore_session"]:
            urls = []
            for w in _open_windows:
                if not w.private:
                    urls += [t.view.get_uri() for t in w.tabs if not t.newtab]
            session.save(urls, self.notebook.get_current_page())
        return False

    def _on_destroy(self, _w):
        if self.ucm in _ucms:
            _ucms.remove(self.ucm)
        if not self.private and PREFS["restore_session"]:
            others = [w for w in _open_windows if w is not self and not w.private]
            if others:
                others[0]._save_session()
            else:
                self._save_session()

    # ---- Tor
    def _connect_tor(self, tab, target):
        try:
            tor.start()
        except tor.TorError as e:
            self._tor_page(tab, "Tor could not start", GLib.markup_escape_text(str(e)))
            return
        self._tor_target = target
        self._tor_shown = None
        GLib.timeout_add(500, self._poll_tor, tab)

    def _poll_tor(self, tab):
        if tab not in self.tabs or tab.view.get_uri() not in (None, "about:blank"):
            return False  # closed, or the user navigated away while connecting
        if not tor.running():
            self._tor_page(tab, "Tor stopped", "Tor exited while connecting. Run <b>nexadark</b> in a terminal to see why.")
            return False
        pct, summary = tor.progress()
        if pct >= 100:
            tab.view.load_uri(self._tor_target)
            return False
        if (pct, summary) != self._tor_shown:
            self._tor_shown = (pct, summary)
            self._tor_page(tab, f"Connecting to Tor… {pct}%",
                           GLib.markup_escape_text(summary or "Starting") + "<br><br>" + GLib.markup_escape_text(TOR_NOTE))
        return True

    def _tor_page(self, tab, heading, body):
        html = ("<html><body style='font-family:sans-serif;background:#16121f;color:#e8e3f5;"
                "display:flex;align-items:center;justify-content:center;height:90vh;margin:0'>"
                f"<div style='max-width:560px;text-align:center'><h2>{GLib.markup_escape_text(heading)}</h2>"
                f"<p style='color:#a99fc2;line-height:1.5'>{body}</p></div></body></html>")
        tab.view.load_html(html, "about:blank")

    # ---- history
    def _attach_completion(self):
        self.completion_store = Gtk.ListStore(str, str)  # title, url
        completion = Gtk.EntryCompletion(model=self.completion_store)
        completion.set_match_func(lambda *_: True)  # rows are already filtered
        completion.set_minimum_key_length(2)
        title_cell = Gtk.CellRendererText(ellipsize=Pango.EllipsizeMode.END, width_chars=40)
        url_cell = Gtk.CellRendererText(ellipsize=Pango.EllipsizeMode.END, foreground="#8a93a6")
        completion.pack_start(title_cell, True)
        completion.add_attribute(title_cell, "text", 0)
        completion.pack_start(url_cell, True)
        completion.add_attribute(url_cell, "text", 1)
        completion.connect("match-selected", self._on_completion_selected)
        self.url_entry.set_completion(completion)
        self.url_entry.connect("changed", self._on_url_typed)

    def _on_url_typed(self, entry):
        if not entry.has_focus():
            return  # the page changed the URL, not the user
        self.completion_store.clear()
        q = entry.get_text()
        seen = set()
        for b in BOOKMARKS.search(q)[:3] if q.strip() else []:
            seen.add(b["url"])
            self.completion_store.append(["★ " + b["title"], b["url"]])
        for row in self.history.search(q, limit=8):
            if row["url"] not in seen:
                self.completion_store.append([row["title"], row["url"]])

    def _on_completion_selected(self, _completion, model, it):
        self.load(model[it][1])
        return True

    def _list_popover(self, title_text, placeholder, clear_label, on_clear, on_search):
        pop = Gtk.Popover()
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        box.set_border_width(10)
        head = Gtk.Box(spacing=6)
        title = Gtk.Label(label=title_text, xalign=0)
        title.set_name("panel-title")
        head.pack_start(title, True, True, 0)
        if clear_label:
            clear = Gtk.Button(label=clear_label)
            clear.set_name("toolbtn")
            clear.connect("clicked", lambda *_: on_clear())
            head.pack_end(clear, False, False, 0)
        box.pack_start(head, False, False, 0)
        search = Gtk.SearchEntry(placeholder_text=placeholder)
        search.connect("search-changed", lambda e: on_search(e.get_text()))
        box.pack_start(search, False, False, 0)
        scroller = Gtk.ScrolledWindow()
        scroller.set_size_request(440, 360)
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        lst = Gtk.ListBox()
        lst.connect("row-activated", lambda _l, row: self._open_row(pop, row))
        scroller.add(lst)
        box.pack_start(scroller, True, True, 0)
        box.show_all()
        pop.add(box)
        return pop, lst, search

    def _open_row(self, pop, row):
        url = getattr(row.get_child(), "url", None)
        if url:
            pop.popdown()
            self.load(url)

    def _fill_list(self, lst, rows, empty_text, removable=None):
        for child in lst.get_children():
            child.destroy()
        if not rows:
            empty = Gtk.Label(label=empty_text)
            empty.set_name("panel-muted")
            lst.add(empty)
        for row in rows:
            item = Gtk.Box(spacing=6)
            item.set_name("panel-row")
            col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            t = Gtk.Label(label=row["title"], xalign=0, ellipsize=Pango.EllipsizeMode.END)
            u = Gtk.Label(label=row["url"], xalign=0, ellipsize=Pango.EllipsizeMode.END)
            u.set_name("panel-muted")
            col.pack_start(t, False, False, 0)
            col.pack_start(u, False, False, 0)
            item.pack_start(col, True, True, 0)
            if removable:
                x = Gtk.Button(label="✕")
                x.set_name("tabclose")
                x.set_tooltip_text("Remove")
                x.connect("clicked", lambda _b, url=row["url"]: removable(url))
                item.pack_end(x, False, False, 0)
            item.url = row["url"]
            lst.add(item)
        lst.show_all()

    def _history_popover(self):
        pop, self.history_list, self.history_search = self._list_popover(
            "History", "Search history", "Clear history", self._on_clear_history, self._fill_history)
        return pop

    def show_history(self):
        if self.history is None:
            return
        self._fill_history("")
        self.history_popover.popup()
        self.history_search.grab_focus()

    def _fill_history(self, query):
        rows = self.history.search(query, limit=100) if query.strip() else self.history.recent(100)
        self._fill_list(self.history_list, rows, "Nothing here yet" if not query.strip() else "No matches")

    def _on_clear_history(self):
        self.history.clear()
        self.context.get_website_data_manager().clear(WebKit2.WebsiteDataTypes.ALL, 0, None, None)
        self._fill_history("")

    def _bookmarks_popover(self):
        pop, self.bookmarks_list, self.bookmarks_search = self._list_popover(
            "Bookmarks", "Search bookmarks", None, None, self._fill_bookmarks)
        return pop

    def show_bookmarks(self):
        self._fill_bookmarks("")
        self.bookmarks_popover.popup()
        self.bookmarks_search.grab_focus()

    def _fill_bookmarks(self, query):
        def remove(url):
            BOOKMARKS.remove(url)
            self._fill_bookmarks(self.bookmarks_search.get_text())
            self._sync_buttons()
        self._fill_list(self.bookmarks_list, BOOKMARKS.search(query),
                        "No bookmarks yet: ☆ or Ctrl+D adds one" if not query.strip() else "No matches", remove)

    # ---- downloads
    def _downloads_popover(self):
        pop = Gtk.Popover()
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        box.set_border_width(10)
        head = Gtk.Box(spacing=6)
        title = Gtk.Label(label="Downloads", xalign=0)
        title.set_name("panel-title")
        head.pack_start(title, True, True, 0)
        folder = Gtk.Button(label="Open folder")
        folder.set_name("toolbtn")
        folder.connect("clicked", lambda *_: _show_in_files(downloads.downloads_dir()))
        head.pack_end(folder, False, False, 0)
        box.pack_start(head, False, False, 0)
        self.downloads_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.downloads_empty = Gtk.Label(label="No downloads yet")
        self.downloads_empty.set_name("panel-muted")
        self.downloads_box.pack_start(self.downloads_empty, False, False, 0)
        scroller = Gtk.ScrolledWindow()
        scroller.set_size_request(400, 260)
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroller.add(self.downloads_box)
        box.pack_start(scroller, True, True, 0)
        box.show_all()
        pop.add(box)
        return pop

    def add_download(self, download):
        item = {"download": download, "name": "download", "path": None, "state": "running",
                "meter": downloads.SpeedMeter()}
        download.connect("decide-destination", self._on_decide_destination, item)
        download.connect("finished", self._on_download_finished, item)
        download.connect("failed", self._on_download_failed, item)

        row = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        top = Gtk.Box(spacing=6)
        item["label"] = Gtk.Label(label="Starting…", xalign=0, ellipsize=Pango.EllipsizeMode.MIDDLE)
        top.pack_start(item["label"], True, True, 0)
        item["button"] = Gtk.Button(label="Cancel")
        item["button"].set_name("toolbtn")
        item["button"].connect("clicked", self._on_download_button, item)
        top.pack_end(item["button"], False, False, 0)
        item["bar"] = Gtk.ProgressBar()
        item["status"] = Gtk.Label(label="", xalign=0)
        item["status"].set_name("panel-muted")
        row.pack_start(top, False, False, 0)
        row.pack_start(item["bar"], False, False, 0)
        row.pack_start(item["status"], False, False, 0)
        row.show_all()
        self.downloads_empty.hide()
        self.downloads_box.pack_start(row, False, False, 0)
        self.downloads_box.reorder_child(row, 0)
        self.downloads.append(item)
        self.downloads_btn.get_popover().popup()
        self._tick_downloads()

    def _on_decide_destination(self, download, suggested, item):
        name = downloads.safe_name(suggested)
        path = downloads.unique_path(downloads.downloads_dir(), name)
        item["name"], item["path"] = os.path.basename(path), path
        item["label"].set_text(item["name"])
        download.set_allow_overwrite(False)
        download.set_destination(GLib.filename_to_uri(path, None))
        return True

    def _on_download_finished(self, _download, item):
        if item["state"] == "running":
            item["state"] = "done"
        self._tick_downloads()

    def _on_download_failed(self, _download, error, item):
        cancelled = error.code == int(WebKit2.DownloadError.CANCELLED_BY_USER)
        item["state"] = "cancelled" if cancelled else "failed"
        item["error"] = error.message

    def _on_download_button(self, _btn, item):
        if item["state"] == "running":
            item["download"].cancel()
        elif item["state"] == "done" and item["path"]:
            _show_in_files(os.path.dirname(item["path"]))

    def _tick_downloads(self):
        active = 0
        for item in self.downloads:
            d = item["download"]
            received = d.get_received_data_length()
            response = d.get_response()
            total = response.get_content_length() if response else 0
            if item["state"] == "running":
                active += 1
                item["meter"].update(received)
                if total:
                    item["bar"].set_fraction(min(received / total, 1.0))
                else:
                    item["bar"].pulse()
                item["status"].set_text(downloads.status_line(received, total, item["meter"].rate()))
            elif item["state"] == "done":
                item["bar"].set_fraction(1.0)
                item["status"].set_text(f"Done · {downloads.human_bytes(received)} · saved to Downloads")
                item["button"].set_label("Show")
            else:
                item["bar"].set_fraction(0.0)
                item["status"].set_text("Cancelled" if item["state"] == "cancelled"
                                        else f"Failed: {item.get('error', '')}")
                item["button"].hide()
        self.downloads_label.set_text(f"⬇ {active}" if active else "⬇")
        return True


def _on_download_started(_context, download):
    """One handler per context (windows share the normal one): the window
    whose tab started it shows it."""
    view = download.get_web_view()
    win = view.get_toplevel() if view is not None else None
    if not isinstance(win, BrowserWindow):
        win = next((w for w in reversed(_open_windows) if w.context is _context), None)
    if win is not None:
        win.add_download(download)


def _show_in_files(directory):
    try:
        Gio.AppInfo.launch_default_for_uri(GLib.filename_to_uri(directory, None), None)
    except GLib.Error:
        pass


_open_windows = []


def _track(win):
    """Quit when the last window (normal or private) closes."""
    _open_windows.append(win)

    def closed(w):
        _open_windows.remove(w)
        if not _open_windows:
            Gtk.main_quit()
    win.connect("destroy", closed)
    win.show_all()
    for t in win.tabs:
        t.infobar.hide()
    win.find_revealer.set_reveal_child(False)
    return win


def _open_private_window():
    _track(BrowserWindow(private=True))


def _refresh_lists_in_background():
    """Added filter lists are fetched again after a week."""
    stale = blocker.stale_lists(PREFS)
    if not stale:
        return

    def work():
        for url in stale:
            try:
                blocker.download_list(url)
            except blocker.BlockerError as e:
                print(f"nexa-browser: {url}: {e}", file=sys.stderr)
        GLib.idle_add(lambda: (refresh_blocking(), False)[1])
    threading.Thread(target=work, daemon=True).start()


def main():
    Gtk.init()
    # `nexa-browser <url>` opens that page (links from other apps).
    # `nexa-browser --private` opens a window that saves nothing; `--tor`
    # (what `nexadark` runs) one that also browses through Tor.
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    private = "--private" in sys.argv[1:]
    use_tor = "--tor" in sys.argv[1:]
    refresh_blocking()
    _refresh_lists_in_background()
    restore = not args and not private and PREFS["restore_session"]
    _track(BrowserWindow(_to_uri(args[0], PREFS.search_url()) if args else None, private=private, use_tor=use_tor,
                         restore=restore))
    Gtk.main()
