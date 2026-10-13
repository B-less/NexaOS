import os
import sys
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gtk, Gdk, Gio, GLib, Pango, WebKit2

from nexa_common.titlebar import attach_titlebar

from . import downloads, tor
from .urls import HOME_URL, to_uri as _to_uri
from .history import History

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#urlentry { background-image: none; background-color: @nexa_field; color: @nexa_text_strong; border-radius: 14px; padding: 6px 12px; border: 1px solid @nexa_border; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#toolbtn:disabled { color: @nexa_disabled; }
#private-badge { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 10px; padding: 2px 10px; font-weight: bold; }
popover { background-image: none; background-color: @nexa_surface; color: @nexa_text; border: 1px solid @nexa_border; }
popover list, popover row { background-image: none; background-color: transparent; color: @nexa_text; }
popover row:hover { background-color: @nexa_hover; }
popover progressbar progress { background-image: none; background-color: @nexa_accent; }
#panel-title { color: @nexa_text_strong; font-weight: bold; }
#panel-muted { color: @nexa_muted; font-size: 9pt; }
#panel-row { padding: 6px 4px; }
"""

TOR_HOME_URL = "https://check.torproject.org/"  # first page says whether Tor is working
TOR_NOTE = ("Traffic from this window goes through the Tor network and nothing is saved. "
            "It hides your IP address from websites and your network, but it is not Tor Browser: "
            "sites can still fingerprint the browser, and anything you sign in to knows it is you.")


def _data_dir():
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "nexa-browser")


def _make_context(private, use_tor=False):
    """A private window keeps nothing: no cookies, cache, or history on disk.
    A Tor window is private and sends everything, DNS lookups included,
    through Tor's SOCKS port. A normal one keeps cookies (so sign-ins survive
    a restart) but refuses third-party cookies, which are mostly tracking."""
    if use_tor:
        manager = WebKit2.WebsiteDataManager.new_ephemeral()
        manager.set_network_proxy_settings(WebKit2.NetworkProxyMode.CUSTOM,
                                           WebKit2.NetworkProxySettings.new(tor.PROXY_URI, None))
        return WebKit2.WebContext.new_with_website_data_manager(manager)
    if private:
        return WebKit2.WebContext.new_ephemeral()
    context = WebKit2.WebContext.get_default()
    cookies = context.get_cookie_manager()
    os.makedirs(_data_dir(), exist_ok=True)
    cookies.set_persistent_storage(os.path.join(_data_dir(), "cookies.sqlite"),
                                   WebKit2.CookiePersistentStorage.SQLITE)
    cookies.set_accept_policy(WebKit2.CookieAcceptPolicy.NO_THIRD_PARTY)
    return context


class BrowserWindow(Gtk.Window):
    def __init__(self, start_url=None, private=False, use_tor=False):
        private = private or use_tor
        title = "Nexa Browser — Tor" if use_tor else "Nexa Browser — Private" if private else "Nexa Browser"
        super().__init__(title=title)
        self.set_icon_name("nexa-browser")
        attach_titlebar(self, title)
        self.set_default_size(1000, 700)

        self.private = private
        self.use_tor = use_tor
        self.history = None if private else History()
        self.context = _make_context(private, use_tor)
        self.context.connect("download-started", self._on_download_started)
        self.downloads = []  # dicts: download, name, path, meter, widgets, state

        self._build_ui()
        if use_tor:
            self._connect_tor(start_url or TOR_HOME_URL)
        else:
            self.webview.load_uri(start_url or HOME_URL)
        GLib.timeout_add_seconds(1, self._tick_downloads)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        root.pack_start(toolbar, False, False, 0)

        self.back_btn = Gtk.Button(label="←")
        self.back_btn.set_name("toolbtn")
        self.back_btn.connect("clicked", lambda *_: self.webview.go_back())
        toolbar.pack_start(self.back_btn, False, False, 0)

        self.fwd_btn = Gtk.Button(label="→")
        self.fwd_btn.set_name("toolbtn")
        self.fwd_btn.connect("clicked", lambda *_: self.webview.go_forward())
        toolbar.pack_start(self.fwd_btn, False, False, 0)

        self.reload_btn = Gtk.Button(label="⟳")
        self.reload_btn.set_name("toolbtn")
        self.reload_btn.connect("clicked", self._on_reload_stop)
        toolbar.pack_start(self.reload_btn, False, False, 0)

        home_btn = Gtk.Button(label="⌂")
        home_btn.set_name("toolbtn")
        home_btn.connect("clicked", lambda *_: self.webview.load_uri(HOME_URL))
        toolbar.pack_start(home_btn, False, False, 0)

        if self.private:
            badge = Gtk.Label(label="Tor" if self.use_tor else "Private")
            badge.set_name("private-badge")
            badge.set_tooltip_text(TOR_NOTE if self.use_tor else
                                   "Nothing from this window is saved: no history, cookies or cache")
            toolbar.pack_start(badge, False, False, 0)

        self.url_entry = Gtk.Entry()
        self.url_entry.set_name("urlentry")
        self.url_entry.set_hexpand(True)
        self.url_entry.connect("activate", self._on_url_activate)
        toolbar.pack_start(self.url_entry, True, True, 0)
        if self.history is not None:
            self._attach_completion()

        self.downloads_btn = Gtk.MenuButton()
        self.downloads_btn.set_name("toolbtn")
        self.downloads_btn.set_tooltip_text("Downloads")
        self.downloads_label = Gtk.Label(label="⬇")
        self.downloads_btn.add(self.downloads_label)
        self.downloads_btn.set_popover(self._downloads_popover())
        toolbar.pack_end(self.downloads_btn, False, False, 0)

        if self.history is not None:
            history_btn = Gtk.MenuButton(label="History")
            history_btn.set_name("toolbtn")
            self.history_popover = self._history_popover()
            history_btn.set_popover(self.history_popover)
            history_btn.connect("toggled", lambda b: b.get_active() and self._fill_history(""))
            toolbar.pack_end(history_btn, False, False, 0)

        private_btn = Gtk.Button(label="Private window")
        private_btn.set_name("toolbtn")
        private_btn.set_tooltip_text("Open a window that saves nothing (Ctrl+Shift+N)")
        private_btn.connect("clicked", lambda *_: _open_private_window())
        toolbar.pack_end(private_btn, False, False, 0)
        tor_btn = Gtk.Button(label="Tor window")
        tor_btn.set_name("toolbtn")
        tor_btn.set_tooltip_text("Open a private window that browses through Tor (also: nexadark in a terminal)")
        tor_btn.connect("clicked", lambda *_: _track(BrowserWindow(use_tor=True)))
        toolbar.pack_end(tor_btn, False, False, 0)
        self.connect("key-press-event", self._on_key)

        self.webview = WebKit2.WebView(web_context=self.context)
        settings = self.webview.get_settings()
        for prop in ("enable-webrtc", "enable-dns-prefetching"):  # both can leak your address
            if settings.find_property(prop):
                settings.set_property(prop, False)
        self.webview.connect("notify::uri", self._on_uri_changed)
        self.webview.connect("notify::title", self._on_title_changed)
        self.webview.connect("notify::estimated-load-progress", self._on_progress)
        self.webview.connect("notify::is-loading", self._on_loading_changed)
        self.webview.connect("load-changed", self._on_load_changed)
        self.webview.connect("decide-policy", self._on_decide_policy)
        root.pack_start(self.webview, True, True, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _on_url_activate(self, entry):
        self.webview.load_uri(_to_uri(entry.get_text()))

    def _on_uri_changed(self, webview, _pspec):
        uri = webview.get_uri()
        if uri:
            self.url_entry.set_text(uri)

    def _on_title_changed(self, webview, _pspec):
        title = webview.get_title()
        self.set_title(title if title else "Nexa Browser")
        if self.history is not None and title:
            self.history.set_title(webview.get_uri(), title)

    def _on_progress(self, webview, _pspec):
        if webview.get_property("is-loading"):
            self.url_entry.set_progress_fraction(webview.get_estimated_load_progress())
        else:
            self.url_entry.set_progress_fraction(0)

    def _on_loading_changed(self, webview, _pspec):
        loading = webview.get_property("is-loading")
        self.reload_btn.set_label("✕" if loading else "⟳")

    def _on_load_changed(self, webview, event):
        self.back_btn.set_sensitive(webview.can_go_back())
        self.fwd_btn.set_sensitive(webview.can_go_forward())
        if event == WebKit2.LoadEvent.COMMITTED and self.history is not None:
            self.history.add(webview.get_uri(), webview.get_title() or "")

    def _on_reload_stop(self, _btn):
        if self.webview.get_property("is-loading"):
            self.webview.stop_loading()
        else:
            self.webview.reload()

    def _on_decide_policy(self, webview, decision, decision_type):
        # Links that ask for a new window (target=_blank, window.open, ...)
        # open in this same view instead of being silently dropped -- this
        # browser has no multi-window/tab model (yet).
        if decision_type == WebKit2.PolicyDecisionType.NEW_WINDOW_ACTION:
            uri = decision.get_navigation_action().get_request().get_uri()
            decision.ignore()
            webview.load_uri(uri)
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

    # ---- Tor -------------------------------------------------------------

    def _connect_tor(self, target):
        try:
            tor.start()
        except tor.TorError as e:
            self._tor_page("Tor could not start", GLib.markup_escape_text(str(e)))
            return
        self._tor_target = target
        self._tor_shown = None
        GLib.timeout_add(500, self._poll_tor)

    def _poll_tor(self):
        if self.webview.get_uri() not in (None, "about:blank"):
            return False  # the user navigated away while connecting
        if not tor.running():
            self._tor_page("Tor stopped", "Tor exited while connecting. Run <b>nexadark</b> in a terminal to see why.")
            return False
        pct, summary = tor.progress()
        if pct >= 100:
            self.webview.load_uri(self._tor_target)
            return False
        if (pct, summary) != self._tor_shown:
            self._tor_shown = (pct, summary)
            self._tor_page(f"Connecting to Tor… {pct}%",
                           GLib.markup_escape_text(summary or "Starting") + "<br><br>" + GLib.markup_escape_text(TOR_NOTE))
        return True

    def _tor_page(self, heading, body):
        html = ("<html><body style='font-family:sans-serif;background:#16121f;color:#e8e3f5;"
                "display:flex;align-items:center;justify-content:center;height:90vh;margin:0'>"
                f"<div style='max-width:560px;text-align:center'><h2>{GLib.markup_escape_text(heading)}</h2>"
                f"<p style='color:#a99fc2;line-height:1.5'>{body}</p></div></body></html>")
        self.webview.load_html(html, "about:blank")

    def _on_key(self, _w, event):
        ctrl_shift = Gdk.ModifierType.CONTROL_MASK | Gdk.ModifierType.SHIFT_MASK
        if (event.state & ctrl_shift) == ctrl_shift and event.keyval in (Gdk.KEY_N, Gdk.KEY_n):
            _open_private_window()
            return True
        if event.state & Gdk.ModifierType.CONTROL_MASK and event.keyval in (Gdk.KEY_l, Gdk.KEY_L):
            self.url_entry.grab_focus()
            return True
        return False

    # ---- history ---------------------------------------------------------

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
        for row in self.history.search(entry.get_text(), limit=8):
            self.completion_store.append([row["title"], row["url"]])

    def _on_completion_selected(self, _completion, model, it):
        self.webview.load_uri(model[it][1])
        self.webview.grab_focus()
        return True

    def _history_popover(self):
        pop = Gtk.Popover()
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        box.set_border_width(10)
        head = Gtk.Box(spacing=6)
        title = Gtk.Label(label="History", xalign=0)
        title.set_name("panel-title")
        head.pack_start(title, True, True, 0)
        clear = Gtk.Button(label="Clear history")
        clear.set_name("toolbtn")
        clear.connect("clicked", self._on_clear_history)
        head.pack_end(clear, False, False, 0)
        box.pack_start(head, False, False, 0)
        self.history_search = Gtk.SearchEntry(placeholder_text="Search history")
        self.history_search.connect("search-changed", lambda e: self._fill_history(e.get_text()))
        box.pack_start(self.history_search, False, False, 0)
        scroller = Gtk.ScrolledWindow()
        scroller.set_size_request(420, 360)
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.history_list = Gtk.ListBox()
        self.history_list.connect("row-activated", self._on_history_row)
        scroller.add(self.history_list)
        box.pack_start(scroller, True, True, 0)
        box.show_all()
        pop.add(box)
        return pop

    def _fill_history(self, query):
        for child in self.history_list.get_children():
            child.destroy()
        rows = self.history.search(query, limit=100) if query.strip() else self.history.recent(100)
        if not rows:
            empty = Gtk.Label(label="Nothing here yet" if not query.strip() else "No matches")
            empty.set_name("panel-muted")
            self.history_list.add(empty)
        for row in rows:
            item = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            item.set_name("panel-row")
            t = Gtk.Label(label=row["title"], xalign=0, ellipsize=Pango.EllipsizeMode.END)
            u = Gtk.Label(label=row["url"], xalign=0, ellipsize=Pango.EllipsizeMode.END)
            u.set_name("panel-muted")
            item.pack_start(t, False, False, 0)
            item.pack_start(u, False, False, 0)
            item.url = row["url"]
            self.history_list.add(item)
        self.history_list.show_all()

    def _on_history_row(self, _list, row):
        url = getattr(row.get_child(), "url", None)
        if url:
            self.history_popover.popdown()
            self.webview.load_uri(url)

    def _on_clear_history(self, _btn):
        self.history.clear()
        self.context.get_website_data_manager().clear(WebKit2.WebsiteDataTypes.ALL, 0, None, None)
        self._fill_history("")

    # ---- downloads -------------------------------------------------------

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

    def _on_download_started(self, _context, download):
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


def _open_private_window():
    _track(BrowserWindow(private=True))


def main():
    Gtk.init()
    # `nexa-browser <url>` opens that page (links from other apps).
    # `nexa-browser --private` opens a window that saves nothing; `--tor`
    # (what `nexadark` runs) one that also browses through Tor.
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    private = "--private" in sys.argv[1:]
    use_tor = "--tor" in sys.argv[1:]
    _track(BrowserWindow(_to_uri(args[0]) if args else None, private=private, use_tor=use_tor))
    Gtk.main()
