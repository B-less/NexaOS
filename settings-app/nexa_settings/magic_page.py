"""Settings > Magic Layer: the Living Background Studio (nexa-living, C)
and the Magic Layer features (TODOnexa_magic.md).

Every control writes ~/.config/nexa/magic.json through nexa_common.magic;
nexa-living picks the change up at once, and so do the apps' windows
(their glass). A feature only gets a switch here once it works.
"""
import os
import subprocess
import tempfile
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import GdkPixbuf, GLib, Gtk

from nexa_common import magic

SLIDERS = (
    ("speed", "Speed", 0.25, 2.0, 0.05),
    ("brightness", "Brightness", 0.2, 1.5, 0.05),
    ("density", "How much is drawn", 0.25, 2.0, 0.05),
    ("reaction", "Reaction to the pointer and clicks", 0.0, 1.0, 0.05),
    ("veil", "Dimming behind windows (keeps text easy to read)", 0.0, 0.85, 0.05),
    ("window_alpha", "How solid app windows are", 0.55, 0.95, 0.01),
)
FPS_CAPS = (("0", "No cap (the quality decides)"), ("30", "30 frames a second"), ("20", "20"), ("12", "12"))
BATTERY = (("lower", "Lower quality on battery"), ("same", "Same as when plugged in"))

# What each feature does, in a line; a switch appears once it's built.
FEATURE_TEXT = {
    "peek": "Peek: preview a file without opening its app",
    "gravity": "Gravity: windows placed side by side move together",
    "rewind": "Rewind: undo recent settings changes from a timeline",
    "hold": "Hold: change a setting for a while, then change it back",
    "thread": "Thread: go back to recent work sessions (opt-in)",
    "orbit": "Orbit: a ring of actions for what's under the pointer",
    "filetrail": "FileTrail: where did I move that file?",
    "passport": "Passport: a card for each USB drive you've used before",
    "postbox": "Postbox: transfers wait for the network and resume",
    "why": "Why: explanations from the computer's own evidence",
    "ready": "Ready: a check before shutting down",
    "lifeboat": "Lifeboat: a recovery screen when the desktop hangs",
    "tomorrow": "Tomorrow: offer to continue yesterday's work (opt-in)",
    "appetite": "Appetite: how much memory an app usually needs (opt-in)",
    "quiet": "Quiet: focus profiles (fewer notifications, calmer visuals)",
    "clipboard": "Clipboard Sense: actions for what you copied (opt-in)",
    "surprise": "Surprise: an occasional useful tip (opt-in)",
    "gestures": "GestureKeys: mouse gestures while holding a key",
}
BUILT = {"why"}  # features with real behaviour behind them (grows as they land)


