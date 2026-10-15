"""Hexa widgets for the assistant's actions, shared by the assistant window
and Settings > Assistant:

  ActionCard        a change the assistant wants to make: what, where, a
                    preview; Allow / Not now; afterwards the result and Undo
  ReadChip          a one-line note for something it only looked at
  ActivityTimeline  every file action, grouped by day, each with Undo

The words come from actiontext.py; the work (and undo) from nexa-tools via
hands.py, always off the GTK thread.
"""
import threading
import time

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("PangoCairo", "1.0")
from gi.repository import Gdk, GLib, Gtk, Pango, PangoCairo

from . import actiontext, design, hands

CSS = f"""
.nx-card {{ background-image: none; background-color: @nexa_surface; border: 1px solid @nexa_border;
           border-radius: {design.RADIUS["large"]}px; padding: 12px 14px; }}
.nx-card.danger {{ border-color: alpha(@nexa_crit, 0.55); }}
.nx-card.done {{ border-color: alpha(@nexa_ok, 0.45); }}
.nx-card.undone, .nx-card.declined {{ opacity: 0.72; }}
.nx-card-title {{ color: @nexa_text_strong; font-weight: 600; font-size: {design.TYPE_SCALE["title"]}pt; }}
.nx-card-status {{ color: @nexa_muted; font-size: {design.TYPE_SCALE["caption"]}pt; }}
.nx-card-status.ok {{ color: @nexa_ok; }}
.nx-card-status.bad {{ color: @nexa_crit; }}
.nx-card-status.warn {{ color: @nexa_warn; }}
.nx-card-line {{ color: @nexa_text; }}
.nx-preview {{ background-image: none; background-color: @nexa_field; color: @nexa_text;
              border-radius: {design.RADIUS["small"]}px; padding: 8px 10px;
              font-family: "{design.FONT_MONO}", monospace; font-size: 9pt; }}
.nx-btn {{ background-image: none; border-radius: {design.RADIUS["medium"]}px; padding: 4px 14px;
          border: 1px solid @nexa_border; background-color: @nexa_hover; color: @nexa_text_strong;
          text-shadow: none; -gtk-icon-shadow: none; box-shadow: none; }}
.nx-btn.primary {{ background-color: @nexa_accent; color: @nexa_on_accent; border-color: @nexa_accent; font-weight: 600; }}
.nx-btn.primary:hover {{ background-color: @nexa_accent_hover; }}
.nx-btn.danger {{ background-color: @nexa_crit; color: @nexa_on_accent; border-color: @nexa_crit; font-weight: 600; }}
.nx-chip {{ color: @nexa_muted; font-size: {design.TYPE_SCALE["caption"]}pt; padding: 2px 4px; }}
.nx-chip.bad {{ color: @nexa_crit; }}
.nx-day {{ color: @nexa_muted; font-weight: 600; font-size: {design.TYPE_SCALE["caption"]}pt;
          letter-spacing: 1px; margin-top: 10px; }}
.nx-act-row {{ padding: 6px 4px; border-radius: {design.RADIUS["medium"]}px; }}
.nx-act-row:hover {{ background-color: @nexa_hover; }}
.nx-act-time {{ color: @nexa_muted; font-size: {design.TYPE_SCALE["caption"]}pt;
               font-family: "{design.FONT_MONO}", monospace; }}
.nx-pill {{ color: @nexa_muted; border: 1px solid @nexa_border; border-radius: 10px; padding: 1px 8px;
           font-size: {design.TYPE_SCALE["caption"]}pt; }}
.nx-empty {{ color: @nexa_muted; }}
"""

_css_screens = set()

TONE_COLOUR = {"change": "nexa_accent", "danger": "nexa_crit", "system": "nexa_warn", "done": "nexa_ok",
               "muted": "nexa_muted"}


def ensure_css():
    screen = Gdk.Screen.get_default()
    if screen is None or id(screen) in _css_screens:
        return
    provider = Gtk.CssProvider()
    provider.load_from_data(CSS.encode())
    Gtk.StyleContext.add_provider_for_screen(screen, provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 2)
    _css_screens.add(id(screen))


def _colour(widget, name, fallback=(0.35, 0.6, 1.0)):
    found, rgba = widget.get_style_context().lookup_color(name)
    return (rgba.red, rgba.green, rgba.blue) if found else fallback


