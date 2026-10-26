"""Nexa Browser's Settings: search engine, home, HTTPS-only, blocking (and
the sites where it's off, and extra lists), site permissions, clearing
browsing data. Every control changes prefs.json at once."""
import threading

from gi.repository import GLib, Gtk, Pango, WebKit2

from nexa_common.titlebar import attach_titlebar

from . import blocker
from .prefs import PERMISSIONS, SEARCH_ENGINES, SUGGESTED_LISTS


class SettingsWindow(Gtk.Window):
    def __init__(self, browser):
        super().__init__(title="Browser settings", transient_for=browser)
        attach_titlebar(self, "Browser settings")
        self.set_default_size(620, 720)
        from .window import PREFS, refresh_blocking
        self.prefs, self.refresh_blocking, self.browser = PREFS, refresh_blocking, browser
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.add(scroll)
        self.box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.box.set_border_width(18)
        scroll.add(self.box)
        p = self.prefs

        self._head("Searching and starting")
        engine = Gtk.ComboBoxText()
        for key, (name, _url) in SEARCH_ENGINES.items():
            engine.append(key, name)
        engine.set_active_id(p["search"])
        engine.connect("changed", lambda c: p.set("search", c.get_active_id()))
        self._row("Search with", engine)
        home = Gtk.Entry(text=p["home"], placeholder_text="The new-tab page")
        home.connect("changed", lambda e: p.set("home", e.get_text().strip()))
        self._row("Home page", home)
        self._switch("Open the tabs from last time", "restore_session")

        self._head("Security and privacy")
        self._switch("HTTPS-only: always ask sites for a secure connection (asks before going to one that has none)",
                     "https_only")
        self._switch("Check spelling while typing (from the next start)", "spellcheck")
        clear = Gtk.Button(label="Clear browsing data…")
        clear.set_halign(Gtk.Align.START)
        clear.connect("clicked", lambda *_: self._clear_data())
        self.box.pack_start(clear, False, False, 0)

        self._head("Ads and trackers")
        sw = self._switch("Block ads and trackers (WebKit's content blocker, compiled by nexa-webfilter)", "block_ads",
                          lambda _on: self._apply_blocking())
        self.block_status = self._note("")
        self._note("Sites where blocking is off (the 🛡 button turns it off for a site):")
        self.off_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.box.pack_start(self.off_box, False, False, 0)
        self._note("Extra filter lists (fetched over https, refreshed weekly):")
        self.lists_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.box.pack_start(self.lists_box, False, False, 0)
        add_row = Gtk.Box(spacing=6)
        self.list_url = Gtk.Entry(placeholder_text="https://… a filter list")
        add_row.pack_start(self.list_url, True, True, 0)
        add = Gtk.Button(label="Add")
        add.connect("clicked", lambda *_: self._add_list(self.list_url.get_text().strip()))
        add_row.pack_start(add, False, False, 0)
        self.box.pack_start(add_row, False, False, 0)
        sugg = Gtk.Box(spacing=6)
        for name, url in SUGGESTED_LISTS:
            b = Gtk.Button(label=f"Add {name}")
            b.connect("clicked", lambda _b, n=name, u=url: self._add_list(u, n))
            sugg.pack_start(b, False, False, 0)
        self.box.pack_start(sugg, False, False, 0)
        self._fill_blocking()
        sw.set_sensitive(True)

        self._head("Site permissions")
        self._note("Camera, microphone, location and notifications: sites ask first; your answers are kept here.")
        self.perm_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.box.pack_start(self.perm_box, False, False, 0)
        self._fill_permissions()

        self._head("Video")
        self._note("Streamed video (YouTube and most sites), HLS live streams and AV1 play here. Copy-protected (DRM) "
                   "video, as on Netflix, Spotify, Disney+ and Prime Video, can't: WebKit has no DRM module on Linux.")

    # ---- small builders
    def _head(self, text):
        lab = Gtk.Label(xalign=0)
        lab.set_markup(f"<b>{GLib.markup_escape_text(text)}</b>")
        lab.set_margin_top(10)
        self.box.pack_start(lab, False, False, 0)

    def _note(self, text):
        lab = Gtk.Label(label=text, xalign=0, wrap=True)
        lab.get_style_context().add_class("dim-label")
        self.box.pack_start(lab, False, False, 0)
        return lab

    def _row(self, text, widget):
        row = Gtk.Box(spacing=12)
        row.pack_start(Gtk.Label(label=text, xalign=0), False, False, 0)
        row.pack_end(widget, True, True, 0)
        self.box.pack_start(row, False, False, 0)

    def _switch(self, text, key, after=None):
        row = Gtk.Box(spacing=12)
        lab = Gtk.Label(label=text, xalign=0, wrap=True)
        row.pack_start(lab, True, True, 0)
        sw = Gtk.Switch(active=bool(self.prefs[key]))
        sw.set_valign(Gtk.Align.CENTER)

        def changed(s, _p):
            self.prefs.set(key, s.get_active())
            if after:
                after(s.get_active())
        sw.connect("notify::active", changed)
        row.pack_end(sw, False, False, 0)
        self.box.pack_start(row, False, False, 0)
        return sw

    def _line(self, parent, text, button, fn):
        row = Gtk.Box(spacing=8)
        lab = Gtk.Label(label=text, xalign=0, ellipsize=Pango.EllipsizeMode.END)
        row.pack_start(lab, True, True, 0)
        b = Gtk.Button(label=button)
        b.connect("clicked", lambda *_: fn())
        row.pack_end(b, False, False, 0)
        parent.pack_start(row, False, False, 0)

    # ---- blocking
    def _fill_blocking(self):
        for box in (self.off_box, self.lists_box):
            for c in box.get_children():
                c.destroy()
        for site in self.prefs["blocking_off"]:
            self._line(self.off_box, site, "Block here again", lambda s=site: self._block_again(s))
        if not self.prefs["blocking_off"]:
            self._line_note(self.off_box, "None")
        for item in self.prefs["filter_lists"]:
            self._line(self.lists_box, f"{item.get('name') or item['url']}", "Remove", lambda u=item["url"]: self._remove_list(u))
        if not self.prefs["filter_lists"]:
            self._line_note(self.lists_box, "None: only the built-in list")
        self.off_box.show_all()
        self.lists_box.show_all()

    def _line_note(self, parent, text):
        lab = Gtk.Label(label=text, xalign=0)
        lab.get_style_context().add_class("dim-label")
        parent.pack_start(lab, False, False, 0)

    def _apply_blocking(self):
        self.block_status.set_text("Updating the blocker…")

        def done(ok):
            self.block_status.set_text("Blocking is up to date." if ok else "The blocker couldn't be updated (see the log).")
            self.browser._sync_buttons()
        self.refresh_blocking(done)

    def _block_again(self, site):
        self.prefs.set("blocking_off", [s for s in self.prefs["blocking_off"] if s != site])
        self._fill_blocking()
        self._apply_blocking()

    def _add_list(self, url, name=None):
        if not url:
            return
        if any(i["url"] == url for i in self.prefs["filter_lists"]):
            self.block_status.set_text("That list is already added.")
            return
        self.block_status.set_text("Downloading the list…")

        def work():
            try:
                n = blocker.download_list(url)
                GLib.idle_add(added, n, None)
            except blocker.BlockerError as e:
                GLib.idle_add(added, 0, str(e))

        def added(n, error):
            if error:
                self.block_status.set_text(error)
                return False
            self.prefs.set("filter_lists", self.prefs["filter_lists"] + [{"name": name or url, "url": url}])
            self.list_url.set_text("")
            self._fill_blocking()
            self._apply_blocking()
            return False
        threading.Thread(target=work, daemon=True).start()

    def _remove_list(self, url):
        self.prefs.set("filter_lists", [i for i in self.prefs["filter_lists"] if i["url"] != url])
        self._fill_blocking()
        self._apply_blocking()

    # ---- permissions
    def _fill_permissions(self):
        for c in self.perm_box.get_children():
            c.destroy()
        perms = self.prefs["permissions"]
        for site, kinds in sorted(perms.items()):
            text = site + ": " + ", ".join(f"{k} {'allowed' if v else 'blocked'}" for k, v in sorted(kinds.items())
                                           if k in PERMISSIONS)
            self._line(self.perm_box, text, "Forget", lambda s=site: (self.prefs.forget_permissions(s), self._fill_permissions()))
        if not perms:
            self._line_note(self.perm_box, "No site has asked yet.")
        self.perm_box.show_all()

    # ---- clearing data
    def _clear_data(self):
        d = Gtk.Dialog(title="Clear browsing data", transient_for=self, modal=True)
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Clear", Gtk.ResponseType.OK)
        area = d.get_content_area()
        area.set_border_width(12)
        checks = {}
        for key, label in (("history", "History"), ("cookies", "Cookies and site data (signs you out)"),
                           ("cache", "Cached pages and images")):
            c = Gtk.CheckButton(label=label, active=key != "cookies")
            area.pack_start(c, False, False, 2)
            checks[key] = c
        d.show_all()
        ok = d.run() == Gtk.ResponseType.OK
        d.destroy()
        if not ok:
            return
        types = 0
        if checks["cookies"].get_active():
            types |= (WebKit2.WebsiteDataTypes.COOKIES | WebKit2.WebsiteDataTypes.LOCAL_STORAGE
                      | WebKit2.WebsiteDataTypes.INDEXEDDB_DATABASES | WebKit2.WebsiteDataTypes.SESSION_STORAGE
                      | WebKit2.WebsiteDataTypes.SERVICE_WORKER_REGISTRATIONS)
        if checks["cache"].get_active():
            types |= (WebKit2.WebsiteDataTypes.MEMORY_CACHE | WebKit2.WebsiteDataTypes.DISK_CACHE)
        if types:
            self.browser.context.get_website_data_manager().clear(WebKit2.WebsiteDataTypes(types), 0, None, None)
        if checks["history"].get_active() and self.browser.history is not None:
            self.browser.history.clear()
