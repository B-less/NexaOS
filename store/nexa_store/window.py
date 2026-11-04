"""Nexa Store: find, install, update and remove NexaOS apps (.nexa).

A thin window over Rust: nexa-store reads the signed catalogues and
downloads packages (checking them against the catalogue), the system
broker installs and removes them (checking the package's own signature).
"""
import sys
import threading
import time

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, GdkPixbuf, Gio, GLib, Gtk, Pango

from . import engine
from nexa_common import askpass, broker, design, privacy, titlebar
from nexa_common.titlebar import attach_titlebar

R = design.RADIUS
CSS = f"""
#store-side {{ background-color: @nexa_surface; border-right: 1px solid @nexa_border; }}
#store-side row {{ padding: 8px 16px; border-radius: {R['medium']}px; margin: 2px 8px; color: @nexa_text; }}
#store-side row:selected {{ background-color: @nexa_selected; color: @nexa_selected_text; }}
#store-side row:hover:not(:selected) {{ background-color: @nexa_hover; }}
#side-head {{ color: @nexa_muted; font-size: 8.5pt; font-weight: bold; margin: 16px 24px 4px 24px; }}
#store-top {{ padding: 12px 16px; }}
#updated {{ color: @nexa_muted; font-size: 8.5pt; }}
#page-title {{ color: @nexa_text_strong; font-size: 15pt; font-weight: bold; }}
#card {{ background-image: none; background-color: @nexa_surface; border: 1px solid @nexa_border;
         border-radius: {R['large']}px; padding: 12px; box-shadow: none; }}
#card:hover {{ background-color: @nexa_surface_hi; border-color: @nexa_accent; }}
#card-name {{ color: @nexa_text_strong; font-weight: bold; }}
#card-desc {{ color: @nexa_muted; font-size: 9pt; }}
.chip {{ border-radius: {R['small']}px; padding: 1px 8px; font-size: 8.5pt; font-weight: bold; }}
.chip-installed {{ background-color: alpha(@nexa_ok, 0.16); color: @nexa_ok; }}
.chip-update {{ background-color: alpha(@nexa_accent, 0.18); color: @nexa_accent; }}
.chip-unverified {{ background-color: alpha(@nexa_warn, 0.16); color: @nexa_warn; }}
#app-name {{ color: @nexa_text_strong; font-size: 22pt; font-weight: bold; }}
#app-meta {{ color: @nexa_muted; }}
#app-desc {{ color: @nexa_text; font-size: 11pt; }}
#section {{ color: @nexa_text_strong; font-size: 12pt; font-weight: bold; }}
#perm {{ color: @nexa_text; }}
#verified {{ color: @nexa_ok; font-weight: bold; }}
#unverified {{ color: @nexa_warn; font-weight: bold; }}
#primary {{ background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; font-weight: bold;
            border: none; border-radius: {R['medium']}px; padding: 8px 24px; box-shadow: none; }}
#primary:hover {{ background-color: @nexa_accent_hover; }}
#primary:disabled {{ background-color: @nexa_surface_hi; color: @nexa_disabled; }}
#danger {{ background-image: none; background-color: transparent; color: @nexa_crit; border: 1px solid @nexa_crit;
           border-radius: {R['medium']}px; padding: 8px 16px; box-shadow: none; }}
#danger:hover {{ background-color: alpha(@nexa_crit, 0.12); }}
#plain {{ background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border: none;
          border-radius: {R['medium']}px; padding: 6px 12px; box-shadow: none; }}
#plain:hover {{ background-color: @nexa_hover; }}
#status {{ color: @nexa_muted; }}
#status.error {{ color: @nexa_crit; }}
#empty {{ color: @nexa_muted; font-size: 11pt; }}
#shot {{ background-image: none; background-color: @nexa_surface; border: 1px solid @nexa_border; border-radius: {R['medium']}px; padding: 4px; box-shadow: none; }}
#shot:hover {{ border-color: @nexa_accent; }}
#viewer {{ background-color: @nexa_bg_deep; }}
#perm-note {{ color: @nexa_muted; font-size: 8.5pt; }}
progressbar trough {{ min-height: 6px; border-radius: 3px; background-color: @nexa_hover; }}
progressbar progress {{ min-height: 6px; border-radius: 3px; background-color: @nexa_accent; }}
"""

