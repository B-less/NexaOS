"""Nexa Link's pop-ups: a card at the top right, over the windows, for what
another device sends. A file or message offered ("Save" / "Decline"; an
unpaired Nexa Drop device says so), a file received ("Open" / "Show in
Files"), a message or link ("Copy" / "Open"), and Ring (a tone until
"Stop"). Listens to nexa-linkd's events; waits for it if it isn't
running yet."""
import math
import os
import struct
import subprocess
import wave

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gdk, Gio, GLib, Gtk, GtkLayerShell, Pango

from nexa_common import link, themes

BAR_H = 44
OFFER_SECONDS = 120


def _css(t, pal):
    return f"""
.nl-card {{ background-color: {pal['surface']}; border-radius: 14px; border: 1px solid alpha({t['accent']}, 0.45);
            padding: 12px 14px; box-shadow: inset 0 1px rgba(255,255,255,0.12); }}
.nl-title {{ color: {pal['text_strong']}; font-weight: 700; font-family: Inter, sans-serif; }}
.nl-body {{ color: {pal['text']}; font-family: Inter, sans-serif; }}
.nl-note {{ color: {pal['muted']}; font-size: 9pt; font-family: Inter, sans-serif; }}
.nl-btn {{ background-image: none; background-color: {pal['surface_hi']}; color: {pal['text']}; border-radius: 8px;
           padding: 4px 12px; border: none; }}
.nl-btn:hover {{ background-color: alpha({t['accent']}, 0.25); }}
.nl-yes {{ background-image: none; background-color: {t['accent']}; color: #ffffff; border-radius: 8px; padding: 4px 14px;
           border: none; font-weight: 700; }}
"""