class HexBadge(Gtk.DrawingArea):
    """A pointy-top hexagon with the action's glyph: tinted fill, crisp edge."""

    def __init__(self, kind, tone="change", size=34):
        super().__init__()
        self.kind, self.tone = kind, tone
        self.set_size_request(size, size)
        self.set_valign(Gtk.Align.START)
        self.connect("draw", self._draw)

    def set_state(self, kind=None, tone=None):
        self.kind = kind or self.kind
        self.tone = tone or self.tone
        self.queue_draw()

    def _draw(self, _area, cr):
        w, h = self.get_allocated_width(), self.get_allocated_height()
        cx, cy, r = w / 2, h / 2, min(w, h) / 2 - 1.5
        colour = _colour(self, TONE_COLOUR.get(self.tone, "nexa_accent"))
        pts = design.hex_points(cx, cy, r)
        cr.move_to(*pts[0])
        for p in pts[1:]:
            cr.line_to(*p)
        cr.close_path()
        cr.set_source_rgba(*colour, 0.18)
        cr.fill_preserve()
        cr.set_source_rgba(*colour, 0.9)
        cr.set_line_width(1.4)
        cr.stroke()
        layout = PangoCairo.create_layout(cr)
        # A known kind's glyph, or a single character given directly (an initial).
        glyph = actiontext.GLYPHS.get(self.kind) or (self.kind if len(self.kind) == 1 else "•")
        layout.set_text(glyph, -1)
        layout.set_font_description(Pango.FontDescription(f"{design.FONT_UI} Bold {max(9, int(r * 1.0))}px"))
        tw, th = layout.get_pixel_size()
        cr.set_source_rgb(*colour)
        cr.move_to(cx - tw / 2, cy - th / 2)
        PangoCairo.show_layout(cr, layout)
        return False


def _label(text, css, wrap=True, selectable=False):
    lbl = Gtk.Label(label=text)
    lbl.set_xalign(0)
    lbl.set_line_wrap(wrap)
    lbl.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
    lbl.set_selectable(selectable)
    for c in css.split():
        lbl.get_style_context().add_class(c)
    return lbl


def _button(text, *classes):
    b = Gtk.Button(label=text)
    for c in ("nx-btn",) + classes:
        b.get_style_context().add_class(c)
    return b


