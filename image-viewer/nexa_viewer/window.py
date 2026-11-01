"""The Image Viewer: zoom around the pointer, drag to pan, photos the right
way up, turning (lossless for JPEG photos), the Trash with undo, a strip of
the folder's pictures, details (EXIF), full screen, slideshow, animations,
copy and set as wallpaper. Picture details and turning are done by
nexa-imageinfo, the Trash by nexa-fileops, thumbnails by nexa-thumb."""
import json
import os
import re
import sys
import threading
import time

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("PangoCairo", "1.0")
from gi.repository import Gtk, Gdk, GdkPixbuf, Gio, GLib, Pango, PangoCairo
import cairo

from nexa_common import compositor
from nexa_common.titlebar import attach_titlebar

from . import engine

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 10px; padding: 4px 10px; border: none; box-shadow: none; }
#toolbtn:hover { background-color: @nexa_hover; }
#toolbtn:checked { background-color: @nexa_selected; color: @nexa_selected_text; }
#toolbtn:disabled { color: @nexa_disabled; }
#zoomlabel { color: @nexa_muted; font-family: "JetBrains Mono", monospace; font-size: 9pt; min-width: 48px; }
#filelabel { color: @nexa_muted; font-size: 9pt; }
#info { background-image: none; background-color: @nexa_surface; }
#info .key { color: @nexa_muted; font-size: 8.5pt; }
#info .val { color: @nexa_text; }
#info .head { color: @nexa_text_strong; font-weight: bold; font-size: 12pt; }
#strip { background-image: none; background-color: @nexa_surface; }
#strip button { background-image: none; background-color: transparent; border: 2px solid transparent; border-radius: 10px; padding: 2px; box-shadow: none; }
#strip button:hover { background-color: @nexa_hover; }
#strip button.current { border-color: @nexa_accent; }
#toast { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 10px; padding: 8px 12px; }
#toast button { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; font-weight: bold; border-radius: 10px; border: none; }
"""

ZOOM_STEPS = 1.15
MIN_SCALE, MAX_SCALE = 0.02, 32.0
SLIDE_SECONDS = 4

# EXIF orientation as a 2x2 matrix (as in nexa-imageinfo): to combine a turn
# made on screen only (pictures that can't be turned losslessly).
ORIENT = [(1, 0, 0, 1), (-1, 0, 0, 1), (-1, 0, 0, -1), (1, 0, 0, -1), (0, 1, 1, 0), (0, -1, 1, 0), (0, -1, -1, 0), (0, 1, -1, 0)]
TURNS = {"right": 6, "left": 8, "flip_h": 2, "flip_v": 4}


def compose(o, how):
    a, b = ORIENT[TURNS[how] - 1], ORIENT[o - 1]
    m = (a[0] * b[0] + a[1] * b[2], a[0] * b[1] + a[1] * b[3], a[2] * b[0] + a[3] * b[2], a[2] * b[1] + a[3] * b[3])
    return ORIENT.index(m) + 1


def oriented(pix, o):
    """The stored picture shown the way EXIF orientation `o` says."""
    R = GdkPixbuf.PixbufRotation
    if o == 2:
        return pix.flip(True)
    if o == 3:
        return pix.rotate_simple(R.UPSIDEDOWN)
    if o == 4:
        return pix.flip(False)
    if o == 5:
        return pix.flip(True).rotate_simple(R.COUNTERCLOCKWISE)
    if o == 6:
        return pix.rotate_simple(R.CLOCKWISE)
    if o == 7:
        return pix.flip(True).rotate_simple(R.CLOCKWISE)
    if o == 8:
        return pix.rotate_simple(R.COUNTERCLOCKWISE)
    return pix


def natural_key(name):
    return [int(t) if t.isdigit() else t.lower() for t in re.split(r"(\d+)", name)]


def human_size(n):
    for unit in ("bytes", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n} {unit}" if unit == "bytes" else f"{n:.1f} {unit}"
        n /= 1024


def config_path():
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "nexa-viewer", "settings.json")


def load_settings():
    s = {"sort": "name", "strip": True, "info": False}
    try:
        with open(config_path()) as f:
            s.update({k: v for k, v in json.load(f).items() if k in s})
    except (OSError, ValueError):
        pass
    return s


def save_settings(s):
    p = config_path()
    try:
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p + ".tmp", "w") as f:
            json.dump(s, f)
        os.replace(p + ".tmp", p)
    except OSError:
        pass


class ViewerWindow(Gtk.Window):
    def __init__(self, path=None):
        super().__init__(title="Viewer")
        self.set_icon_name("nexa-viewer")
        attach_titlebar(self, "Viewer")
        self.set_default_size(1000, 700)
        self.settings = load_settings()

        self.path = None
        self.files = []
        self.index = -1
        self.meta = None           # nexa-imageinfo's answer for the current picture
        self.base = None           # the picture as stored
        self.pix = None            # as shown (orientation applied)
        self.anim_iter = None
        self.anim_timer = 0
        self.orientation = 1       # EXIF orientation of the file
        self.view_turn = 1         # extra turn made on screen only
        self.problem = None        # (title, detail) shown instead of a picture
        self.fit = True
        self.scale = 1.0
        self.cx = self.cy = 0.0    # the picture point at the middle of the view
        self.drag = None
        self.slide_timer = 0
        self.load_gen = 0
        self.toast_timer = 0
        self.undo_steps = None
        self.thumbs = {}

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self._build_ui()
        self.connect("key-press-event", self._on_key)
        self.drag_dest_set(Gtk.DestDefaults.ALL, [], Gdk.DragAction.COPY)
        self.drag_dest_add_uri_targets()
        self.connect("drag-data-received", self._on_drop)
        if path:
            self.open_path(os.path.abspath(path))
        self._sync_buttons()

    # ---------- building ----------

    def _btn(self, label, tip, fn, toggle=False):
        b = Gtk.ToggleButton(label=label) if toggle else Gtk.Button(label=label)
        b.set_name("toolbtn")
        b.set_tooltip_text(tip)
        b.connect("toggled" if toggle else "clicked", lambda *_a: fn())
        return b

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.add(root)
        self.toolbar = Gtk.Box(spacing=6)
        self.toolbar.set_name("toolbar")
        self.toolbar.set_border_width(8)
        t = self.toolbar
        t.pack_start(self._btn("Open", "Open a picture (Ctrl+O)", self.on_open), False, False, 0)
        self.prev_btn = self._btn("‹", "Previous (Left)", lambda: self.step(-1))
        self.next_btn = self._btn("›", "Next (Right)", lambda: self.step(1))
        t.pack_start(self.prev_btn, False, False, 0)
        t.pack_start(self.next_btn, False, False, 0)
        t.pack_start(Gtk.Separator(orientation=Gtk.Orientation.VERTICAL), False, False, 4)
        self.zoom_out_btn = self._btn("−", "Zoom out (−, or the wheel)", lambda: self.zoom_by(1 / ZOOM_STEPS))
        self.zoom_label = Gtk.Label()
        self.zoom_label.set_name("zoomlabel")
        self.zoom_in_btn = self._btn("+", "Zoom in (+, or the wheel)", lambda: self.zoom_by(ZOOM_STEPS))
        self.fit_btn = self._btn("Fit", "Fit to the window (0)", self.set_fit)
        self.actual_btn = self._btn("1:1", "Actual size (1)", lambda: self.set_scale(1.0))
        for w in (self.zoom_out_btn, self.zoom_label, self.zoom_in_btn, self.fit_btn, self.actual_btn):
            t.pack_start(w, False, False, 0)
        t.pack_start(Gtk.Separator(orientation=Gtk.Orientation.VERTICAL), False, False, 4)
        self.left_btn = self._btn("⟲", "Turn left (Ctrl+L)", lambda: self.turn("left"))
        self.right_btn = self._btn("⟳", "Turn right (Ctrl+R)", lambda: self.turn("right"))
        self.trash_btn = self._btn("Delete", "Move to the Trash (Delete)", self.trash)
        self.info_btn = self._btn("Details", "Details (I)", self._toggle_info, toggle=True)
        self.more_btn = self._btn("More", "Flip, copy, wallpaper, slideshow, sort", lambda: self._pop(self.more_btn, self._more_menu()))
        for w in (self.left_btn, self.right_btn, self.trash_btn, self.info_btn, self.more_btn):
            t.pack_start(w, False, False, 0)
        self.name_label = Gtk.Label()
        self.name_label.set_name("filelabel")
        self.name_label.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
        self.name_label.set_xalign(1.0)
        t.pack_start(self.name_label, True, True, 4)
        root.pack_start(t, False, False, 0)

        middle = Gtk.Box()
        overlay = Gtk.Overlay()
        self.canvas = Gtk.DrawingArea()
        self.canvas.set_can_focus(True)
        self.canvas.add_events(Gdk.EventMask.SCROLL_MASK | Gdk.EventMask.SMOOTH_SCROLL_MASK | Gdk.EventMask.BUTTON_PRESS_MASK
                               | Gdk.EventMask.BUTTON_RELEASE_MASK | Gdk.EventMask.POINTER_MOTION_MASK)
        self.canvas.connect("draw", self._on_draw)
        self.canvas.connect("scroll-event", self._on_scroll)
        self.canvas.connect("button-press-event", self._on_press)
        self.canvas.connect("button-release-event", self._on_release)
        self.canvas.connect("motion-notify-event", self._on_motion)
        self.canvas.connect("size-allocate", lambda *_a: self.canvas.queue_draw())
        overlay.add(self.canvas)
        self.toast = Gtk.Revealer()
        self.toast.set_halign(Gtk.Align.CENTER)
        self.toast.set_valign(Gtk.Align.END)
        self.toast.set_margin_bottom(16)
        tbox = Gtk.Box(spacing=12)
        tbox.set_name("toast")
        self.toast_label = Gtk.Label()
        self.toast_btn = Gtk.Button(label="Undo")
        self.toast_btn.connect("clicked", lambda *_a: self._toast_action())
        tbox.pack_start(self.toast_label, False, False, 0)
        tbox.pack_start(self.toast_btn, False, False, 0)
        self.toast.add(tbox)
        overlay.add_overlay(self.toast)
        middle.pack_start(overlay, True, True, 0)
        self.info_revealer = Gtk.Revealer()
        self.info_revealer.set_transition_type(Gtk.RevealerTransitionType.SLIDE_LEFT)
        self.info_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.info_box.set_name("info")
        self.info_box.set_border_width(16)
        self.info_box.set_size_request(280, -1)
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.add(self.info_box)
        self.info_revealer.add(scroll)
        middle.pack_start(self.info_revealer, False, False, 0)
        root.pack_start(middle, True, True, 0)

        self.strip_revealer = Gtk.Revealer()
        sscroll = Gtk.ScrolledWindow()
        sscroll.set_policy(Gtk.PolicyType.AUTOMATIC, Gtk.PolicyType.NEVER)
        sscroll.set_name("strip")
        self.strip = Gtk.Box(spacing=4)
        self.strip.set_border_width(6)
        sscroll.add(self.strip)
        sscroll.set_size_request(-1, 96)
        self.strip_scroll = sscroll
        self.strip_revealer.add(sscroll)
        root.pack_start(self.strip_revealer, False, False, 0)
        self.info_btn.set_active(bool(self.settings.get("info")))

    def _pop(self, widget, menu):
        menu.attach_to_widget(widget, None)
        menu.connect("deactivate", lambda m: GLib.idle_add(m.destroy))
        menu.popup_at_widget(widget, Gdk.Gravity.SOUTH_WEST, Gdk.Gravity.NORTH_WEST, None)

    def _more_menu(self):
        m = Gtk.Menu()
        have = self.base is not None

        def item(label, fn, ok=True):
            it = Gtk.MenuItem(label=label)
            it.set_sensitive(ok)
            it.connect("activate", lambda *_a: fn())
            m.append(it)
        item("Flip Left to Right", lambda: self.turn("flip_h"), have)
        item("Flip Top to Bottom", lambda: self.turn("flip_v"), have)
        item("Save a Turned Copy", self.save_turned_copy, have and self.view_turn != 1)
        m.append(Gtk.SeparatorMenuItem())
        item("Copy Picture  (Ctrl+C)", self.copy, have)
        item("Set as Wallpaper", self.set_wallpaper, have)
        m.append(Gtk.SeparatorMenuItem())
        item("Slideshow  (F5)", self.start_slideshow, len(self.files) > 1)
        item("Full Screen  (F11)", self.toggle_fullscreen, have)
        strip = Gtk.CheckMenuItem(label="Strip of Pictures")
        strip.set_active(bool(self.settings.get("strip")))
        strip.connect("toggled", lambda w: self._set("strip", w.get_active()))
        m.append(strip)
        m.append(Gtk.SeparatorMenuItem())
        for key, label in (("name", "Sort by Name"), ("date", "Sort by Date, Newest First")):
            r = Gtk.CheckMenuItem(label=label)
            r.set_draw_as_radio(True)
            r.set_active(self.settings.get("sort") == key)
            r.connect("activate", lambda _w, k=key: self._set("sort", k))
            m.append(r)
        m.show_all()
        return m

    def _set(self, key, value):
        if self.settings.get(key) == value:
            return
        self.settings[key] = value
        save_settings(self.settings)
        if key == "sort" and self.path:
            self._list_folder(self.path)
        self._sync_buttons()

    # ---------- opening ----------

    def on_open(self):
        dialog = Gtk.FileChooserDialog(title="Open a Picture", transient_for=self, action=Gtk.FileChooserAction.OPEN)
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Open", Gtk.ResponseType.OK)
        f = Gtk.FileFilter()
        f.set_name("Pictures")
        for ext in sorted(engine.PICTURES):
            f.add_pattern(f"*{ext}")
            f.add_pattern(f"*{ext.upper()}")
        dialog.add_filter(f)
        folder = os.path.dirname(self.path) if self.path else os.path.join(os.path.expanduser("~"), "Pictures")
        if os.path.isdir(folder):
            dialog.set_current_folder(folder)
        path = dialog.get_filename() if dialog.run() == Gtk.ResponseType.OK else None
        dialog.destroy()
        if path:
            self.open_path(path)

    def open_path(self, path):
        if os.path.isdir(path):
            self._list_folder(os.path.join(path, ""))
            if self.files:
                self.show(0)
            else:
                self._set_problem("No pictures in this folder", path)
            return
        self._list_folder(path)
        self.show(self.files.index(path) if path in self.files else 0)

    def _list_folder(self, path):
        folder = os.path.dirname(path)
        try:
            names = [n for n in os.listdir(folder) if os.path.splitext(n)[1].lower() in engine.PICTURES and not n.startswith(".")]
        except OSError:
            names = []
        files = [os.path.join(folder, n) for n in names]
        if self.settings.get("sort") == "date":
            def mtime(p):
                try:
                    return os.stat(p).st_mtime
                except OSError:
                    return 0
            files.sort(key=mtime, reverse=True)
        else:
            files.sort(key=lambda p: natural_key(os.path.basename(p)))
        if os.path.isfile(path) and path not in files:
            files.insert(0, path)  # opened from elsewhere, or an unusual name
        current = self.files[self.index] if 0 <= self.index < len(self.files) else None
        self.files = files
        self.index = files.index(current) if current in files else self.index
        self._build_strip()

    def show(self, index):
        if not self.files:
            return
        self.index = index % len(self.files)
        path = self.files[self.index]
        self.path = path
        self.load_gen += 1
        gen = self.load_gen
        self._stop_animation()
        self.problem = None
        self.view_turn = 1
        self.name_label.set_text(f"{os.path.basename(path)}   {self.index + 1} of {len(self.files)}")
        self.set_title(f"{os.path.basename(path)} — Viewer")
        bar = self.get_titlebar()
        if isinstance(bar, Gtk.HeaderBar):
            bar.set_title(f"{os.path.basename(path)} — Viewer")
        self._mark_strip()

        def work():
            meta, anim, error = None, None, None
            try:
                meta = engine.info(path)
            except engine.EngineError as e:
                meta = None
                error = str(e)
            try:
                anim = GdkPixbuf.PixbufAnimation.new_from_file(path)
                error = None
            except GLib.Error as e:
                error = error or e.message
            GLib.idle_add(self._loaded, gen, path, meta, anim, error)
        threading.Thread(target=work, daemon=True).start()

    def _loaded(self, gen, path, meta, anim, error):
        if gen != self.load_gen:
            return False
        self.meta = meta
        if anim is None:
            self.base = self.pix = None
            self._set_problem(f"Can't show {os.path.basename(path)}", error or "It may be damaged, or not a picture.")
            return False
        self.orientation = int(((meta or {}).get("exif") or {}).get("orientation") or 1)
        if anim.is_static_image():
            self.base = anim.get_static_image()
        else:
            self.anim_iter = anim.get_iter(None)
            self.base = self.anim_iter.get_pixbuf()
            self._next_frame()
        self._apply_orientation()
        self.fit = True
        self._fill_info()
        self._sync_buttons()
        self.canvas.queue_draw()
        self._preload_thumbs()
        return False

    def _apply_orientation(self):
        if self.base is None:
            return
        # The file's orientation, then a turn made on screen only.
        a = ORIENT[self.view_turn - 1]
        b = ORIENT[self.orientation - 1]
        m = (a[0] * b[0] + a[1] * b[2], a[0] * b[1] + a[1] * b[3], a[2] * b[0] + a[3] * b[2], a[2] * b[1] + a[3] * b[3])
        self.pix = oriented(self.base, ORIENT.index(m) + 1)

    def _set_problem(self, title, detail):
        self.problem = (title, detail)
        self.base = self.pix = None
        self._fill_info()
        self._sync_buttons()
        self.canvas.queue_draw()

    # ---------- animation ----------

    def _next_frame(self):
        if not self.anim_iter:
            return False
        delay = self.anim_iter.get_delay_time()
        if delay < 0:
            return False
        self.anim_timer = GLib.timeout_add(max(delay, 20), self._advance)
        return False

    def _advance(self):
        self.anim_timer = 0
        if not self.anim_iter:
            return False
        self.anim_iter.advance(None)
        self.base = self.anim_iter.get_pixbuf()
        self._apply_orientation()
        self.canvas.queue_draw()
        self._next_frame()
        return False

    def _stop_animation(self):
        if self.anim_timer:
            GLib.source_remove(self.anim_timer)
            self.anim_timer = 0
        self.anim_iter = None

    # ---------- drawing, zoom and pan ----------

    def _fit_scale(self):
        if not self.pix:
            return 1.0
        w, h = self.canvas.get_allocated_width(), self.canvas.get_allocated_height()
        return min((w - 8) / self.pix.get_width(), (h - 8) / self.pix.get_height(), 1.0)

    def _view(self):
        """(scale, x offset, y offset) of the picture in the canvas."""
        w, h = self.canvas.get_allocated_width(), self.canvas.get_allocated_height()
        pw, ph = self.pix.get_width(), self.pix.get_height()
        s = self._fit_scale() if self.fit else self.scale
        if self.fit:
            self.cx, self.cy = pw / 2, ph / 2
        # Smaller than the view: centred; bigger: kept from drifting off.
        for axis, (size, view) in enumerate(((pw * s, w), (ph * s, h))):
            c = self.cx if axis == 0 else self.cy
            full = pw if axis == 0 else ph
            if size <= view:
                c = full / 2
            else:
                half = view / 2 / s
                c = min(max(c, half), full - half)
            if axis == 0:
                self.cx = c
            else:
                self.cy = c
        return s, w / 2 - self.cx * s, h / 2 - self.cy * s

    def _on_draw(self, widget, cr):
        ctx = widget.get_style_context()
        w, h = widget.get_allocated_width(), widget.get_allocated_height()
        bg = ctx.lookup_color("nexa_bg_deep")
        cr.set_source_rgba(*(bg[1] if bg[0] else Gdk.RGBA(0.03, 0.05, 0.12, 1)))
        cr.paint()
        if self.pix is None:
            title, detail = self.problem or ("No picture open", "Press Open (Ctrl+O), or drop a picture here.")
            text = ctx.lookup_color("nexa_text")
            muted = ctx.lookup_color("nexa_muted")
            for i, (t, colour, size) in enumerate(((title, text, 15), (detail, muted, 10))):
                layout = widget.create_pango_layout(t)
                layout.set_font_description(Pango.FontDescription(f"Inter {size}"))
                layout.set_width(int(min(w - 48, 640) * Pango.SCALE))
                layout.set_alignment(Pango.Alignment.CENTER)
                lw, lh = layout.get_pixel_size()
                cr.move_to((w - min(w - 48, 640)) / 2, h / 2 - 28 + i * 34)
                c = colour[1] if colour[0] else Gdk.RGBA(0.85, 0.88, 0.96, 1)
                cr.set_source_rgba(c.red, c.green, c.blue, c.alpha)
                PangoCairo.show_layout(cr, layout)
            self.zoom_label.set_text("")
            return
        s, ox, oy = self._view()
        cr.save()
        cr.translate(ox, oy)
        cr.scale(s, s)
        Gdk.cairo_set_source_pixbuf(cr, self.pix, 0, 0)
        cr.get_source().set_filter(cairo.FILTER_NEAREST if s >= 3 else cairo.FILTER_GOOD)
        cr.paint()
        cr.restore()
        self.zoom_label.set_text(f"{round(s * 100)}%")

    def set_fit(self):
        self.fit = True
        self.canvas.queue_draw()

    def set_scale(self, s, at=None):
        if not self.pix:
            return
        old_s, ox, oy = self._view()
        s = max(MIN_SCALE, min(MAX_SCALE, s))
        w, h = self.canvas.get_allocated_width(), self.canvas.get_allocated_height()
        px, py = at if at else (w / 2, h / 2)
        # The picture point under the pointer stays under it.
        ix, iy = (px - ox) / old_s, (py - oy) / old_s
        self.fit = False
        self.scale = s
        self.cx, self.cy = ix - (px - w / 2) / s, iy - (py - h / 2) / s
        self.canvas.queue_draw()

    def zoom_by(self, factor, at=None):
        if self.pix:
            s = self._view()[0]
            self.set_scale(s * factor, at)

    def _on_scroll(self, _w, ev):
        if ev.direction == Gdk.ScrollDirection.SMOOTH:
            dy = ev.get_scroll_deltas()[2]
            if dy == 0:
                return True
            factor = ZOOM_STEPS ** (-dy)
        else:
            factor = ZOOM_STEPS if ev.direction == Gdk.ScrollDirection.UP else 1 / ZOOM_STEPS
        self.zoom_by(factor, (ev.x, ev.y))
        return True

    def _on_press(self, _w, ev):
        self.canvas.grab_focus()
        if ev.type == Gdk.EventType._2BUTTON_PRESS and ev.button == 1 and self.pix:
            if self.fit or abs(self._view()[0] - 1.0) > 0.01:
                self.set_scale(1.0, (ev.x, ev.y))
            else:
                self.set_fit()
            return True
        if ev.button == 1 and self.pix:
            self.drag = (ev.x, ev.y, self.cx, self.cy)
            self.canvas.get_window().set_cursor(Gdk.Cursor.new_from_name(self.canvas.get_display(), "grabbing"))
        return True

    def _on_release(self, *_a):
        self.drag = None
        if self.canvas.get_window():
            self.canvas.get_window().set_cursor(None)

    def _on_motion(self, _w, ev):
        if self.drag and self.pix:
            x0, y0, cx, cy = self.drag
            s = self._view()[0]
            if self.fit:
                self.fit, self.scale = False, s
            self.cx, self.cy = cx - (ev.x - x0) / s, cy - (ev.y - y0) / s
            self.canvas.queue_draw()

    # ---------- going through the folder ----------

    def step(self, n):
        if len(self.files) > 1:
            self.show(self.index + n)

    def _sync_buttons(self):
        have = self.pix is not None
        many = len(self.files) > 1
        for b in (self.prev_btn, self.next_btn):
            b.set_sensitive(many)
        for b in (self.zoom_out_btn, self.zoom_in_btn, self.fit_btn, self.actual_btn, self.left_btn, self.right_btn):
            b.set_sensitive(have)
        self.trash_btn.set_sensitive(bool(self.path) and os.path.exists(self.path or ""))
        self.strip_revealer.set_reveal_child(bool(self.settings.get("strip")) and many and not self._presenting())
        self.info_revealer.set_reveal_child(self.info_btn.get_active() and not self._presenting())

    # ---------- the strip ----------

    def _build_strip(self):
        for c in self.strip.get_children():
            c.destroy()
        self.strip_buttons = {}
        for i, p in enumerate(self.files[:2000]):
            b = Gtk.Button()
            b.set_tooltip_text(os.path.basename(p))
            img = Gtk.Image()
            img.set_size_request(72, 72)
            if p in self.thumbs:
                self._set_thumb(img, self.thumbs[p])
            else:
                img.set_from_icon_name("image-x-generic", Gtk.IconSize.DIALOG)
            b.add(img)
            b.connect("clicked", lambda _b, i=i: self.show(i))
            self.strip.pack_start(b, False, False, 0)
            self.strip_buttons[p] = (b, img)
        self.strip.show_all()
        self._sync_buttons()

    def _set_thumb(self, img, png):
        try:
            pix = GdkPixbuf.Pixbuf.new_from_file_at_scale(png, 72, 72, True)
            img.set_from_pixbuf(pix)
        except GLib.Error:
            pass

    def _mark_strip(self):
        for p, (b, _img) in getattr(self, "strip_buttons", {}).items():
            ctx = b.get_style_context()
            if p == self.path:
                ctx.add_class("current")
                GLib.idle_add(self._scroll_strip_to, b)
            else:
                ctx.remove_class("current")

    def _scroll_strip_to(self, b):
        alloc = b.get_allocation()
        adj = self.strip_scroll.get_hadjustment()
        if alloc.x >= 0:
            adj.set_value(max(0, min(alloc.x - (adj.get_page_size() - alloc.width) / 2, adj.get_upper() - adj.get_page_size())))
        return False

    def _preload_thumbs(self):
        if not self.settings.get("strip") or len(self.files) < 2:
            return
        lo = max(0, self.index - 30)
        want = [p for p in self.files[lo:self.index + 30] if p not in self.thumbs]
        if not want:
            return

        def work():
            found = engine.thumbnails(want)
            GLib.idle_add(self._got_thumbs, found)
        threading.Thread(target=work, daemon=True).start()

    def _got_thumbs(self, found):
        self.thumbs.update(found)
        for p, png in found.items():
            if p in getattr(self, "strip_buttons", {}):
                self._set_thumb(self.strip_buttons[p][1], png)
        return False

    # ---------- details ----------

    def _toggle_info(self):
        self._set("info", self.info_btn.get_active())
        self._sync_buttons()

    def _fill_info(self):
        for c in self.info_box.get_children():
            c.destroy()

        def head(text):
            l = Gtk.Label(label=text)
            l.get_style_context().add_class("head")
            l.set_xalign(0)
            l.set_margin_top(8)
            self.info_box.pack_start(l, False, False, 0)

        def row(key, value):
            if value in (None, ""):
                return
            k = Gtk.Label(label=key)
            k.get_style_context().add_class("key")
            k.set_xalign(0)
            v = Gtk.Label(label=str(value))
            v.get_style_context().add_class("val")
            v.set_xalign(0)
            v.set_line_wrap(True)
            v.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
            v.set_selectable(True)
            self.info_box.pack_start(k, False, False, 0)
            self.info_box.pack_start(v, False, False, 4)

        if not self.path:
            head("Details")
            row("", "Open a picture to see its details.")
            self.info_box.show_all()
            return
        head("File")
        row("Name", os.path.basename(self.path))
        row("Folder", os.path.dirname(self.path).replace(os.path.expanduser("~"), "~", 1))
        m = self.meta or {}
        try:
            st = os.stat(self.path)
            row("Size", human_size(st.st_size))
            row("Modified", time.strftime("%d %B %Y, %H:%M", time.localtime(st.st_mtime)))
        except OSError:
            pass
        if self.pix is not None:
            w, h = self.pix.get_width(), self.pix.get_height()
            row("Dimensions", f"{w} × {h} ({w * h / 1e6:.1f} megapixels)")
        row("Type", m.get("format") + (", moving" if m.get("animated") else "") if m.get("format") else None)
        x = m.get("exif") or {}
        camera = " ".join(v for v in (x.get("make"), x.get("model")) if v)
        if x and (camera or x.get("date") or x.get("exposure")):
            head("Photo")
            if x.get("model") and x.get("make") and x["model"].startswith(x["make"]):
                camera = x["model"]
            row("Camera", camera)
            row("Lens", x.get("lens"))
            date = x.get("date")
            if date and re.match(r"\d{4}:\d\d:\d\d \d\d:\d\d", date):
                try:
                    date = time.strftime("%d %B %Y, %H:%M", time.strptime(date[:16], "%Y:%m:%d %H:%M"))
                except ValueError:
                    pass
            row("Taken", date)
            settings = ", ".join(v for v in (x.get("exposure"), x.get("fnumber"), f"ISO {x['iso']}" if x.get("iso") else None, x.get("focal")) if v)
            row("Settings", settings)
            row("Software", x.get("software"))
            if x.get("gps"):
                row("Location", "Saved in the photo. Anyone you send it to can see where it was taken.")
        self.info_box.show_all()

    # ---------- changing the picture ----------

    def _expect(self):
        m = self.meta or {}
        return {"mtime_ns": m.get("mtime_ns"), "size": m.get("bytes")} if m.get("mtime_ns") else None

    def turn(self, how):
        if self.base is None:
            return
        fmt = (self.meta or {}).get("format")
        if fmt == "JPEG" and self.view_turn == 1:
            try:
                r = engine.turn(self.path, how, self._expect())
            except engine.EngineError as e:
                if e.code != "no_lossless":
                    self.notify(str(e))
                    return
            else:
                self.orientation = r["orientation"]
                self.meta.update({"mtime_ns": r["mtime_ns"], "bytes": r["size"]})
                self.meta.setdefault("exif", {})
                if self.meta["exif"] is not None:
                    self.meta["exif"]["orientation"] = r["orientation"]
                back = {"right": "left", "left": "right"}.get(how, how)
                self._apply_orientation()
                self.fit = True
                self.canvas.queue_draw()
                self.thumbs.pop(self.path, None)
                self.notify("Turned and saved, without losing any quality.", "Undo", lambda: self.turn(back))
                return
        self.view_turn = compose(self.view_turn, how)
        self._apply_orientation()
        self.fit = True
        self.canvas.queue_draw()
        if self.view_turn == 1:
            self.notify("Back as it was.")
        else:
            self.notify("Turned on screen only: this picture can't be turned without saving it again.", "Save a Copy", self.save_turned_copy)

    def save_turned_copy(self):
        if self.pix is None or not self.path:
            return
        stem, ext = os.path.splitext(self.path)
        kind = "jpeg" if ext.lower() in (".jpg", ".jpeg", ".jpe") else "png"
        ext = ".jpg" if kind == "jpeg" else ".png"
        n, target = 1, f"{stem} (turned){ext}"
        while os.path.exists(target):
            n += 1
            target = f"{stem} (turned {n}){ext}"
        try:
            self.pix.savev(target, kind, ["quality"] if kind == "jpeg" else [], ["95"] if kind == "jpeg" else [])
        except GLib.Error as e:
            self.notify(f"The copy wasn't saved: {e.message}")
            return
        self._list_folder(self.path)
        self.notify(f"Saved as {os.path.basename(target)}.", "Show It", lambda: self.show(self.files.index(target)) if target in self.files else None)

    def trash(self):
        if not self.path:
            return
        path, index = self.path, self.index
        try:
            steps = engine.trash(path)
        except engine.EngineError as e:
            self.notify(str(e))
            return
        self.files = [p for p in self.files if p != path]
        self.thumbs.pop(path, None)
        self._build_strip()
        name = os.path.basename(path)
        if self.files:
            self.show(min(index, len(self.files) - 1))
        else:
            self.path = None
            self.index = -1
            self.meta = None
            self._stop_animation()
            self._set_problem("No pictures left here", "Open another one (Ctrl+O), or press Undo to bring this one back.")
        self.notify(f"{name} moved to the Trash.", "Undo", lambda: self._undo_trash(steps, path))

    def _undo_trash(self, steps, path):
        try:
            engine.undo(steps)
        except engine.EngineError as e:
            self.notify(str(e))
            return
        self._list_folder(path)
        self.show(self.files.index(path) if path in self.files else 0)

    def copy(self):
        if self.pix is not None:
            Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_image(self.pix)
            self.notify("Copied. Paste it into another app.")

    def set_wallpaper(self):
        if not self.path:
            return
        try:
            compositor.apply_wallpaper(self.path)
        except OSError as e:
            self.notify(f"The wallpaper wasn't changed: {e}")
            return
        self.notify("This is your wallpaper now.")

    # ---------- full screen and slideshow ----------

    def _presenting(self):
        return getattr(self, "_fullscreen", False)

    def toggle_fullscreen(self, on=None):
        on = not self._presenting() if on is None else on
        self._fullscreen = on
        (self.fullscreen if on else self.unfullscreen)()
        self.toolbar.set_visible(not on)
        if self.get_titlebar():
            self.get_titlebar().set_visible(not on)
        if not on:
            self.stop_slideshow()
        self.fit = True
        self._sync_buttons()

    def start_slideshow(self):
        if len(self.files) < 2:
            return
        self.toggle_fullscreen(True)
        self.slide_timer = GLib.timeout_add_seconds(SLIDE_SECONDS, self._slide)

    def _slide(self):
        self.step(1)
        return True

    def stop_slideshow(self):
        if self.slide_timer:
            GLib.source_remove(self.slide_timer)
            self.slide_timer = 0

    # ---------- messages ----------

    def notify(self, text, action=None, fn=None):
        self.toast_label.set_text(text)
        self.toast_btn.set_label(action or "")
        self.toast_btn.set_visible(bool(action))
        self._toast_fn = fn
        self.toast.show_all()
        self.toast_btn.set_visible(bool(action))
        self.toast.set_reveal_child(True)
        if self.toast_timer:
            GLib.source_remove(self.toast_timer)
        self.toast_timer = GLib.timeout_add_seconds(6, self._hide_toast)

    def _hide_toast(self):
        self.toast_timer = 0
        self.toast.set_reveal_child(False)
        return False

    def _toast_action(self):
        fn = getattr(self, "_toast_fn", None)
        self._hide_toast()
        if fn:
            fn()

    # ---------- keys and drops ----------

    def _on_key(self, _w, ev):
        ctrl = ev.state & Gdk.ModifierType.CONTROL_MASK
        name = Gdk.keyval_name(ev.keyval) or ""
        actions = {
            "Left": lambda: self.step(-1), "Right": lambda: self.step(1), "Page_Up": lambda: self.step(-1),
            "Page_Down": lambda: self.step(1), "BackSpace": lambda: self.step(-1), "space": lambda: self.step(1),
            "Home": lambda: self.files and self.show(0), "End": lambda: self.files and self.show(len(self.files) - 1),
            "plus": lambda: self.zoom_by(ZOOM_STEPS), "equal": lambda: self.zoom_by(ZOOM_STEPS), "KP_Add": lambda: self.zoom_by(ZOOM_STEPS),
            "minus": lambda: self.zoom_by(1 / ZOOM_STEPS), "KP_Subtract": lambda: self.zoom_by(1 / ZOOM_STEPS),
            "0": self.set_fit, "1": lambda: self.set_scale(1.0), "Delete": self.trash, "F11": self.toggle_fullscreen,
            "F5": self.start_slideshow, "i": lambda: self.info_btn.set_active(not self.info_btn.get_active()),
        }
        ctrl_actions = {"o": self.on_open, "c": self.copy, "r": lambda: self.turn("right"), "l": lambda: self.turn("left"),
                        "q": self.close, "w": self.close}
        if name == "Escape":
            if self.slide_timer or self._presenting():
                self.toggle_fullscreen(False)
            else:
                self.close()
            return True
        if ctrl and name.lower() in ctrl_actions:
            ctrl_actions[name.lower()]()
            return True
        if not ctrl and name in actions:
            if self.slide_timer and name not in ("F11", "F5"):
                self.stop_slideshow()
            actions[name]()
            return True
        return False

    def _on_drop(self, _w, ctx, _x, _y, data, _info, time_):
        for uri in data.get_uris() or []:
            path = Gio.File.new_for_uri(uri).get_path()
            if path:
                self.open_path(path)
                break
        Gtk.drag_finish(ctx, True, False, time_)


def main():
    Gtk.init()
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    win = ViewerWindow(args[0] if args else None)
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    win.toast.set_reveal_child(False)
    win._sync_buttons()
    Gtk.main()
