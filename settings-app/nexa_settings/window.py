import json
import os
import shutil
import subprocess

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GdkPixbuf, GLib

from . import privileged
from nexa_common import sysconf, themes
from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#sidebar { background-image: none; background-color: #232323; }
#navbtn { background-image: none; background-color: transparent; color: #b0b0b0; border-radius: 6px; padding: 8px 14px; }
#navbtn:hover { background-image: none; background-color: #2f2f2f; }
#navbtn.active { background-image: none; background-color: #2c3a52; color: #dbe4f5; }
#pagetitle { color: #ffffff; font-size: 16pt; font-weight: bold; }
#fieldlabel { color: #9a9a9a; font-size: 9pt; }
#value { color: #d4d4d4; }
#entry { background-image: none; background-color: #2a2a2a; color: #ffffff; border-radius: 6px; padding: 6px 10px; border: 1px solid #3c3c3c; }
#actionbtn { background-image: none; background-color: #7aa2f7; color: #1e1e1e; border-radius: 6px; padding: 6px 14px; font-weight: bold; }
#actionbtn:hover { background-image: none; background-color: #92b6ff; }
#toggle { border-radius: 12px; padding: 4px 12px; font-weight: bold; font-size: 9pt; }
#toggle.on { background-image: none; background-color: #7aa2f7; color: #1e1e1e; }
#toggle.off { background-image: none; background-color: #3a3a3a; color: #9a9a9a; }
#status { color: #a6e3a1; font-size: 9pt; }
#error { color: #f7768e; font-size: 9pt; }
#wordmark { color: #ffffff; font-size: 22pt; font-weight: bold; }
#themecard { background-image: none; background-color: #262626; border-radius: 14px; padding: 8px; border: 2px solid transparent; }
#themecard:hover { background-color: #2f2f2f; }
#themecard.chosen { border-color: #7aa2f7; }
"""

WALLPAPER_PATH = "/usr/share/backgrounds/nexaos/wallpaper.png"
WALLPAPER_GALLERY = "/usr/share/backgrounds/nexaos/gallery"
AI_CONFIG_PATH = "/etc/nexa-ai/config.json"
PAGES = ("Personalization", "System", "Time & Keyboard", "Power", "Assistant", "About")


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
        root.pack_start(sidebar, False, False, 0)

        self.stack = Gtk.Stack()
        self.stack.set_transition_type(Gtk.StackTransitionType.CROSSFADE)
        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        content.set_border_width(24)
        content.pack_start(self.stack, True, True, 0)
        root.pack_start(content, True, True, 0)

        self.stack.add_named(self._scrolled(self._build_appearance_page()), "Personalization")
        self.stack.add_named(self._scrolled(self._build_system_page()), "System")
        self.stack.add_named(self._scrolled(self._build_time_page()), "Time & Keyboard")
        self.stack.add_named(self._scrolled(self._build_power_page()), "Power")
        self.stack.add_named(self._scrolled(self._build_assistant_page()), "Assistant")
        self.stack.add_named(self._scrolled(self._build_about_page()), "About")

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )
        self._on_nav(None, "Personalization")

    def _on_nav(self, _btn, name):
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
            subprocess.run(["swaymsg", "output", "*", "bg", WALLPAPER_PATH, "fill"], check=True)
        except (OSError, subprocess.CalledProcessError) as e:
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
        return page

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
            subprocess.run(["swaymsg", "input", "type:keyboard", "xkb_layout", code], check=True,
                           capture_output=True)
            sysconf.write_keyboard_layout(code)
        except (OSError, subprocess.CalledProcessError) as e:
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

        self.power_status = Gtk.Label(label="")
        self.power_status.set_xalign(0)
        page.pack_start(self.power_status, False, False, 0)
        return page

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

        page.pack_start(self._field_label("Claude API key"), False, False, 0)
        self.api_key_entry = Gtk.Entry()
        self.api_key_entry.set_name("entry")
        self.api_key_entry.set_visibility(False)
        self.api_key_entry.set_text(cfg.get("api_key", ""))
        page.pack_start(self.api_key_entry, False, False, 0)

        page.pack_start(self._field_label("Model"), False, False, 0)
        self.model_entry = Gtk.Entry()
        self.model_entry.set_name("entry")
        self.model_entry.set_text(cfg.get("model", "claude-sonnet-5"))
        page.pack_start(self.model_entry, False, False, 0)

        self.allow_control_toggle = self._toggle_row(
            page, "Allow control actions", cfg.get("allow_control", False),
        )
        self.require_confirmation_toggle = self._toggle_row(
            page, "Require confirmation for control actions", cfg.get("require_confirmation", True),
        )

        save_btn = Gtk.Button(label="Save")
        save_btn.set_name("actionbtn")
        save_btn.connect("clicked", self._on_save_assistant)
        page.pack_start(save_btn, False, False, 0)

        self.assistant_status = Gtk.Label(label="")
        self.assistant_status.set_xalign(0)
        page.pack_start(self.assistant_status, False, False, 0)
        return page

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
            "model": self.model_entry.get_text().strip() or "claude-sonnet-5",
            "allow_control": self.allow_control_toggle.get_active(),
            "require_confirmation": self.require_confirmation_toggle.get_active(),
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
    ]


def main():
    Gtk.init()
    win = SettingsWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