class ActionCard(Gtk.Box):
    """One change, from proposal to result. Every method is GTK-thread only."""

    def __init__(self, tool_name, args, max_width_chars=52):
        super().__init__(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        ensure_css()
        self.desc = actiontext.describe(tool_name, args)
        self.undo_id = None
        self.get_style_context().add_class("nx-card")
        if self.desc["tone"] == "danger":
            self.get_style_context().add_class("danger")
        self.badge = HexBadge(self.desc["kind"], self.desc["tone"])
        self.pack_start(self.badge, False, False, 0)

        body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.pack_start(body, True, True, 0)
        title = _label(self.desc["title"], "nx-card-title")
        title.set_max_width_chars(max_width_chars)
        body.pack_start(title, False, False, 0)
        self.status = _label("Waiting…", "nx-card-status")
        body.pack_start(self.status, False, False, 0)
        for line in self.desc["lines"]:
            lbl = _label(line, "nx-card-line", selectable=True)
            lbl.set_max_width_chars(max_width_chars)
            body.pack_start(lbl, False, False, 0)
        if self.desc["preview"]:
            prev = _label(self.desc["preview"], "nx-preview", wrap=False, selectable=True)
            prev.set_ellipsize(Pango.EllipsizeMode.END)
            prev.set_max_width_chars(max_width_chars + 8)
            body.pack_start(prev, False, False, 0)
        self.buttons = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        self.revealer = Gtk.Revealer()
        self.revealer.set_transition_type(Gtk.RevealerTransitionType.SLIDE_DOWN)
        self.revealer.set_transition_duration(design.DURATION_MS["panel"])
        self.revealer.add(self.buttons)
        body.pack_start(self.revealer, False, False, 0)
        self.show_all()

    def _set_status(self, text, kind=None):
        self.status.set_text(text)
        ctx = self.status.get_style_context()
        for c in ("ok", "bad"):
            ctx.remove_class(c)
        if kind:
            ctx.add_class(kind)

    def _set_buttons(self, *buttons):
        for child in self.buttons.get_children():
            child.destroy()
        for b in buttons:
            self.buttons.pack_start(b, False, False, 0)
        self.buttons.show_all()
        self.revealer.set_reveal_child(bool(buttons))

    def _card_state(self, state):
        ctx = self.get_style_context()
        for c in ("done", "undone", "declined"):
            ctx.remove_class(c)
        if state:
            ctx.add_class(state)

    def ask(self, on_answer):
        """Show Allow / Not now; on_answer(bool) is called once."""
        self._set_status("Needs your OK")
        allow = _button("Move to Trash" if self.desc["kind"] == "trash" else "Allow",
                        "danger" if self.desc["tone"] == "danger" else "primary")
        deny = _button("Not now")

        def answer(ok):
            self._set_buttons()
            if ok:
                self.set_running()
            on_answer(ok)

        allow.connect("clicked", lambda _b: answer(True))
        deny.connect("clicked", lambda _b: answer(False))
        self._set_buttons(allow, deny)
        allow.grab_focus()

    def note(self, text):
        """An extra line under the title, e.g. why this needs an OK."""
        self._note = getattr(self, "_note", None)
        if self._note is None:
            self._note = _label(text, "nx-card-status")
            self._note.get_style_context().add_class("warn")
            body = self.status.get_parent()
            body.pack_start(self._note, False, False, 0)
            body.reorder_child(self._note, 2)
            self._note.show()
        else:
            self._note.set_text(text)

    def set_running(self):
        self._set_status("Working…")

    def set_declined(self):
        self._set_buttons()
        self._card_state("declined")
        self.badge.set_state(tone="muted")
        self._set_status("Not done: you said not now")

    def set_failed(self, error):
        self._set_buttons()
        self._set_status(f"Couldn't do it: {error}", "bad")

    def set_done(self, result):
        self._card_state("done")
        self.badge.set_state(tone="done")
        self._set_status("✓ " + actiontext.done_text(result), "ok")
        self.undo_id = result.get("undo_id") if isinstance(result, dict) else None
        if self.undo_id:
            undo = _button("Undo")
            undo.connect("clicked", self._on_undo)
            self._set_buttons(undo)
        else:
            self._set_buttons()

    def _on_undo(self, button):
        button.set_sensitive(False)
        self._set_status("Undoing…")
        run_async(lambda: hands.undo(self.undo_id), self._undo_finished)

    def _undo_finished(self, result, error):
        if error:
            self._set_status(f"Couldn't undo: {error}", "bad")
            self._set_buttons()
            return
        self._card_state("undone")
        self.badge.set_state(kind="undo", tone="muted")
        self._set_status(actiontext.done_text(result))
        self._set_buttons()


class ReadChip(Gtk.Box):
    def __init__(self, tool_name, args):
        super().__init__(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        ensure_css()
        self.tool_name, self.args = tool_name, args
        self.badge = HexBadge("web" if tool_name in actiontext.WEB_TOOLS else "look", "muted", size=18)
        self.pack_start(self.badge, False, False, 0)
        self.label = _label(actiontext.read_summary(tool_name, args, None) + "…", "nx-chip")
        self.pack_start(self.label, False, False, 0)
        self.show_all()

    def set_result(self, result, error=None):
        if error:
            self.label.set_text(f"{actiontext.read_summary(self.tool_name, self.args, None)}: {error}")
            self.label.get_style_context().add_class("bad")
        else:
            self.label.set_text(actiontext.read_summary(self.tool_name, self.args, result))


def run_async(work, done):
    """work() on a thread; done(result, error_text) back on the GTK thread."""
    def target():
        try:
            result, error = work(), None
        except hands.HandsError as e:
            result, error = None, str(e)
        GLib.idle_add(lambda: done(result, error) and False)
    threading.Thread(target=target, daemon=True).start()


class ActivityTimeline(Gtk.Box):
    """The assistant's file actions, newest first, grouped by day."""

    def __init__(self, limit=60, show_header=True):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        ensure_css()
        self.limit = limit
        if show_header:
            head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
            head.pack_start(_label("Every file change the assistant makes is listed here and can be undone.",
                                   "nx-card-status"), True, True, 0)
            refresh = _button("Refresh")
            refresh.connect("clicked", lambda _b: self.reload())
            head.pack_start(refresh, False, False, 0)
            self.pack_start(head, False, False, 0)
        self.list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        self.pack_start(self.list, False, False, 0)
        self.show_all()
        self.reload()

    def reload(self):
        run_async(lambda: hands.history(self.limit), self._fill)

    def _clear(self):
        for child in self.list.get_children():
            child.destroy()

    def _fill(self, actions, error):
        self._clear()
        if error:
            self.list.pack_start(_label(error, "nx-empty"), False, False, 8)
        elif not actions:
            self.list.pack_start(_label("Nothing yet. When the assistant creates, moves or trashes a file, "
                                        "it shows up here, ready to undo.", "nx-empty"), False, False, 8)
        for day, items in actiontext.group_by_day(actions or []):
            self.list.pack_start(_label(day.upper(), "nx-day", wrap=False), False, False, 0)
            for act in items:
                self.list.pack_start(self._row(act), False, False, 0)
        self.list.show_all()

    def _row(self, act):
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        row.get_style_context().add_class("nx-act-row")
        kind = actiontext.kind_of_summary(act.get("summary"))
        row.pack_start(HexBadge(kind, "muted" if act.get("undone") else
                                ("danger" if kind == "trash" else "change"), size=26), False, False, 0)
        summary = _label(act.get("summary", ""), "nx-card-line")
        summary.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
        summary.set_line_wrap(False)
        row.pack_start(summary, True, True, 0)
        row.pack_start(_label(time.strftime("%H:%M", time.localtime(act.get("time", 0))), "nx-act-time", wrap=False),
                       False, False, 0)
        if act.get("undone"):
            row.pack_start(_label("Undone", "nx-pill", wrap=False), False, False, 0)
        else:
            undo = _button("Undo")
            undo.connect("clicked", self._undo, act.get("id"))
            row.pack_start(undo, False, False, 0)
        return row

    def _undo(self, button, action_id):
        button.set_sensitive(False)
        run_async(lambda: hands.undo(action_id), lambda _r, err: self._after_undo(button, err))

    def _after_undo(self, button, error):
        if error:
            button.set_sensitive(True)
            button.set_tooltip_text(error)
            button.set_label("Couldn't undo")
        else:
            self.reload()
