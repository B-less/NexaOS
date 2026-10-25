import json
import os
import shutil
import subprocess
import sys
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GdkPixbuf, GLib

from . import privileged
from .connections import BluetoothPage, VpnPage
from .privacy_page import PrivacyPage
from .system_pages import StartupPage, UpdatesPage, UsersPage
from nexa_common import actionui, compositor, mind, persist, sysconf, themes
from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#sidebar { background-image: none; background-color: @nexa_surface; }
#navbtn { background-image: none; background-color: transparent; color: @nexa_muted; border-radius: 6px; padding: 8px 14px; }
#navbtn:hover { background-image: none; background-color: @nexa_surface_hi; }
#navbtn.active { background-image: none; background-color: @nexa_selected; color: @nexa_selected_text; }
#pagetitle { color: @nexa_text_strong; font-size: 16pt; font-weight: bold; }
#fieldlabel { color: @nexa_muted; font-size: 9pt; }
#value { color: @nexa_text; }
#entry { background-image: none; background-color: @nexa_field; color: @nexa_text_strong; border-radius: 6px; padding: 6px 10px; border: 1px solid @nexa_border; }
#actionbtn { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 6px; padding: 6px 14px; font-weight: bold; }
#actionbtn:hover { background-image: none; background-color: @nexa_accent_hover; }
#actionbtn:disabled { background-color: @nexa_hover; color: @nexa_disabled; }
#rowbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 12px; border: 1px solid @nexa_border; }
#rowbtn:hover { background-image: none; background-color: @nexa_hover; }
list, list row { background-image: none; background-color: transparent; }
list row { border-bottom: 1px solid @nexa_border; }
#toggle { border-radius: 12px; padding: 4px 12px; font-weight: bold; font-size: 9pt; }
#toggle.on { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; }
#toggle.off { background-image: none; background-color: @nexa_hover; color: @nexa_muted; }
#status { color: @nexa_ok; font-size: 9pt; }
#error { color: @nexa_crit; font-size: 9pt; }
#wordmark { color: @nexa_text_strong; font-size: 22pt; font-weight: bold; }
#themecard { background-image: none; background-color: @nexa_surface; border-radius: 14px; padding: 8px; border: 2px solid transparent; }
#themecard:hover { background-color: @nexa_surface_hi; }
#themecard.chosen { border-color: @nexa_accent; }
"""

WALLPAPER_PATH = "/usr/share/backgrounds/nexaos/wallpaper.png"
WALLPAPER_GALLERY = "/usr/share/backgrounds/nexaos/gallery"
LOGIN_REQUIRED = "/etc/nexa/login-required"  # written by the OS installer
SKIP_LOGIN_LOCK = os.path.expanduser("~/.config/nexa/skip-login-lock")
AI_CONFIG_PATH = "/etc/nexa-ai/config.json"
PAGES = ("Personalization", "Bluetooth", "VPN", "System", "Users", "Updates", "Startup", "Time & Keyboard", "Power", "Privacy", "Assistant", "About")


class SettingsWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Settings")
        self.set_icon_name("nexa-settings")
        attach_titlebar(self, "Settings")
        self.set_default_size(760, 520)
        self._build_ui()

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=0)
        self.add(root)

        sidebar = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        sidebar.set_name("sidebar")
        sidebar.set_border_width(10)
        sidebar.set_size_request(180, -1)
        self.nav_buttons = {}
        for name in PAGES:
            btn = Gtk.Button(label=name)
            btn.set_name("navbtn")
            btn.set_relief(Gtk.ReliefStyle.NONE)
            btn.connect("clicked", self._on_nav, name)
            sidebar.pack_start(btn, False, False, 0)
            self.nav_buttons[name] = btn
        # Scrolls on small screens: there are a dozen pages.
        side_scroll = Gtk.ScrolledWindow()
        side_scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        side_scroll.add(sidebar)
        root.pack_start(side_scroll, False, False, 0)

        self.stack = Gtk.Stack()
        self.stack.set_transition_type(Gtk.StackTransitionType.CROSSFADE)
        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        content.set_border_width(24)
        content.pack_start(self.stack, True, True, 0)
        root.pack_start(content, True, True, 0)

        self.stack.add_named(self._scrolled(self._build_appearance_page()), "Personalization")
        self.bluetooth_page = BluetoothPage(self)
        self.stack.add_named(self._scrolled(self.bluetooth_page.page), "Bluetooth")
        self.vpn_page = VpnPage(self)
        self.stack.add_named(self._scrolled(self.vpn_page.page), "VPN")
        self.stack.add_named(self._scrolled(self._build_system_page()), "System")
        self.users_page = UsersPage(self)
        self.stack.add_named(self._scrolled(self.users_page.page), "Users")
        self.updates_page = UpdatesPage(self)
        self.stack.add_named(self._scrolled(self.updates_page.page), "Updates")
        self.startup_page = StartupPage(self)
        self.stack.add_named(self._scrolled(self.startup_page.page), "Startup")
        self.stack.add_named(self._scrolled(self._build_time_page()), "Time & Keyboard")
        self.stack.add_named(self._scrolled(self._build_power_page()), "Power")
        self.privacy_page = PrivacyPage(self)
        self.stack.add_named(self._scrolled(self.privacy_page.page), "Privacy")
        self.stack.add_named(self._scrolled(self._build_assistant_page()), "Assistant")
        self.stack.add_named(self._scrolled(self._build_about_page()), "About")

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )
        self._on_nav(None, "Personalization")

    def _on_nav(self, _btn, name):
        if name == "Privacy":
            self.privacy_page.reload()  # apps may have been installed meanwhile
        elif name == "Users":
            self.users_page.reload()
        elif name == "Startup":
            self.startup_page.reload()
        self.stack.set_visible_child_name(name)
        for n, b in self.nav_buttons.items():
            ctx = b.get_style_context()
            ctx.remove_class("active")
            if n == name:
                ctx.add_class("active")

    def _scrolled(self, page):
        # A page taller than the screen otherwise forces the whole window to
        # that height: it opened under the top bar with its bottom cut off.
        scroller = Gtk.ScrolledWindow()
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroller.add(page)
        return scroller

    def _page(self, title):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
        label = Gtk.Label(label=title)
        label.set_name("pagetitle")
        label.set_xalign(0)
        box.pack_start(label, False, False, 0)
        return box

    def _field_label(self, text):
        label = Gtk.Label(label=text)
        label.set_name("fieldlabel")
        label.set_xalign(0)
        return label

    # ---- Appearance ----

    def _build_appearance_page(self):
        page = self._page("Personalization")

        # Themes: each card previews the theme's background and accents.
        page.pack_start(self._field_label("Theme"), False, False, 0)
        saved = themes.load_settings()
        self._theme_choice = saved["theme"]
        css = []
        cards = Gtk.FlowBox()
        cards.set_selection_mode(Gtk.SelectionMode.NONE)
        cards.set_max_children_per_line(5)
        cards.set_column_spacing(10)
        cards.set_row_spacing(10)
        self._theme_buttons = {}
        for key, t in themes.THEMES.items():
            bg = "rgb(%d, %d, %d)" % t["bg"]
            css.append(f"#swatch-{key} {{ background-color: {bg}; border-radius: 10px; }}")
            css.append(f"#dot1-{key} {{ background-color: {t['accent']}; border-radius: 7px; }}")
            css.append(f"#dot2-{key} {{ background-color: {t['accent2']}; border-radius: 7px; }}")
            css.append(f"#dot3-{key} {{ background-color: {t['text']}; border-radius: 7px; }}")
            btn = Gtk.Button()
            btn.set_name("themecard")
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
            swatch = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
            swatch.set_name(f"swatch-{key}")
            swatch.set_size_request(150, 70)
            swatch.set_border_width(12)
            for dot in ("dot1", "dot2", "dot3"):
                d = Gtk.Box()
                d.set_name(f"{dot}-{key}")
                d.set_size_request(14, 14)
                d.set_valign(Gtk.Align.END)
                swatch.pack_start(d, False, False, 0)
            box.pack_start(swatch, False, False, 0)
            name = Gtk.Label(label=t["label"])
            name.set_name("value")
            box.pack_start(name, False, False, 0)
            tag = Gtk.Label(label=t["tagline"])
            tag.set_name("fieldlabel")
            tag.set_line_wrap(True)
            tag.set_max_width_chars(20)
            box.pack_start(tag, False, False, 0)
            btn.add(box)
            btn.connect("clicked", lambda _b, k=key: self._pick_theme(k))
            self._theme_buttons[key] = btn
            cards.add(btn)
        provider = Gtk.CssProvider()
        provider.load_from_data("\n".join(css).encode())
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider,
                                                 Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        page.pack_start(cards, False, False, 0)
        self._mark_theme()

        page.pack_start(self._field_label("Transparency of the bar, panels and terminal"), False, False, 0)
        self._opacity = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL,
                                                 themes.MIN_OPACITY * 100, themes.MAX_OPACITY * 100, 1)
        self._opacity.set_value(saved["opacity"] * 100)
        self._opacity.set_draw_value(True)
        self._opacity.set_value_pos(Gtk.PositionType.RIGHT)
        self._opacity.connect("format-value", lambda _s, v: f"{int(round(100 - v))}% see-through")
        page.pack_start(self._opacity, False, False, 0)

        glass_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        glass_row.pack_start(self._field_label("Glass windows: let the desktop show through app windows "
                                               "(blurred on Hyprland)"), True, True, 0)
        self._glass = Gtk.Switch(active=saved["glass"], valign=Gtk.Align.CENTER)
        self._glass.connect("notify::active", lambda sw, _p: themes.apply(glass=sw.get_active()))
        glass_row.pack_start(self._glass, False, False, 0)
        page.pack_start(glass_row, False, False, 0)

        # App icons: full colour, dark, or tinted in the theme's accent
        # (NexaHexa icons; they also follow the theme).
        icons_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        icons_row.pack_start(self._field_label("App icons: full colour, dark, or tinted in the theme's colour"), True, True, 0)
        self._icon_style = Gtk.ComboBoxText(valign=Gtk.Align.CENTER)
        for key, label in (("colour", "Colour"), ("dark", "Dark"), ("tinted", "Tinted")):
            self._icon_style.append(key, label)
        self._icon_style.set_active_id(saved.get("icon_style", "colour"))
        if themes.icon_styles_available():
            self._icon_style.connect("changed", lambda c: themes.apply(icon_style=c.get_active_id()))
        else:
            self._icon_style.set_sensitive(False)
            self._icon_style.set_tooltip_text("Comes with the NexaHexa icons, which aren't installed yet")
        icons_row.pack_start(self._icon_style, False, False, 0)
        page.pack_start(icons_row, False, False, 0)

        apply_btn = Gtk.Button(label="Apply Theme")
        apply_btn.set_name("actionbtn")
        apply_btn.set_halign(Gtk.Align.START)
        apply_btn.connect("clicked", self._apply_theme)
        page.pack_start(apply_btn, False, False, 0)

        page.pack_start(self._field_label("Desktop wallpaper"), False, False, 0)

        gallery = Gtk.FlowBox()
        gallery.set_selection_mode(Gtk.SelectionMode.NONE)
        gallery.set_max_children_per_line(4)
        gallery.set_column_spacing(10)
        gallery.set_row_spacing(10)
        gallery.set_halign(Gtk.Align.START)  # keep thumbnails compact, not stretched
        for path in self._gallery_images():
            try:
                thumb = GdkPixbuf.Pixbuf.new_from_file_at_scale(path, 192, 108, False)
            except GLib.Error:
                continue
            btn = Gtk.Button()
            btn.set_image(Gtk.Image.new_from_pixbuf(thumb))
            btn.set_tooltip_text(os.path.splitext(os.path.basename(path))[0].replace("-", " ").title())
            btn.connect("clicked", lambda _b, p=path: self._apply_wallpaper(p))
            gallery.add(btn)
        page.pack_start(gallery, False, False, 0)

        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        self.wallpaper_path_label = Gtk.Label(label=WALLPAPER_PATH)
        self.wallpaper_path_label.set_name("value")
        self.wallpaper_path_label.set_xalign(0)
        row.pack_start(self.wallpaper_path_label, True, True, 0)
        choose_btn = Gtk.Button(label="Upload Your Own...")
        choose_btn.set_name("actionbtn")
        choose_btn.connect("clicked", self._on_choose_wallpaper)
        row.pack_start(choose_btn, False, False, 0)
        page.pack_start(row, False, False, 0)

        self.appearance_status = Gtk.Label(label="")
        self.appearance_status.set_xalign(0)
        page.pack_start(self.appearance_status, False, False, 0)
        return page

    def _pick_theme(self, key):
        self._theme_choice = key
        self._mark_theme()

    def _mark_theme(self):
        for key, btn in self._theme_buttons.items():
            ctx = btn.get_style_context()
            (ctx.add_class if key == self._theme_choice else ctx.remove_class)("chosen")

    def _apply_theme(self, _btn):
        try:
            applied = themes.apply(self._theme_choice, self._opacity.get_value() / 100)
        except OSError as e:
            self._set_label(self.appearance_status, f"Couldn't apply the theme: {e}", error=True)
            return
        label = themes.THEMES[applied["theme"]]["label"]
        self._set_label(self.appearance_status, f"{label} theme applied.", error=False)

    def _gallery_images(self):
        try:
            names = sorted(os.listdir(WALLPAPER_GALLERY))
        except OSError:
            return []
        return [os.path.join(WALLPAPER_GALLERY, n) for n in names
                if n.lower().endswith((".png", ".jpg", ".jpeg"))]

    def _on_choose_wallpaper(self, _btn):
        dialog = Gtk.FileChooserDialog(
            title="Choose Wallpaper", transient_for=self, action=Gtk.FileChooserAction.OPEN,
        )
        dialog.add_buttons(Gtk.STOCK_CANCEL, Gtk.ResponseType.CANCEL, Gtk.STOCK_OPEN, Gtk.ResponseType.OK)
        img_filter = Gtk.FileFilter()
        img_filter.set_name("Images")
        for pattern in ("*.png", "*.jpg", "*.jpeg"):
            img_filter.add_pattern(pattern)
        dialog.add_filter(img_filter)
        if dialog.run() == Gtk.ResponseType.OK:
            self._apply_wallpaper(dialog.get_filename())
        dialog.destroy()

    def _apply_wallpaper(self, path):
        try:
            shutil.copyfile(path, WALLPAPER_PATH)
            if not compositor.set_wallpaper(WALLPAPER_PATH):
                raise OSError("the desktop session didn't accept the new wallpaper")
        except OSError as e:
            self._set_label(self.appearance_status, f"Couldn't set wallpaper: {e}", error=True)
            return
        self._set_label(self.appearance_status, "Wallpaper updated.", error=False)

    def _set_label(self, label, text, error):
        label.set_name("error" if error else "status")
        label.set_text(text)
        label.get_style_context().add_class("error" if error else "status")

    # ---- System ----

    def _build_system_page(self):
        page = self._page("System")
        info = _system_info()
        grid = Gtk.Grid(row_spacing=8, column_spacing=16)
        for i, (label, value) in enumerate(info):
            l = self._field_label(label)
            v = Gtk.Label(label=value)
            v.set_name("value")
            v.set_xalign(0)
            grid.attach(l, 0, i, 1, 1)
            grid.attach(v, 1, i, 1, 1)
        page.pack_start(grid, False, False, 0)

        page.pack_start(Gtk.Box(), False, False, 6)
        page.pack_start(self._field_label("Hostname"), False, False, 0)
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        self.hostname_entry = Gtk.Entry()
        self.hostname_entry.set_name("entry")
        self.hostname_entry.set_text(os.uname().nodename)
        row.pack_start(self.hostname_entry, True, True, 0)
        apply_btn = Gtk.Button(label="Apply")
        apply_btn.set_name("actionbtn")
        apply_btn.connect("clicked", self._on_apply_hostname)
        row.pack_start(apply_btn, False, False, 0)
        page.pack_start(row, False, False, 0)

        self.system_status = Gtk.Label(label="")
        self.system_status.set_xalign(0)
        page.pack_start(self.system_status, False, False, 0)

        st = persist.status()
        if st["live"]:
            self._build_persist_section(page, st)
        return page

    def _build_persist_section(self, page, st):
        page.pack_start(Gtk.Box(), False, False, 6)
        page.pack_start(self._field_label("Live USB"), False, False, 0)
        if st["active"]:
            text = ("Your files and settings are kept on this USB stick between restarts. "
                    "To start once without them, choose \"Live, don't load saved changes\" in the boot menu (Esc).")
        elif st["possible"]:
            text = (f"Right now everything you do is forgotten when you shut down. NexaOS can keep your files, "
                    f"settings, Wi-Fi networks and apps on this stick instead, using its {st['free_gb']} GB of free "
                    f"space. What you have now is copied over.")
        else:
            text = f"Files are forgotten when you shut down. {st['reason']}"
        info = Gtk.Label(label=text, xalign=0)
        info.set_line_wrap(True)
        info.set_name("value")
        page.pack_start(info, False, False, 0)
        self.persist_status = Gtk.Label(label="", xalign=0)
        self.persist_status.set_line_wrap(True)
        if st["possible"]:
            btn = Gtk.Button(label="Keep my files on this USB")
            btn.set_name("actionbtn")
            btn.set_halign(Gtk.Align.START)
            btn.connect("clicked", self._on_persist)
            page.pack_start(btn, False, False, 0)
            self.persist_btn = btn
        page.pack_start(self.persist_status, False, False, 0)

    def _on_persist(self, btn):
        btn.set_sensitive(False)
        self._set_label(self.persist_status, "Setting up the stick, this takes a minute…", error=False)

        def work():
            try:
                result = privileged.call("persist_create", timeout=600)
                GLib.idle_add(self._set_label, self.persist_status,
                              f"Done: {result['size_gb']} GB for your files. Restart to start using it.", False)
            except privileged.PrivilegedError as e:
                GLib.idle_add(self._set_label, self.persist_status, f"Couldn't set it up: {e}", True)
                GLib.idle_add(btn.set_sensitive, True)
        threading.Thread(target=work, daemon=True).start()

    def _on_apply_hostname(self, _btn):
        name = self.hostname_entry.get_text().strip()
        try:
            privileged.call("set_hostname", name=name)
        except privileged.PrivilegedError as e:
            self._set_label(self.system_status, f"Couldn't set hostname: {e}", error=True)
            return
        self._set_label(self.system_status, "Hostname updated. Takes full effect after a reboot.", error=False)

    # ---- Time & Keyboard ----

    def _build_time_page(self):
        page = self._page("Time & Keyboard")

        page.pack_start(self._field_label("Time zone"), False, False, 0)
        self._zone_regions = {}
        for zone in sysconf.list_zones():
            region, _, city = zone.partition("/")
            if not city:
                region, city = "Other", zone
            self._zone_regions.setdefault(region, []).append((city, zone))
        self._city_zones = []
        zone_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        self.region_combo = Gtk.ComboBoxText()
        self.city_combo = Gtk.ComboBoxText()
        for region in sorted(self._zone_regions):
            self.region_combo.append_text(region)
        self.region_combo.connect("changed", self._on_region_changed)
        apply_zone = Gtk.Button(label="Apply")
        apply_zone.set_name("actionbtn")
        apply_zone.connect("clicked", self._on_apply_zone)
        zone_row.pack_start(self.region_combo, False, False, 0)
        zone_row.pack_start(self.city_combo, True, True, 0)
        zone_row.pack_start(apply_zone, False, False, 0)
        page.pack_start(zone_row, False, False, 0)
        self._select_zone(sysconf.current_zone())

        page.pack_start(Gtk.Box(), False, False, 6)
        page.pack_start(self._field_label("Keyboard layout"), False, False, 0)
        self._layouts = sysconf.list_layouts()
        layout_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        self.layout_combo = Gtk.ComboBoxText()
        current_layout = sysconf.read_keyboard_layout()
        for i, (code, desc) in enumerate(self._layouts):
            self.layout_combo.append_text(f"{desc} ({code})")
            if code == current_layout:
                self.layout_combo.set_active(i)
        apply_layout = Gtk.Button(label="Apply")
        apply_layout.set_name("actionbtn")
        apply_layout.connect("clicked", self._on_apply_layout)
        layout_row.pack_start(self.layout_combo, True, True, 0)
        layout_row.pack_start(apply_layout, False, False, 0)
        page.pack_start(layout_row, False, False, 0)

        self.time_status = Gtk.Label(label="")
        self.time_status.set_xalign(0)
        self.time_status.set_line_wrap(True)
        page.pack_start(self.time_status, False, False, 0)
        return page

    def _select_zone(self, zone):
        region, _, city = zone.partition("/")
        if not city:
            region = "Other"
        regions = sorted(self._zone_regions)
        if region in regions:
            self.region_combo.set_active(regions.index(region))
            for i, (_c, z) in enumerate(self._city_zones):
                if z == zone:
                    self.city_combo.set_active(i)
        elif regions:
            self.region_combo.set_active(0)

    def _on_region_changed(self, combo):
        self.city_combo.remove_all()
        self._city_zones = sorted(self._zone_regions.get(combo.get_active_text(), []))
        for city, _zone in self._city_zones:
            self.city_combo.append_text(city.replace("_", " "))
        if self._city_zones:
            self.city_combo.set_active(0)

    def _restart_background(self, name, argv):
        subprocess.run(["pkill", "-x", name], check=False)
        try:
            subprocess.Popen(argv, start_new_session=True,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except OSError:
            pass

    def _on_apply_zone(self, _btn):
        idx = self.city_combo.get_active()
        if idx < 0 or idx >= len(self._city_zones):
            return
        zone = self._city_zones[idx][1]
        try:
            privileged.call("set_timezone", zone=zone)
        except privileged.PrivilegedError as e:
            self._set_label(self.time_status, f"Couldn't set time zone: {e}", error=True)
            return
        # The bar's clock resolves the zone once at startup.
        self._restart_background("waybar", ["waybar"])
        self._set_label(self.time_status, f"Time zone set to {zone}.", error=False)

    def _on_apply_layout(self, _btn):
        idx = self.layout_combo.get_active()
        if idx < 0 or idx >= len(self._layouts):
            return
        code = self._layouts[idx][0]
        if not sysconf.valid_layout(code, self._layouts):
            self._set_label(self.time_status, "Unknown keyboard layout.", error=True)
            return
        try:
            if not compositor.set_keyboard_layout(code):
                raise OSError("the desktop session didn't accept the layout")
            sysconf.write_keyboard_layout(code)
        except OSError as e:
            self._set_label(self.time_status, f"Couldn't change layout: {e}", error=True)
            return
        self._set_label(self.time_status, f"Keyboard layout set to {code}.", error=False)

    # ---- Power ----

    def _build_power_page(self):
        page = self._page("Power")
        idle = sysconf.read_idle()
        self.lock_spin = Gtk.SpinButton.new_with_range(0, sysconf.MAX_IDLE_MINUTES, 1)
        self.lock_spin.set_value(idle["lock_minutes"])
        self.blank_spin = Gtk.SpinButton.new_with_range(0, sysconf.MAX_IDLE_MINUTES, 1)
        self.blank_spin.set_value(idle["blank_minutes"])

        for label, spin in (("Lock the screen after (minutes, 0 = never)", self.lock_spin),
                            ("Turn the screen off after (minutes, 0 = never)", self.blank_spin)):
            page.pack_start(self._field_label(label), False, False, 0)
            page.pack_start(spin, False, False, 0)

        apply_btn = Gtk.Button(label="Apply")
        apply_btn.set_name("actionbtn")
        apply_btn.set_halign(Gtk.Align.START)
        apply_btn.connect("clicked", self._on_apply_idle)
        page.pack_start(apply_btn, False, False, 0)

        if os.path.exists(LOGIN_REQUIRED):
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
            row.pack_start(self._field_label("Ask for my password when NexaOS starts"), True, True, 0)
            self.login_switch = Gtk.Switch()
            self.login_switch.set_active(not os.path.exists(SKIP_LOGIN_LOCK))
            self.login_switch.connect("notify::active", self._on_login_lock_toggled)
            row.pack_start(self.login_switch, False, False, 0)
            page.pack_start(row, False, False, 8)

        self.power_status = Gtk.Label(label="")
        self.power_status.set_xalign(0)
        page.pack_start(self.power_status, False, False, 0)
        return page

    def _on_login_lock_toggled(self, switch, _pspec):
        try:
            if switch.get_active():
                if os.path.exists(SKIP_LOGIN_LOCK):
                    os.remove(SKIP_LOGIN_LOCK)
            else:
                os.makedirs(os.path.dirname(SKIP_LOGIN_LOCK), exist_ok=True)
                open(SKIP_LOGIN_LOCK, "w").close()
        except OSError as e:
            self._set_label(self.power_status, f"Couldn't save: {e}", error=True)
            return
        self._set_label(self.power_status,
                        "Your password will be asked at startup." if switch.get_active()
                        else "NexaOS will open without asking for your password.", error=False)

    def _on_apply_idle(self, _btn):
        try:
            sysconf.write_idle(self.lock_spin.get_value_as_int(), self.blank_spin.get_value_as_int())
        except OSError as e:
            self._set_label(self.power_status, f"Couldn't save: {e}", error=True)
            return
        self._restart_background("swayidle", ["nexa-idle"])
        self._set_label(self.power_status, "Saved. Applied now.", error=False)

    # ---- Assistant ----

    def _build_assistant_page(self):
        page = self._page("Assistant")
        cfg = _load_ai_config()

        # Nexa Mind: the assistant on this computer (nexa-mindd, no internet).
        page.pack_start(self._field_label("Nexa Mind — the assistant on this computer"), False, False, 0)
        intro = Gtk.Label(label="Answers and acts without the internet; nothing you ask leaves this computer. "
                                "Needs a one-time download of the model.", xalign=0)
        intro.set_line_wrap(True)
        intro.set_name("value")
        page.pack_start(intro, False, False, 0)
        self.mind_status = Gtk.Label(label="", xalign=0)
        self.mind_status.set_line_wrap(True)
        page.pack_start(self.mind_status, False, False, 0)
        self.mind_progress = Gtk.ProgressBar()
        self.mind_progress.set_no_show_all(True)
        page.pack_start(self.mind_progress, False, False, 0)
        self.mind_download = Gtk.Button(label="Download the offline model")
        self.mind_download.set_name("actionbtn")
        self.mind_download.set_halign(Gtk.Align.START)
        self.mind_download.connect("clicked", self._on_mind_download)
        self.mind_download.set_no_show_all(True)
        page.pack_start(self.mind_download, False, False, 0)

        page.pack_start(self._field_label("Which assistant answers"), False, False, 0)
        self.backend_combo = Gtk.ComboBoxText()
        for key, label in (("auto", "Automatic: this computer if the model is installed, else Claude"),
                           ("local", "Only this computer (never online)"),
                           ("claude", "Only Claude (online)")):
            self.backend_combo.append(key, label)
        self.backend_combo.set_active_id(cfg.get("backend", "auto"))
        page.pack_start(self.backend_combo, False, False, 0)
        page.pack_start(Gtk.Box(), False, False, 6)
        self._refresh_mind()
        GLib.timeout_add_seconds(2, self._tick_mind)

        page.pack_start(self._field_label("Claude API key"), False, False, 0)
        self.api_key_entry = Gtk.Entry()
        self.api_key_entry.set_name("entry")
        self.api_key_entry.set_visibility(False)
        self.api_key_entry.set_text(cfg.get("api_key", ""))
        page.pack_start(self.api_key_entry, False, False, 0)

        page.pack_start(self._field_label("Model"), False, False, 0)
        self.model_entry = Gtk.Entry()
        self.model_entry.set_name("entry")
        self.model_entry.set_text(cfg.get("model", "claude-opus-5-5"))
        page.pack_start(self.model_entry, False, False, 0)

        self.allow_control_toggle = self._toggle_row(
            page, "Allow control actions", cfg.get("allow_control", False),
        )
        self.require_confirmation_toggle = self._toggle_row(
            page, "Ask before the assistant changes anything", cfg.get("require_confirmation", True),
        )

        save_btn = Gtk.Button(label="Save")
        save_btn.set_name("actionbtn")
        save_btn.connect("clicked", self._on_save_assistant)
        page.pack_start(save_btn, False, False, 0)

        self.assistant_status = Gtk.Label(label="")
        self.assistant_status.set_xalign(0)
        page.pack_start(self.assistant_status, False, False, 0)

        # Activity: every file change the assistant made (nexa-tools' journal), with Undo.
        page.pack_start(self._field_label("Activity"), False, False, 0)
        self.activity = actionui.ActivityTimeline(limit=30)
        page.pack_start(self.activity, False, False, 0)
        return page

    def _tick_mind(self):
        if self.stack.get_visible_child_name() == "Assistant":
            self._refresh_mind()
        return True

    def _refresh_mind(self):
        st = mind.status()
        if st is None:
            self._set_label(self.mind_status, "The Nexa Mind service isn't running.", error=True)
            self.mind_download.hide()
            self.mind_progress.hide()
            return
        catalogue = {m["name"]: m for m in mind.models()}
        name = st.get("default_model")
        model = catalogue.get(name, {})
        dl = (st.get("downloads") or {}).get(name)
        if name in (st.get("installed") or []):
            state = {"idle": "ready (loads when you ask something)", "loading": "loading the model…",
                     "ready": "ready and loaded"}.get(st.get("state"), st.get("state"))
            self._set_label(self.mind_status, f"{model.get('title', name)}: {state}. "
                            f"Uses {st.get('threads')} CPU threads.", error=False)
            self.mind_download.hide()
            self.mind_progress.hide()
        elif dl and dl["phase"] in ("Downloading", "Verifying"):
            total = max(dl.get("total") or 1, 1)
            self.mind_progress.set_fraction(min(dl.get("bytes", 0) / total, 1.0))
            self.mind_progress.show()
            verb = "Checking the download" if dl["phase"] == "Verifying" else "Downloading"
            self._set_label(self.mind_status, f"{verb}: {dl.get('bytes', 0) / 1e9:.2f} of {total / 1e9:.2f} GB",
                            error=False)
            self.mind_download.hide()
        else:
            size = model.get("size", 0) / 1e9
            failed = dl["phase"].get("Failed") if dl and isinstance(dl.get("phase"), dict) else None
            msg = f"Last download failed: {failed}. " if failed else ""
            if persist.status()["live"] and not persist.status()["active"]:
                msg += ("On the live USB the model would only be kept in memory until you shut down; "
                        "turn on \"Keep my files on this USB\" (System) or install NexaOS first.")
            self._set_label(self.mind_status, msg or "Not downloaded yet.", error=bool(failed))
            self.mind_download.set_label(f"Download the offline model ({size:.1f} GB)")
            self.mind_download.show()
            self.mind_progress.hide()
        return False

    def _on_mind_download(self, _btn):
        try:
            mind.download(mind.status()["default_model"])
        except (mind.MindError, mind.MindUnreachable, TypeError) as e:
            self._set_label(self.mind_status, f"Couldn't start the download: {e}", error=True)
            return
        self._refresh_mind()

    def _toggle_row(self, page, label_text, initial):
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        label = Gtk.Label(label=label_text)
        label.set_xalign(0)
        row.pack_start(label, True, True, 0)
        toggle = Gtk.ToggleButton()
        toggle.set_name("toggle")
        toggle.set_active(initial)
        toggle.connect("toggled", self._on_toggle_changed)
        row.pack_start(toggle, False, False, 0)
        page.pack_start(row, False, False, 0)
        self._update_toggle_label(toggle)
        return toggle

    def _on_toggle_changed(self, toggle):
        self._update_toggle_label(toggle)

    def _update_toggle_label(self, toggle):
        on = toggle.get_active()
        toggle.set_label("On" if on else "Off")
        ctx = toggle.get_style_context()
        ctx.remove_class("on")
        ctx.remove_class("off")
        ctx.add_class("on" if on else "off")

    def _on_save_assistant(self, _btn):
        data = {
            "api_key": self.api_key_entry.get_text(),
            "model": self.model_entry.get_text().strip() or "claude-opus-5-5",
            "allow_control": self.allow_control_toggle.get_active(),
            "require_confirmation": self.require_confirmation_toggle.get_active(),
            "backend": self.backend_combo.get_active_id() or "auto",
        }
        try:
            privileged.call("write_ai_config", data=data)
        except privileged.PrivilegedError as e:
            self._set_label(self.assistant_status, f"Couldn't save: {e}", error=True)
            return
        self._set_label(self.assistant_status, "Saved.", error=False)

    # ---- About ----

    def _build_about_page(self):
        page = self._page("About")
        wordmark = Gtk.Label(label="NexaOS")
        wordmark.set_name("wordmark")
        wordmark.set_xalign(0)
        page.pack_start(wordmark, False, False, 0)
        tagline = Gtk.Label(label="A modern desktop, built from scratch.")
        tagline.set_xalign(0)
        page.pack_start(tagline, False, False, 0)

        info = _system_info()
        grid = Gtk.Grid(row_spacing=8, column_spacing=16)
        grid.set_margin_top(16)
        for i, (label, value) in enumerate(info):
            l = self._field_label(label)
            v = Gtk.Label(label=value)
            v.set_name("value")
            v.set_xalign(0)
            grid.attach(l, 0, i, 1, 1)
            grid.attach(v, 1, i, 1, 1)
        page.pack_start(grid, False, False, 0)
        return page


def _load_ai_config():
    try:
        with open(AI_CONFIG_PATH) as f:
            return json.load(f)
    except (OSError, json.JSONDecodeError):
        return {}


def _system_info():
    u = os.uname()
    mem_total = "unknown"
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemTotal"):
                    mem_total = f"{int(line.split()[1]) / 1024 / 1024:.1f} GiB"
                    break
    except OSError:
        pass
    st = os.statvfs("/")
    disk_total = st.f_frsize * st.f_blocks / 1e9
    disk_free = st.f_frsize * st.f_bavail / 1e9
    return [
        ("Hostname", u.nodename),
        ("Kernel", f"{u.sysname} {u.release}"),
        ("Architecture", u.machine),
        ("Memory", mem_total),
        ("Disk", f"{disk_free:.1f} GB free of {disk_total:.1f} GB"),
    ] + _security_info()


def _security_info():
    """The security chip (TPM 1.2 / 2.0) and Secure Boot, read by nexa-doctor (Rust)."""
    try:
        out = subprocess.run(["nexa-doctor", "security"], capture_output=True, text=True, timeout=3).stdout
        r = json.loads(out)["result"]
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError):
        return []
    return [("Security chip", r["tpm"]["summary"]), ("Secure Boot", r["secure_boot"])]


def main():
    Gtk.init()
    win = SettingsWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    # `nexa-settings bluetooth` (or vpn, power, ...) opens that page.
    wanted = {p.lower().split()[0]: p for p in PAGES}
    for arg in sys.argv[1:]:
        if arg.lower() in wanted:
            win._on_nav(None, wanted[arg.lower()])
    Gtk.main()