def _tone_file():
    """A short two-note chime (written once): Ring plays it over and over."""
    path = os.path.join(os.environ.get("XDG_RUNTIME_DIR") or "/tmp", "nexa-link-ring.wav")
    if not os.path.exists(path):
        rate = 22050
        frames = bytearray()
        for freq, secs in ((880, 0.18), (0, 0.06), (1320, 0.28), (0, 0.5)):
            for i in range(int(rate * secs)):
                fade = min(1.0, (int(rate * secs) - i) / 800)
                v = int(9000 * fade * math.sin(2 * math.pi * freq * i / rate)) if freq else 0
                frames += struct.pack("<h", v)
        with wave.open(path, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(rate)
            w.writeframes(bytes(frames))
    return path


def _play(path):
    for player in (["pw-play", path], ["paplay", path], ["aplay", "-q", path]):
        try:
            return subprocess.Popen(player, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except OSError:
            continue
    return None


class Notifier(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa Link")
        GtkLayerShell.init_for_window(self)
        GtkLayerShell.set_namespace(self, "nexa-link-notify")
        GtkLayerShell.set_layer(self, GtkLayerShell.Layer.OVERLAY)
        GtkLayerShell.set_keyboard_mode(self, GtkLayerShell.KeyboardMode.NONE)
        for edge, on in ((GtkLayerShell.Edge.TOP, True), (GtkLayerShell.Edge.RIGHT, True)):
            GtkLayerShell.set_anchor(self, edge, on)
        GtkLayerShell.set_margin(self, GtkLayerShell.Edge.TOP, BAR_H + 10)
        GtkLayerShell.set_margin(self, GtkLayerShell.Edge.RIGHT, 14)
        visual = self.get_screen().get_rgba_visual()
        if visual:
            self.set_visual(visual)
        self.set_app_paintable(True)
        self.connect("draw", self._clear)
        self.stack = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.add(self.stack)
        self.provider = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), self.provider,
                                                 Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 2)
        self._load_theme()
        self._theme_mon = Gio.File.new_for_path(themes.settings_path()).monitor_file(Gio.FileMonitorFlags.NONE, None)
        self._theme_mon.connect("changed", lambda *_a: self._load_theme())
        self.sock = None
        self.buf = b""
        self.ringer = None
        self._connect()

    def _clear(self, _w, cr):
        import cairo
        cr.set_operator(cairo.OPERATOR_CLEAR)
        cr.paint()
        cr.set_operator(cairo.OPERATOR_OVER)
        return False

    def _load_theme(self):
        t = themes.THEMES[themes.load_settings()["theme"]]
        self.provider.load_from_data(_css(t, themes.app_palette(t)).encode())

    # ---- the service's events -------------------------------------------
    def _connect(self):
        try:
            self.sock = link.subscribe()
        except link.LinkError:
            GLib.timeout_add_seconds(5, lambda: self._connect() and False)
            return False
        GLib.io_add_watch(self.sock.fileno(), GLib.PRIORITY_DEFAULT, GLib.IO_IN | GLib.IO_HUP | GLib.IO_ERR, self._readable)
        return False

    def _readable(self, _fd, condition):
        data = b""
        if condition & GLib.IO_IN:
            try:
                data = self.sock.recv(65536)
            except OSError:
                data = b""
        if not data:
            self.sock.close()
            self.sock = None
            GLib.timeout_add_seconds(3, lambda: self._connect() and False)
            return False
        self.buf += data
        while b"\n" in self.buf:
            line, self.buf = self.buf.split(b"\n", 1)
            try:
                import json
                ev = json.loads(line)
            except ValueError:
                continue
            self.handle(ev)
        return True

    def handle(self, ev):
        kind = ev.get("event")
        if kind == "offer":
            self._offer(ev)
        elif kind == "received":
            self._received(ev)
        elif kind == "text":
            self._text(ev)
        elif kind == "ring":
            self._ring(ev)

    # ---- cards ------------------------------------------------------------
    def _card(self, title, body=None, note=None, seconds=None):
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        card.get_style_context().add_class("nl-card")
        card.set_size_request(340, -1)
        head = Gtk.Box(spacing=8)
        icon = Gtk.Image.new_from_icon_name("nexa-network", Gtk.IconSize.LARGE_TOOLBAR)
        head.pack_start(icon, False, False, 0)
        t = Gtk.Label(label=title, xalign=0)
        t.set_line_wrap(True)
        t.set_max_width_chars(36)
        t.get_style_context().add_class("nl-title")
        head.pack_start(t, True, True, 0)
        card.pack_start(head, False, False, 0)
        if body:
            b = Gtk.Label(label=body, xalign=0)
            b.set_line_wrap(True)
            b.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
            b.set_max_width_chars(42)
            b.get_style_context().add_class("nl-body")
            card.pack_start(b, False, False, 0)
        if note:
            n = Gtk.Label(label=note, xalign=0)
            n.set_line_wrap(True)
            n.set_max_width_chars(44)
            n.get_style_context().add_class("nl-note")
            card.pack_start(n, False, False, 0)
        card.buttons = Gtk.Box(spacing=8, halign=Gtk.Align.END)
        card.pack_start(card.buttons, False, False, 2)
        self.stack.pack_start(card, False, False, 0)
        if seconds:
            GLib.timeout_add_seconds(seconds, lambda: self._close(card) and False)
        return card

    def _button(self, card, label, action, primary=False):
        b = Gtk.Button(label=label)
        b.get_style_context().add_class("nl-yes" if primary else "nl-btn")
        b.connect("clicked", lambda *_a: (action(), self._close(card)))
        card.buttons.pack_start(b, False, False, 0)
        return b

    def _show(self):
        self.stack.show_all()
        self.show_all()

    def _close(self, card):
        if card.get_parent() is self.stack:
            card.destroy()
        if not self.stack.get_children():
            self.hide()
        return False

    def _offer(self, ev):
        offer, who = ev.get("offer"), ev.get("from") or "A Nexa device"
        if ev.get("kind") == "text":
            title, body = f"{who} wants to send you a message", None
        else:
            title, body = f"{who} wants to send you a file", f"{ev.get('name')} · {link.human_size(ev.get('size'))}"
        note = None if ev.get("paired") else "A Nexa device you haven't paired with (Nexa Drop)."
        card = self._card(title, body, note, seconds=OFFER_SECONDS)
        answered = []

        def reply(ok):
            answered.append(ok)
            try:
                link.answer(offer, ok)
            except link.LinkError:
                pass
        self._button(card, "Decline", lambda: reply(False))
        self._button(card, "Save" if ev.get("kind") != "text" else "Show", lambda: reply(True), primary=True)
        card.connect("destroy", lambda *_a: None if answered else reply(False))
        self._show()

    def _received(self, ev):
        path = ev.get("path", "")
        card = self._card(f"Saved {os.path.basename(path)}", f"From {ev.get('from')}, in Downloads.", seconds=12)
        self._button(card, "Show in Files", lambda: subprocess.Popen(["nexa-files", os.path.dirname(path)], start_new_session=True))
        self._button(card, "Open", lambda: Gio.AppInfo.launch_default_for_uri(Gio.File.new_for_path(path).get_uri(), None),
                     primary=True)
        self._show()

    def _text(self, ev):
        text = ev.get("text", "")
        card = self._card(f"{ev.get('from')} sent you a message", text, seconds=45)
        self._button(card, "Copy", lambda: Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text(text, -1))
        if text.strip().startswith(("https://", "http://")) and " " not in text.strip():
            self._button(card, "Open", lambda: Gio.AppInfo.launch_default_for_uri(text.strip(), None), primary=True)
        self._show()

    def _ring(self, ev):
        card = self._card(f"{ev.get('from')} is looking for this device", None, None, seconds=60)
        tone = _tone_file()

        def again():
            if card.get_parent() is None:
                return False
            if self.ringer is None or self.ringer.poll() is not None:
                self.ringer = _play(tone)
            return True
        again()
        GLib.timeout_add(1100, again)

        def stop():
            if self.ringer and self.ringer.poll() is None:
                self.ringer.terminate()
        self._button(card, "Stop", stop, primary=True)
        card.connect("destroy", lambda *_a: stop())
        self._show()


def main():
    Gtk.init()
    win = Notifier()
    win.connect("destroy", Gtk.main_quit)
    Gtk.main()
    return 0