def _preview_png(style, w=320, h=180):
    """One frame of a style, drawn by the renderer's own code."""
    out = os.path.join(tempfile.gettempdir(), f"nexa-living-preview-{os.getuid()}-{style}.png")
    try:
        subprocess.run(["nexa-living-preview", style, str(w), str(h), "8", out, "1"],
                       check=True, timeout=10, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return out
    except (OSError, subprocess.SubprocessError):
        return None


class MagicPage:
    def __init__(self, win):
        self.win = win
        self.page = win._page("Magic Layer")
        self._loading = True
        self._save_timer = None
        self._pending = {}
        cfg = magic.load()
        lv = cfg["living"]

        intro = win._field_label("Living backgrounds: each Nexa app gets a moving background behind its glass, and the "
                                 "desktop one around the windows. One small program draws them all; it slows down or "
                                 "stops when the computer is busy, on battery, or when you'd rather have calm.")
        intro.set_line_wrap(True)
        self.page.pack_start(intro, False, False, 0)

        # ---- the studio
        self.enabled = self._switch_row("Living backgrounds", lv["enabled"], lambda on: self._set(enabled=on))
        self.quality = self._combo_row("Quality", magic.QUALITIES, lv["quality"], lambda v: self._set(quality=v))
        self.desktop = self._combo_row("Desktop", [("none", "Wallpaper only")] + magic.STYLES, lv["desktop"],
                                       lambda v: (self._set(desktop=v), self._show_preview(v)))
        self.preview = Gtk.Image()
        self.preview.set_halign(Gtk.Align.START)
        self.page.pack_start(self.preview, False, False, 0)
        self.sliders = {}
        for key, label, lo, hi, step in SLIDERS:
            self.sliders[key] = self._slider_row(label, key, lo, hi, step, float(lv[key]))
        self.fps = self._combo_row("Frame-rate cap", FPS_CAPS, str(lv["fps_cap"]), lambda v: self._set(fps_cap=int(v)))
        self.battery = self._combo_row("On battery", BATTERY, lv["battery"], lambda v: self._set(battery=v))
        self.reduced = self._switch_row("Reduced motion (a still picture)", lv["reduced_motion"],
                                        lambda on: self._set(reduced_motion=on))
        self.measured = win._field_label("")
        self.measured.set_line_wrap(True)
        self.page.pack_start(self.measured, False, False, 0)
        reset = Gtk.Button(label="Restore the defaults")
        reset.set_halign(Gtk.Align.START)
        reset.connect("clicked", self._on_reset)
        self.page.pack_start(reset, False, False, 0)

        # ---- per app
        self.page.pack_start(win._field_label("Each app's background"), False, False, 0)
        self.app_combos, self.app_checks = {}, {}
        grid = Gtk.Grid(column_spacing=12, row_spacing=6)
        for row, (app, title, _default) in enumerate(magic.APPS):
            check = Gtk.CheckButton(label=title)
            grid.attach(check, 0, row, 1, 1)
            combo = Gtk.ComboBoxText()
            for key, name in magic.STYLES + [("off", "Off")]:
                combo.append(key, name)
            combo.set_active_id(lv["apps"].get(app, "off"))
            combo.connect("changed", self._on_app_style, app)
            combo.get_accessible().set_name(f"{title} background")
            grid.attach(combo, 1, row, 1, 1)
            self.app_combos[app], self.app_checks[app] = combo, check
        self.page.pack_start(grid, False, False, 0)
        many = Gtk.Box(spacing=8)
        many.pack_start(Gtk.Label(label="Give the ticked apps:"), False, False, 0)
        self.many_style = Gtk.ComboBoxText()
        for key, name in magic.STYLES + [("off", "Off")]:
            self.many_style.append(key, name)
        self.many_style.set_active(0)
        many.pack_start(self.many_style, False, False, 0)
        apply_btn = Gtk.Button(label="Apply")
        apply_btn.connect("clicked", self._on_apply_many)
        many.pack_start(apply_btn, False, False, 0)
        self.page.pack_start(many, False, False, 0)
        note = win._field_label("Other apps keep the theme's look: their insides can't be changed from outside.")
        note.set_line_wrap(True)
        self.page.pack_start(note, False, False, 0)

        # ---- the Magic Layer features
        self.page.pack_start(win._field_label("Magic Layer"), False, False, 0)
        self.feature_switches = {}
        coming = []
        for name in magic.FEATURES:
            if name in BUILT:
                self.feature_switches[name] = self._switch_row(
                    FEATURE_TEXT[name], cfg["features"][name]["enabled"],
                    lambda on, n=name: magic.set_feature(n, enabled=on))
            else:
                coming.append(FEATURE_TEXT[name].split(":")[0])
        if coming:
            more = win._field_label("Being built (not switchable yet): " + ", ".join(coming) + ".")
            more.set_line_wrap(True)
            self.page.pack_start(more, False, False, 0)

        self.status = win._field_label("")
        self.page.pack_start(self.status, False, False, 0)
        self._loading = False
        self._show_preview(lv["desktop"])
        GLib.timeout_add_seconds(2, self._refresh_measured)
        self._refresh_measured()

    # ---- rows
    def _row(self, label, widget):
        box = Gtk.Box(spacing=12)
        text = self.win._field_label(label)
        text.set_line_wrap(True)
        box.pack_start(text, True, True, 0)
        widget.set_valign(Gtk.Align.CENTER)
        box.pack_end(widget, False, False, 0)
        self.page.pack_start(box, False, False, 0)
        return widget

    def _switch_row(self, label, active, fn):
        sw = Gtk.Switch(active=bool(active))
        sw.get_accessible().set_name(label)
        sw.connect("notify::active", lambda s, _p: None if self._loading else fn(s.get_active()))
        return self._row(label, sw)

    def _combo_row(self, label, options, active, fn):
        combo = Gtk.ComboBoxText()
        for key, name in options:
            combo.append(key, name)
        combo.set_active_id(active)
        combo.get_accessible().set_name(label)
        combo.connect("changed", lambda c: None if self._loading or c.get_active_id() is None else fn(c.get_active_id()))
        return self._row(label, combo)

    def _slider_row(self, label, key, lo, hi, step, value):
        scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, lo, hi, step)
        scale.set_value(value)
        scale.set_size_request(220, -1)
        scale.set_digits(2)
        scale.get_accessible().set_name(label)
        scale.connect("value-changed", lambda s: None if self._loading else self._set_later(**{key: round(s.get_value(), 2)}))
        return self._row(label, scale)

    # ---- saving
    def _set(self, **changes):
        try:
            magic.set_living(**changes)
            self.status.set_text("")
        except (OSError, ValueError) as e:
            self.status.set_text(f"Couldn't save that: {e}")

    def _set_later(self, **changes):
        """Sliders save once they stop moving, not on every step."""
        self._pending.update(changes)
        if self._save_timer:
            GLib.source_remove(self._save_timer)

        def save():
            self._save_timer = None
            pending, self._pending = self._pending, {}
            self._set(**pending)
            return False
        self._save_timer = GLib.timeout_add(300, save)

    def _on_app_style(self, combo, app):
        if self._loading or combo.get_active_id() is None:
            return
        try:
            magic.set_app_style(app, combo.get_active_id())
        except (OSError, ValueError) as e:
            self.status.set_text(f"Couldn't save that: {e}")

    def _on_apply_many(self, _b):
        apps = [a for a, c in self.app_checks.items() if c.get_active()]
        if not apps:
            self.status.set_text("Tick the apps first.")
            return
        style = self.many_style.get_active_id()
        magic.set_app_style(apps, style)
        self._loading = True
        for a in apps:
            self.app_combos[a].set_active_id(style)
            self.app_checks[a].set_active(False)
        self._loading = False
        self.status.set_text(f"{len(apps)} app(s) set to {dict(magic.STYLES + [('off', 'Off')])[style]}.")

    def _on_reset(self, _b):
        magic.reset("living")
        lv = magic.load()["living"]
        self._loading = True
        self.enabled.set_active(lv["enabled"])
        self.quality.set_active_id(lv["quality"])
        self.desktop.set_active_id(lv["desktop"])
        for key, scale in self.sliders.items():
            scale.set_value(float(lv[key]))
        self.fps.set_active_id(str(lv["fps_cap"]))
        self.battery.set_active_id(lv["battery"])
        self.reduced.set_active(lv["reduced_motion"])
        for app, combo in self.app_combos.items():
            combo.set_active_id(lv["apps"].get(app, "off"))
        self._loading = False
        self.status.set_text("Back to the defaults.")

    # ---- what it shows
    def _show_preview(self, style):
        if style not in magic.STYLE_NAMES:
            self.preview.clear()
            return

        def work():
            path = _preview_png(style)
            GLib.idle_add(lambda: self._set_preview(path) and False)
        threading.Thread(target=work, daemon=True).start()

    def _set_preview(self, path):
        if path:
            try:
                self.preview.set_from_pixbuf(GdkPixbuf.Pixbuf.new_from_file(path))
                return
            except GLib.Error:
                pass
        self.preview.clear()

    def _refresh_measured(self):
        if not self.page.get_mapped():
            return True
        s = magic.stats()
        if not s:
            self.measured.set_text("Measured: the renderer isn't running (it starts with the desktop).")
        elif not s.get("enabled"):
            self.measured.set_text("Measured: off, nothing drawn.")
        else:
            why = []
            if s.get("on_battery"):
                why.append("on battery")
            if s.get("brain_low"):
                why.append("MicroBrain asked for less")
            if s.get("low_memory"):
                why.append("2 GB profile")
            self.measured.set_text(
                f"Measured just now: {s['quality']} quality, {s['fps']:.0f} frames a second, "
                f"{s['frame_ms']:.1f} ms a frame, {s['rss_kb'] / 1024:.0f} MB of memory, "
                f"{s['styled']} app window(s) with a background" + (f" ({', '.join(why)})" if why else "") + ".")
        return True