REFRESH_AFTER = 6 * 3600   # catalogues older than this are fetched again on start
CARD_ICON = 48
SHOT_HEIGHT = 220
PAGE_ICON = 88


class HexIcon(Gtk.Overlay):
    """An app icon in a hexagon, the NexaOS mark."""

    def __init__(self, path, size):
        super().__init__()
        area = Gtk.DrawingArea()
        area.set_has_window(False)  # clicks belong to the app card around it
        area.set_size_request(size + 20, size + 20)
        area.connect("draw", self._draw)
        self.add(area)
        image = Gtk.Image()
        pixbuf = None
        if path:
            try:
                from gi.repository import GdkPixbuf
                pixbuf = GdkPixbuf.Pixbuf.new_from_file_at_size(path, size, size)
            except GLib.Error:
                pixbuf = None
        if pixbuf:
            image.set_from_pixbuf(pixbuf)
        else:
            image.set_from_icon_name("application-x-executable", Gtk.IconSize.DIALOG)
            image.set_pixel_size(size)
        self.add_overlay(image)
        self.set_overlay_pass_through(image, True)

    def _draw(self, area, cr):
        w, h = area.get_allocated_width(), area.get_allocated_height()
        accent = titlebar._colors.get("accent", (0.3, 0.64, 1.0))
        pts = design.hex_points(w / 2, h / 2, min(w, h) / 2 - 1)
        cr.move_to(*pts[0])
        for p in pts[1:]:
            cr.line_to(*p)
        cr.close_path()
        cr.set_source_rgba(*accent, 0.14)
        cr.fill_preserve()
        cr.set_source_rgba(*accent, 0.4)
        cr.set_line_width(1)
        cr.stroke()
        return False


def _label(text, name=None, wrap=False, xalign=0, lines=0):
    lab = Gtk.Label(label=text, xalign=xalign)
    if name:
        lab.set_name(name)
    if wrap:
        lab.set_line_wrap(True)
        lab.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
    if lines:
        lab.set_lines(lines)
        lab.set_ellipsize(Pango.EllipsizeMode.END)
        lab.set_line_wrap(True)
    return lab


def _chip(app):
    if app.get("state") == "update":
        text, cls = "Update", "chip-update"
    elif app.get("state") == "installed":
        text, cls = "Installed", "chip-installed"
    elif not app.get("verified", True):
        text, cls = "Not verified", "chip-unverified"
    else:
        return None
    chip = Gtk.Label(label=text)
    chip.get_style_context().add_class("chip")
    chip.get_style_context().add_class(cls)
    chip.set_halign(Gtk.Align.START)
    return chip


class StoreWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa Store")
        self.set_icon_name("nexa-store")
        attach_titlebar(self, "Nexa Store")
        self.set_default_size(980, 680)
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS.encode())
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider,
                                                 Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.apps = []          # every app in the catalogues
        self.installed = []     # every installed app (any source)
        self.fetched = None
        self.view = "discover"  # discover | updates | installed | a category label
        self.current = None     # the app whose page is open
        self.busy = False
        self._rebuilding = False
        self._build()
        self.connect("key-press-event", self._on_key)
        self.reload(refresh_if_old=True)

    # ---- layout ----------------------------------------------------------

    def _build(self):
        root = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        self.add(root)

        side = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        side.set_name("store-side")
        side.set_size_request(200, -1)
        self.side = Gtk.ListBox()
        self.side.set_name("store-side")
        self.side.set_selection_mode(Gtk.SelectionMode.SINGLE)
        self.side.connect("row-selected", self._on_side)
        side.pack_start(self.side, True, True, 8)
        root.pack_start(side, False, False, 0)

        main = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        root.pack_start(main, True, True, 0)

        top = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        top.set_name("store-top")
        self.search = Gtk.SearchEntry()
        self.search.set_placeholder_text("Search apps")
        self.search.set_tooltip_text("Search apps (Ctrl+F)")
        self.search.set_hexpand(True)
        self.search.connect("search-changed", lambda _e: self._show_list())
        top.pack_start(self.search, True, True, 0)
        self.updated = _label("", "updated")
        top.pack_start(self.updated, False, False, 0)
        self.refresh_btn = Gtk.Button(label="Refresh")
        self.refresh_btn.set_name("plain")
        self.refresh_btn.set_tooltip_text("Check the catalogues for new apps and updates (Ctrl+R)")
        self.refresh_btn.connect("clicked", lambda _b: self.reload(refresh=True))
        top.pack_start(self.refresh_btn, False, False, 0)
        main.pack_start(top, False, False, 0)

        self.pages = Gtk.Stack()
        self.pages.set_transition_type(Gtk.StackTransitionType.CROSSFADE)
        self.pages.set_transition_duration(design.DURATION_MS["page"])
        main.pack_start(self.pages, True, True, 0)

        # The list of apps.
        list_page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        list_page.set_border_width(16)
        head = Gtk.Box(spacing=12)
        self.page_title = _label("", "page-title")
        head.pack_start(self.page_title, False, False, 0)
        self.update_all_btn = Gtk.Button(label="Update All")
        self.update_all_btn.set_name("primary")
        self.update_all_btn.set_tooltip_text("Download and install every update")
        self.update_all_btn.set_no_show_all(True)
        self.update_all_btn.connect("clicked", lambda _b: self._update_all())
        head.pack_end(self.update_all_btn, False, False, 0)
        list_page.pack_start(head, False, False, 0)
        self.grid = Gtk.FlowBox()
        self.grid.set_selection_mode(Gtk.SelectionMode.NONE)
        self.grid.set_homogeneous(True)
        self.grid.set_min_children_per_line(1)
        self.grid.set_max_children_per_line(4)
        self.grid.set_column_spacing(12)
        self.grid.set_row_spacing(12)
        self.grid.set_valign(Gtk.Align.START)
        self.empty = _label("", "empty", wrap=True, xalign=0.5)
        self.empty.set_justify(Gtk.Justification.CENTER)
        self.empty.set_no_show_all(True)
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        inner = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=24)
        inner.pack_start(self.grid, False, False, 0)
        inner.pack_start(self.empty, False, False, 32)
        scroll.add(inner)
        list_page.pack_start(scroll, True, True, 0)
        self.pages.add_named(list_page, "list")

        # One app's page.
        self.app_scroll = Gtk.ScrolledWindow()
        self.app_scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.pages.add_named(self.app_scroll, "app")

        bottom = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        bottom.set_border_width(8)
        self.status = _label("", "status", wrap=True)
        bottom.pack_start(self.status, True, True, 8)
        self.progress = Gtk.ProgressBar()
        self.progress.set_valign(Gtk.Align.CENTER)
        self.progress.set_size_request(200, -1)
        self.progress.set_no_show_all(True)
        bottom.pack_end(self.progress, False, False, 8)
        main.pack_end(bottom, False, False, 0)

    def _side_rows(self):
        # Rebuilding re-selects the current view; that isn't a click, so it
        # mustn't close an open app page (it did, right after installing).
        self._rebuilding = True
        for child in self.side.get_children():
            self.side.remove(child)
        updates = sum(1 for a in self.installed if a.get("state") == "update")
        rows = [("discover", "Discover"), ("updates", f"Updates ({updates})" if updates else "Updates"),
                ("installed", "Installed")]
        cats = sorted({engine.category_of(a) for a in self.apps})
        for key, text in rows + [(None, "CATEGORIES")] + [(c, c) for c in cats]:
            row = Gtk.ListBoxRow()
            if key is None:
                row.set_selectable(False)
                row.set_activatable(False)
                row.add(_label(text, "side-head"))
            else:
                row.add(_label(text))
            row.key = key
            self.side.add(row)
            if key == self.view:
                self.side.select_row(row)
        self.side.show_all()
        self._rebuilding = False

    # ---- data ------------------------------------------------------------

    def reload(self, refresh=False, refresh_if_old=False):
        """Read the catalogues (fetching them first when asked or stale)."""
        if self.busy:
            return
        self.busy = True
        self.refresh_btn.set_sensitive(False)
        if not getattr(self, "_keep_status", None):
            self._status("Checking the catalogues…" if refresh else "Loading…")

        def work():
            error = note = None
            try:
                listing = engine.list_apps()
                old = not listing.get("fetched") or listing.get("now", 0) - listing["fetched"] > REFRESH_AFTER
                if refresh or (refresh_if_old and old):
                    GLib.idle_add(self._status, "Checking the catalogues…")
                    result = engine.refresh()
                    cats = result.get("catalogues", [])
                    bad = [c for c in cats if not c.get("ok")]
                    if bad and len(bad) == len(cats):
                        error = f"Couldn't read {bad[0]['name']}: {bad[0].get('error', '')}"
                    elif bad:
                        # Usually just no internet: the others still work.
                        note = f"Couldn't reach {', '.join(c['name'] for c in bad)} right now - showing the other catalogues."
                    listing = engine.list_apps()
                inst = engine.installed()
            except engine.StoreError as e:
                listing, inst, error = {"apps": []}, [], str(e)
            GLib.idle_add(self._loaded, listing, inst, error, note)

        threading.Thread(target=work, daemon=True).start()

    def _loaded(self, listing, inst, error, note=None):
        self.busy = False
        self.refresh_btn.set_sensitive(True)
        self.apps = listing.get("apps", [])
        self.installed = inst
        self.fetched = listing.get("fetched")
        self._side_rows()
        self._show_updated()
        if self.current:
            fresh = self._find(self.current["id"])
            if fresh:
                self.current = fresh
                self._show_app(fresh)
            else:
                self._show_list()
        else:
            self._show_list()
        kept, self._keep_status = getattr(self, "_keep_status", None), None
        self._status(error or note or kept or "", error=bool(error))
        return False

    def _find(self, app_id):
        for a in self.apps + self.installed:
            if a["id"] == app_id:
                merged = dict(a)
                for i in self.installed:
                    if i["id"] == app_id:
                        merged.update({k: i[k] for k in ("installed", "state") if k in i})
                return merged
        return None

    def _show_updated(self):
        if not self.fetched:
            self.updated.set_text("")
            return
        mins = max(0, int((time.time() - self.fetched) / 60))
        self.updated.set_text("Checked just now" if mins < 2 else
                              f"Checked {mins} minutes ago" if mins < 120 else
                              f"Checked {mins // 60} hours ago" if mins < 48 * 60 else
                              f"Checked {mins // 1440} days ago")

    # ---- the list --------------------------------------------------------

    def _on_side(self, _box, row):
        if self._rebuilding or row is None or getattr(row, "key", None) is None:
            return
        self.view = row.key
        self.current = None
        self._show_list()

    def _visible_apps(self):
        q = self.search.get_text().strip().lower()
        if q:
            # A search looks through everything, whatever page is open:
            # every word in the name, description, id, category or catalogue.
            seen, apps = set(), []
            for a in self.apps + self.installed:
                if a["id"] in seen:
                    continue
                seen.add(a["id"])
                text = " ".join(str(a.get(k) or "") for k in ("name", "description", "id", "categories", "catalogue")).lower()
                text += " " + engine.category_of(a).lower()
                if all(w in text for w in q.split()):
                    apps.append(self._find(a["id"]) or a)
            return sorted(apps, key=lambda a: a.get("name", "").lower())
        if self.view == "installed":
            apps = list(self.installed)
        elif self.view == "updates":
            apps = [a for a in self.installed if a.get("state") == "update"]
        elif self.view == "discover":
            apps = list(self.apps)
        else:
            apps = [a for a in self.apps if engine.category_of(a) == self.view]
        return sorted(apps, key=lambda a: a.get("name", "").lower())

    def _show_list(self):
        self.current = None
        titles = {"discover": "Discover", "updates": "Updates", "installed": "Installed"}
        q = self.search.get_text().strip()
        self.page_title.set_text(f"Results for “{q}”" if q else titles.get(self.view, self.view))
        n_updates = sum(1 for a in self.installed if a.get("state") == "update" and a.get("catalogue"))
        self.update_all_btn.set_label(f"Update All ({n_updates})")
        self.update_all_btn.set_visible(self.view == "updates" and not q and n_updates > 1)
        self.update_all_btn.set_sensitive(not self.busy)
        for child in self.grid.get_children():
            self.grid.remove(child)
        apps = self._visible_apps()
        for app in apps:
            self.grid.add(self._card(app))
        self.grid.show_all()
        if apps:
            self.empty.hide()
        else:
            q = self.search.get_text().strip()
            self.empty.set_text(
                f"No apps match “{q}”." if q else
                "Everything is up to date." if self.view == "updates" else
                "No apps installed from the Store yet.\nFind some in Discover." if self.view == "installed" else
                "No apps to show yet.\nPress Refresh to check the catalogues.")
            self.empty.show()
        self.pages.set_visible_child_name("list")

    def _card(self, app):
        btn = Gtk.Button()
        btn.set_name("card")
        btn.set_relief(Gtk.ReliefStyle.NONE)
        btn.set_tooltip_text(app.get("description") or app.get("name"))
        box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        box.pack_start(HexIcon(app.get("icon"), CARD_ICON), False, False, 0)
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        text.set_valign(Gtk.Align.CENTER)
        name = _label(app.get("name") or app["id"], "card-name")
        name.set_ellipsize(Pango.EllipsizeMode.END)
        name.set_max_width_chars(24)
        text.pack_start(name, False, False, 0)
        desc = _label(app.get("description") or "", "card-desc", lines=2)
        desc.set_max_width_chars(30)   # cards share a row instead of each claiming its full width
        desc.set_width_chars(30)
        text.pack_start(desc, False, False, 0)
        chip = _chip(app)
        if chip:
            text.pack_start(chip, False, False, 0)
        box.pack_start(text, True, True, 0)
        btn.add(box)
        btn.set_size_request(260, -1)
        btn.connect("clicked", lambda _b, a=app: self._open_app(a["id"]))
        return btn

    # ---- one app ---------------------------------------------------------

    def _open_app(self, app_id):
        app = self._find(app_id)
        if app:
            self.current = app
            self._show_app(app)

    def _show_app(self, app):
        child = self.app_scroll.get_child()
        if child:
            self.app_scroll.remove(child)
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
        page.set_border_width(24)

        back = Gtk.Button(label="← Back")
        back.set_name("plain")
        back.set_halign(Gtk.Align.START)
        back.set_tooltip_text("Back (Esc or Alt+Left)")
        back.connect("clicked", lambda _b: self._show_list())
        page.pack_start(back, False, False, 0)

        head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=24)
        head.pack_start(HexIcon(app.get("icon"), PAGE_ICON), False, False, 0)
        info = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        info.set_valign(Gtk.Align.CENTER)
        info.pack_start(_label(app.get("name") or app["id"], "app-name", wrap=True), False, False, 0)
        if app.get("verified"):
            who = _label("✓ Verified publisher", "verified")
            if app.get("publisher"):
                who.set_tooltip_text(f"Signed with key {app['publisher']}")
        else:
            who = _label("Not verified: its publisher's signature isn't one NexaOS trusts", "unverified", wrap=True)
        info.pack_start(who, False, False, 0)
        meta = [f"Version {app.get('version', '?')}"]
        if app.get("installed") and app.get("installed") != app.get("version"):
            meta.append(f"installed {app['installed']}")
        if app.get("size"):
            meta.append(engine.human_size(app["size"]))
        if app.get("catalogue"):
            meta.append(f"from {app['catalogue']}")
        info.pack_start(_label(" · ".join(meta), "app-meta", wrap=True), False, False, 0)

        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        state = app.get("state")
        if state == "available":
            primary = Gtk.Button(label="Install")
            primary.connect("clicked", lambda _b: self._install(app))
        elif state == "update":
            primary = Gtk.Button(label="Update")
            primary.connect("clicked", lambda _b: self._install(app))
        else:
            primary = Gtk.Button(label="Open")
            primary.connect("clicked", lambda _b: self._launch(app))
        primary.set_name("primary")
        primary.set_sensitive(not self.busy and (state != "update" or bool(app.get("catalogue"))))
        actions.pack_start(primary, False, False, 0)
        if state in ("installed", "update"):
            remove = Gtk.Button(label="Remove")
            remove.set_name("danger")
            remove.set_sensitive(not self.busy)
            remove.connect("clicked", lambda _b: self._remove(app))
            actions.pack_start(remove, False, False, 0)
        info.pack_start(actions, False, False, 8)
        head.pack_start(info, True, True, 0)
        page.pack_start(head, False, False, 0)

        if app.get("description"):
            page.pack_start(_label(app["description"], "app-desc", wrap=True), False, False, 0)

        shots = [s for s in app.get("screenshots") or [] if isinstance(s, str)]
        if shots:
            page.pack_start(self._gallery(app, shots), False, False, 0)

        installed = state in ("installed", "update")
        page.pack_start(_label("What it may use" if installed else "This app can", "section"), False, False, 8)
        perms = app.get("permissions") or []
        denied = set(((privacy.load_prefs().get(app["id"]) or {}).get("denied") or [])) if installed else set()
        grid = Gtk.Grid(row_spacing=8, column_spacing=12)
        self.perm_switches = {}
        for i, p in enumerate(perms):
            icon, words = engine.PERMISSIONS.get(p, ("dialog-question-symbolic", p))
            grid.attach(Gtk.Image.new_from_icon_name(icon, Gtk.IconSize.MENU), 0, i, 1, 1)
            label = _label(words, "perm", wrap=True)
            label.set_hexpand(True)
            grid.attach(label, 1, i, 1, 1)
            if installed:
                sw = Gtk.Switch(active=p not in denied, valign=Gtk.Align.CENTER)
                sw.set_tooltip_text(f"Let {app.get('name') or app['id']} {words[0].lower() + words[1:]}")
                sw.connect("notify::active", lambda w, _p, perm=p: self._set_permission(app, perm, w.get_active()))
                grid.attach(sw, 2, i, 1, 1)
                self.perm_switches[p] = sw
                if not privacy.PERMISSIONS.get(p, ("", False))[1]:
                    grid.attach(_label("NexaOS can't enforce this one yet", "perm-note"), 3, i, 1, 1)
        if not perms:
            grid.attach(_label("Use only its own window and settings - no files, internet or devices.", "perm", wrap=True), 0, 0, 2, 1)
        page.pack_start(grid, False, False, 0)
        note = _label("Switch any of these off: the app can only use what's on, from the next time it starts. "
                      "(Settings > Privacy has the same switches.)" if installed and perms else
                      "NexaOS enforces this: without a permission the app simply can't, and you can switch "
                      "permissions off after installing it.", "app-meta", wrap=True)
        page.pack_start(note, False, False, 0)

        self.app_scroll.add(page)
        self.app_scroll.show_all()
        self.pages.set_visible_child_name("app")

    def _gallery(self, app, shots):
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.AUTOMATIC, Gtk.PolicyType.NEVER)
        row = Gtk.Box(spacing=12)
        shown = 0
        for i, path in enumerate(shots):
            try:
                pix = GdkPixbuf.Pixbuf.new_from_file_at_scale(path, -1, SHOT_HEIGHT, True)
            except GLib.Error:
                continue  # a picture that doesn't decode is left out
            b = Gtk.Button()
            b.set_name("shot")
            b.set_tooltip_text("See it bigger")
            b.add(Gtk.Image.new_from_pixbuf(pix))
            b.connect("clicked", lambda _b, i=i: self._view_shot(app, shots, i))
            row.pack_start(b, False, False, 0)
            shown += 1
        scroll.add(row)
        scroll.set_size_request(-1, SHOT_HEIGHT + 24)
        self.shot_count = shown
        return scroll

    def _view_shot(self, app, shots, index):
        """A screenshot as big as the screen allows; Left/Right for the others, Esc closes."""
        win = Gtk.Window(title=f"{app.get('name') or app['id']} — screenshot")
        win.set_name("viewer")
        win.set_transient_for(self)
        win.set_modal(True)
        geo = self.get_screen().get_monitor_geometry(self.get_screen().get_monitor_at_window(self.get_window())) if self.get_window() else None
        maxw, maxh = (int(geo.width * 0.85), int(geo.height * 0.85)) if geo else (1200, 800)
        img = Gtk.Image()
        win.add(img)
        state = {"i": index}

        def show():
            try:
                img.set_from_pixbuf(GdkPixbuf.Pixbuf.new_from_file_at_scale(shots[state["i"]], maxw, maxh, True))
            except GLib.Error:
                img.set_from_icon_name("image-missing", Gtk.IconSize.DIALOG)
            win.set_title(f"{app.get('name') or app['id']} — {state['i'] + 1} of {len(shots)}")

        def key(_w, e):
            if e.keyval == Gdk.KEY_Escape:
                win.destroy()
            elif e.keyval in (Gdk.KEY_Right, Gdk.KEY_space):
                state["i"] = (state["i"] + 1) % len(shots)
                show()
            elif e.keyval == Gdk.KEY_Left:
                state["i"] = (state["i"] - 1) % len(shots)
                show()
            return True
        win.connect("key-press-event", key)
        win.connect("button-press-event", lambda *_: win.destroy())
        win.add_events(Gdk.EventMask.BUTTON_PRESS_MASK)
        show()
        win.show_all()
        self.shot_viewer = win
        return win

    def _set_permission(self, app, perm, allowed):
        try:
            privacy.set_allowed(app["id"], perm, allowed)
        except OSError as e:
            self._status(f"That wasn't saved: {e}", error=True)
            return
        name = app.get("name") or app["id"]
        words = engine.PERMISSIONS.get(perm, ("", perm))[1]
        self._status(f"{name} may {words[0].lower() + words[1:]} again." if allowed else
                     f"{name} can't {words[0].lower() + words[1:]} any more, from the next time it starts.")

    # ---- actions ---------------------------------------------------------

    def _update_all(self):
        """Every update, one after the other; an unverified one is left for you to decide on its page."""
        apps = [self._find(a["id"]) for a in self.installed if a.get("state") == "update" and a.get("catalogue")]
        apps = [a for a in apps if a]
        if not apps:
            return
        self._set_busy(True)
        self.update_all_btn.set_sensitive(False)

        def work():
            done, waiting, failed = [], [], []
            for n, app in enumerate(apps, 1):
                name = app.get("name") or app["id"]
                GLib.idle_add(self._status, f"Updating {name} ({n} of {len(apps)})…")
                GLib.idle_add(self.progress.set_fraction, (n - 1) / len(apps))
                try:
                    got = engine.download(app["id"])
                    engine.install(got["path"], False, self._ask_password)
                    done.append(name)
                except broker.Unverified:
                    waiting.append(name)
                except (engine.StoreError, broker.BrokerError) as e:
                    if str(e) == "cancelled":
                        failed.append(f"{name} (cancelled)")
                        break
                    failed.append(f"{name}: {e}")
            parts = []
            if done:
                parts.append(f"Updated {', '.join(done)}.")
            if waiting:
                parts.append(f"{', '.join(waiting)}: its publisher isn't verified - open its page to decide.")
            if failed:
                parts.append("Not updated: " + "; ".join(failed) + ".")
            GLib.idle_add(self._action_done, " ".join(parts), None if done or waiting else " ".join(parts), None)

        threading.Thread(target=work, daemon=True).start()

    def _ask_password(self, what, error):
        # From the worker thread; askpass shows its dialog on the main thread.
        return askpass.ask(f"{what or 'Installing an app'} needs your password.", self)(error is not None)

    def _set_busy(self, busy):
        self.busy = busy
        self.refresh_btn.set_sensitive(not busy)
        if busy:
            self.progress.set_fraction(0)
            self.progress.show()
        else:
            self.progress.hide()
        if self.current:
            self._show_app(self.current)

    def _install(self, app, allow_unverified=False):
        name = app.get("name") or app["id"]
        self._set_busy(True)
        self._status(f"Downloading {name}…")

        def progress(p):
            total = p.get("total") or 0
            if total:
                GLib.idle_add(self.progress.set_fraction, min(1.0, p.get("done", 0) / total))

        def work():
            try:
                got = engine.download(app["id"], progress=progress)
                GLib.idle_add(self._status, f"Installing {name}…")
                GLib.idle_add(self.progress.pulse)
                result = engine.install(got["path"], allow_unverified, self._ask_password)
                GLib.idle_add(self._action_done, (result or {}).get("done") or f"Installed {name}", None, None)
            except broker.Unverified as e:
                GLib.idle_add(self._action_done, None, str(e), app)
            except (engine.StoreError, broker.BrokerError) as e:
                msg = "Cancelled." if str(e) == "cancelled" else str(e)
                GLib.idle_add(self._action_done, None, msg, None)

        threading.Thread(target=work, daemon=True).start()

    def _action_done(self, done, error, unverified_app):
        self._set_busy(False)
        if unverified_app is not None:
            if self._confirm("Install an app from an unverified publisher?",
                             f"{error}\n\nNexaOS still limits it to the permissions shown, but nobody NexaOS "
                             "trusts vouches for it. Install anyway?"):
                self._install(unverified_app, allow_unverified=True)
            else:
                self._status("Not installed.")
            return False
        if error:
            self._status(error, error=True)
            return False
        self._status(done)
        self._keep_status = done  # still there after the reload below
        self.reload()
        return False

    def _remove(self, app):
        name = app.get("name") or app["id"]
        if not self._confirm(f"Remove {name}?", "The app goes; your own files stay where they are."):
            return
        self._set_busy(True)
        self.progress.pulse()
        self._status(f"Removing {name}…")

        def work():
            try:
                result = engine.uninstall(app["id"], self._ask_password)
                GLib.idle_add(self._action_done, (result or {}).get("done") or f"Removed {name}", None, None)
            except broker.BrokerError as e:
                GLib.idle_add(self._action_done, None, "Cancelled." if str(e) == "cancelled" else str(e), None)

        threading.Thread(target=work, daemon=True).start()

    def _launch(self, app):
        info = Gio.DesktopAppInfo.new(f"nexa-{app['id']}.desktop")
        try:
            if info is None or not info.launch([], None):
                raise GLib.Error("no menu entry")
            self._status(f"Opening {app.get('name') or app['id']}…")
        except GLib.Error:
            self._status(f"Couldn't open {app.get('name') or app['id']} - try it from Launchpad.", error=True)

    def _confirm(self, text, secondary):
        dialog = Gtk.MessageDialog(transient_for=self, flags=0, message_type=Gtk.MessageType.QUESTION,
                                   buttons=Gtk.ButtonsType.YES_NO, text=text)
        dialog.format_secondary_text(secondary)
        answer = dialog.run()
        dialog.destroy()
        return answer == Gtk.ResponseType.YES

    def _status(self, text, error=False):
        self.status.set_text(text or "")
        ctx = self.status.get_style_context()
        (ctx.add_class if error else ctx.remove_class)("error")
        return False

    def _on_key(self, _w, e):
        ctrl = e.state & Gdk.ModifierType.CONTROL_MASK
        alt = e.state & Gdk.ModifierType.MOD1_MASK
        if ctrl and e.keyval in (Gdk.KEY_f, Gdk.KEY_F):
            self.search.grab_focus()
            return True
        if ctrl and e.keyval in (Gdk.KEY_r, Gdk.KEY_R):
            self.reload(refresh=True)
            return True
        if ctrl and e.keyval in (Gdk.KEY_q, Gdk.KEY_Q):
            self.close()
            return True
        if (e.keyval == Gdk.KEY_Escape or (alt and e.keyval == Gdk.KEY_Left)) and self.pages.get_visible_child_name() == "app":
            self._show_list()
            return True
        return False


def main():
    Gtk.init()
    win = StoreWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    # `nexa-store-app ID` opens that app's page once the catalogues are read.
    if len(sys.argv) > 1:
        wanted = sys.argv[1]

        def open_when_ready():
            if win.busy:
                return True
            win._open_app(wanted)
            return False

        GLib.timeout_add(200, open_when_ready)
    Gtk.main()
