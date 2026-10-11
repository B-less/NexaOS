import gi
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gtk, Gdk, GLib, WebKit2

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#toolbar { background-image: none; background-color: #252525; }
#urlentry { background-image: none; background-color: #2a2a2a; color: #ffffff; border-radius: 14px; padding: 6px 12px; border: 1px solid #3c3c3c; }
#toolbtn { background-image: none; background-color: #2f2f2f; color: #d4d4d4; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: #3a3a3a; }
#toolbtn:disabled { color: #5a5a5a; }
"""

HOME_URL = "https://duckduckgo.com/"


def _looks_like_url(text):
    if " " in text:
        return False
    if text.startswith(("http://", "https://", "file://", "about:")):
        return True
    # bare "host.tld" or "host.tld/path" with no spaces -- treat as a URL,
    # otherwise (no dot, or multiple words) treat it as a search query.
    host = text.split("/", 1)[0]
    return "." in host and " " not in host


def _to_uri(text):
    text = text.strip()
    if not text:
        return HOME_URL
    if _looks_like_url(text):
        if "://" not in text:
            text = "https://" + text
        return text
    return "https://duckduckgo.com/?q=" + GLib.uri_escape_string(text, None, False)


class BrowserWindow(Gtk.Window):
    def __init__(self, start_url=None):
        super().__init__(title="Nexa Browser")
        self.set_icon_name("nexa-browser")
        attach_titlebar(self, "Nexa Browser")
        self.set_default_size(1000, 700)

        self._build_ui()
        self.webview.load_uri(start_url or HOME_URL)

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

        self.url_entry = Gtk.Entry()
        self.url_entry.set_name("urlentry")
        self.url_entry.set_hexpand(True)
        self.url_entry.connect("activate", self._on_url_activate)
        toolbar.pack_start(self.url_entry, True, True, 0)

        self.webview = WebKit2.WebView()
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

    def _on_progress(self, webview, _pspec):
        if webview.get_property("is-loading"):
            self.url_entry.set_progress_fraction(webview.get_estimated_load_progress())
        else:
            self.url_entry.set_progress_fraction(0)

    def _on_loading_changed(self, webview, _pspec):
        loading = webview.get_property("is-loading")
        self.reload_btn.set_label("✕" if loading else "⟳")

    def _on_load_changed(self, webview, _event):
        self.back_btn.set_sensitive(webview.can_go_back())
        self.fwd_btn.set_sensitive(webview.can_go_forward())

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
        return False


def main():
    Gtk.init()
    win = BrowserWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
