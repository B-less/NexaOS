"""Settings > Nexa Link: this device, Nexa Drop, pairing (a QR code to scan,
or a code to type), and the paired devices: what each may send, Send
file, Ring, Forget. nexa-linkd (Rust) does the work (nexa_common.link)."""
import json
import os
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk

from nexa_common import link
from nexa_common.linkui import send_files, watch_lines

POLICIES = (("ask", "Ask each time"), ("always", "Always accept"), ("never", "Never accept"))
DROP_MINUTES = 10


def _bg(work, done):
    def run():
        try:
            result, error = work(), None
        except Exception as e:  # shown to the person, never raised into GTK
            result, error = None, e
        GLib.idle_add(lambda: done(result, error) and False)
    threading.Thread(target=run, daemon=True).start()


def _button(label, handler, *args):
    b = Gtk.Button(label=label, valign=Gtk.Align.CENTER)
    b.set_name("rowbtn")
    b.connect("clicked", lambda *_: handler(*args))
    return b


class QrCode(Gtk.DrawingArea):
    """A QR code, dark on white (what cameras read best), whatever the theme."""

    def __init__(self, rows, size=240):
        super().__init__()
        self.rows = rows
        self.set_size_request(size, size)
        self.set_halign(Gtk.Align.CENTER)
        self.connect("draw", self._draw)

    def _draw(self, area, cr):
        w, h = area.get_allocated_width(), area.get_allocated_height()
        n = len(self.rows)
        if not n:
            return False
        side = min(w, h)
        cell = side / (n + 8)  # 4 modules of white all round
        x0, y0 = (w - side) / 2, (h - side) / 2
        cr.set_source_rgb(1, 1, 1)
        cr.rectangle(x0, y0, side, side)
        cr.fill()
        cr.set_source_rgb(0.05, 0.07, 0.12)
        for r, row in enumerate(self.rows):
            for c, bit in enumerate(row):
                if bit == "1":
                    cr.rectangle(x0 + (c + 4) * cell, y0 + (r + 4) * cell, cell + 0.4, cell + 0.4)
        cr.fill()
        return False


class PairDialog(Gtk.Dialog):
    """Pairing, either way: show a code (QR) and wait, or type the other
    device's code. Both screens then show six digits to compare."""

    def __init__(self, parent, code=None):
        super().__init__(title="Pair a device", transient_for=parent, modal=True)
        self.set_default_size(420, -1)
        self.proc = None
        self.paired = False
        box = self.get_content_area()
        box.set_spacing(12)
        box.set_border_width(18)
        self.body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        box.pack_start(self.body, True, True, 0)
        self.close_btn = self.add_button("Cancel", Gtk.ResponseType.CANCEL)
        self.connect("response", lambda *_a: self._stop())
        if code is None:
            self._say("Getting a pairing code ready…")
            self._start("pair-offer")
        else:
            self._say("Connecting…")
            self._start("pair-accept", code)
        self.show_all()

    def _clear(self):
        for c in self.body.get_children():
            c.destroy()

    def _say(self, text, title=None):
        self._clear()
        if title:
            t = Gtk.Label(label=title, xalign=0)
            t.set_name("value")
            self.body.pack_start(t, False, False, 0)
        label = Gtk.Label(label=text, xalign=0)
        label.set_line_wrap(True)
        label.set_max_width_chars(48)
        self.body.pack_start(label, False, False, 0)
        self.body.show_all()

    def _start(self, *args):
        try:
            self.proc = link.start(*args)
        except OSError as e:
            self._say(f"Nexa Link isn't installed ({e}).")
            return
        watch_lines(self.proc, self._on_event)

    def _on_event(self, ev):
        kind = ev.get("event")
        if kind == "code":
            self._clear()
            intro = Gtk.Label(label="On the other NexaOS device, open Settings > Nexa Link > Enter a pairing code, "
                                    "and scan this or type the code under it.", xalign=0)
            intro.set_line_wrap(True)
            intro.set_max_width_chars(48)
            self.body.pack_start(intro, False, False, 0)
            try:
                self.body.pack_start(QrCode(link.qr_rows(ev["code"])), False, False, 0)
            except link.LinkError:
                pass
            code = Gtk.Label(label=ev["code"], selectable=True)
            code.set_name("fieldlabel")
            self.body.pack_start(code, False, False, 0)
            self.body.pack_start(Gtk.Label(label="The code works once, for 2 minutes.", xalign=0), False, False, 0)
            self.body.show_all()
        elif kind == "confirm":
            self._clear()
            digits = ev.get("sas", "")
            t = Gtk.Label(label=f"Pairing with “{ev.get('name', 'a device')}”", xalign=0)
            t.set_name("value")
            self.body.pack_start(t, False, False, 0)
            self.body.pack_start(Gtk.Label(label="Does the other device show these digits?", xalign=0), False, False, 0)
            big = Gtk.Label()
            big.set_markup(f"<span size='xx-large' weight='bold' font_family='JetBrains Mono'>{digits[:3]} {digits[3:]}</span>")
            self.body.pack_start(big, False, False, 6)
            row = Gtk.Box(spacing=8, halign=Gtk.Align.END)
            no = Gtk.Button(label="No, they're different")
            no.connect("clicked", lambda *_: self._confirm(False))
            yes = Gtk.Button(label="Yes, they match")
            yes.set_name("actionbtn")
            yes.connect("clicked", lambda *_: self._confirm(True))
            row.pack_start(no, False, False, 0)
            row.pack_start(yes, False, False, 0)
            self.body.pack_start(row, False, False, 0)
            self.body.show_all()
        elif kind == "paired":
            self.paired = True
            self._say(f"You can now send files to it, and it finds this device by itself whenever it's near.",
                      title=f"Paired with “{ev.get('name')}”")
            self.close_btn.set_label("Done")
        elif kind == "cancelled":
            self._say("Nothing was paired. If the digits were different, someone else may have tried to connect: "
                      "start again with a new code.", title="Not paired")
            self.close_btn.set_label("Close")
        elif kind == "expired":
            self._say("No device came to pair within 2 minutes.", title="The code ran out")
            self.close_btn.set_label("Close")
        elif kind == "ended" and not self.paired and ev.get("error"):
            self._say(ev["error"], title="Couldn't pair")
            self.close_btn.set_label("Close")

    def _confirm(self, ok):
        self._say("Waiting for the other device…")
        try:
            self.proc.stdin.write("yes\n" if ok else "no\n")
            self.proc.stdin.flush()
        except (OSError, ValueError):
            pass

    def _stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()


class CodeDialog(Gtk.Dialog):
    def __init__(self, parent):
        super().__init__(title="Enter a pairing code", transient_for=parent, modal=True)
        box = self.get_content_area()
        box.set_spacing(10)
        box.set_border_width(18)
        intro = Gtk.Label(label="Type the code the other NexaOS device shows (Settings > Nexa Link > Pair a device).", xalign=0)
        intro.set_line_wrap(True)
        intro.set_max_width_chars(44)
        box.pack_start(intro, False, False, 0)
        self.entry = Gtk.Entry(placeholder_text="NEXA-LINK:1:…")
        self.entry.set_activates_default(True)
        box.pack_start(self.entry, False, False, 0)
        self.add_button("Cancel", Gtk.ResponseType.CANCEL)
        ok = self.add_button("Pair", Gtk.ResponseType.OK)
        ok.set_name("actionbtn")
        self.set_default_response(Gtk.ResponseType.OK)
        self.show_all()


class LinkPage:
    def __init__(self, win):
        self.win = win
        self.page = win._page("Nexa Link")
        intro = win._field_label("Your NexaOS devices find each other on the same network and talk directly, "
                                 "encrypted: no cloud, no account. Only Nexa devices can read their signals.")
        intro.set_line_wrap(True)
        self.page.pack_start(intro, False, False, 0)

        me = Gtk.Box(spacing=10)
        self.me_label = Gtk.Label(xalign=0)
        self.me_label.set_name("value")
        me.pack_start(self.me_label, True, True, 0)
        me.pack_end(_button("Rename…", self._on_rename), False, False, 0)
        self.page.pack_start(me, False, False, 0)

        drop = Gtk.Box(spacing=12)
        drop_text = win._field_label(f"Nexa Drop: receive from any Nexa device nearby for {DROP_MINUTES} minutes "
                                     "(each item is asked first)")
        drop_text.set_line_wrap(True)
        drop.pack_start(drop_text, True, True, 0)
        self.drop_switch = Gtk.Switch(valign=Gtk.Align.CENTER)
        self._drop_handler = self.drop_switch.connect("notify::active", self._on_drop)
        drop.pack_end(self.drop_switch, False, False, 0)
        self.page.pack_start(drop, False, False, 0)

        buttons = Gtk.Box(spacing=8)
        pair = Gtk.Button(label="Pair a device…")
        pair.set_name("actionbtn")
        pair.connect("clicked", self._on_pair)
        buttons.pack_start(pair, False, False, 0)
        code = Gtk.Button(label="Enter a pairing code…")
        code.connect("clicked", self._on_code)
        buttons.pack_start(code, False, False, 0)
        self.page.pack_start(buttons, False, False, 0)

        self.page.pack_start(win._field_label("Your devices"), False, False, 0)
        self.devices = Gtk.ListBox()
        self.devices.set_selection_mode(Gtk.SelectionMode.NONE)
        self.page.pack_start(self.devices, False, False, 0)
        self.page.pack_start(win._field_label("Taking drops nearby"), False, False, 0)
        self.nearby = Gtk.ListBox()
        self.nearby.set_selection_mode(Gtk.SelectionMode.NONE)
        self.page.pack_start(self.nearby, False, False, 0)
        self.status = Gtk.Label(xalign=0)
        self.status.set_line_wrap(True)
        self.page.pack_start(self.status, False, False, 0)
        self._last = None
        self.refresh()
        GLib.timeout_add_seconds(3, self._tick)

    def _tick(self):
        if self.page.get_mapped():
            self.refresh()
        return True

    def _say(self, text, error=False):
        self.win._set_label(self.status, text, error=error)

    def refresh(self):
        def work():
            return link.identity(), link.status(), link.devices(), link.nearby()
        _bg(work, self._show)

    def _show(self, result, error):
        if error:
            self.me_label.set_text("Nexa Link")
            self._say(f"{error} It starts with the desktop; signing out and in again starts it.", error=True)
            return
        me, status, devices, nearby = result
        self.me_label.set_text(f"This device: {me.get('name')}")
        self.me_label.set_tooltip_text(f"Device id {me.get('id')}")
        with self.drop_switch.handler_block(self._drop_handler):
            self.drop_switch.set_active(bool(status.get("drop_open")))
        key = json.dumps([devices, [n for n in nearby if n.get("open_drop")]], sort_keys=True)
        if key == self._last:
            return
        self._last = key
        for lb in (self.devices, self.nearby):
            for child in lb.get_children():
                child.destroy()
        if not devices:
            self.devices.add(self.win._field_label("No devices yet. Pair one: both need NexaOS and the same network."))
        for d in devices:
            self.devices.add(self._device_row(d))
        drops = [n for n in nearby if n.get("open_drop")]
        if not drops:
            self.nearby.add(self.win._field_label("None right now."))
        for n in drops:
            row = Gtk.Box(spacing=10)
            row.set_margin_top(4)
            row.pack_start(self.win._field_label(f"A Nexa device at {n['addr']}"), True, True, 0)
            row.pack_end(_button("Send file…", self._on_send, n["addr"], f"the device at {n['addr']}"), False, False, 0)
            self.nearby.add(row)
        self.devices.show_all()
        self.nearby.show_all()

    def _device_row(self, d):
        row = Gtk.Box(spacing=10)
        row.set_margin_top(6)
        row.set_margin_bottom(6)
        dot = Gtk.Label()
        dot.set_markup("<span foreground='#5ee6a8'>●</span>" if d.get("online") else "<span foreground='#8f9bc4'>○</span>")
        row.pack_start(dot, False, False, 0)
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        name = Gtk.Label(label=d["name"], xalign=0)
        name.set_name("value")
        text.pack_start(name, False, False, 0)
        text.pack_start(self.win._field_label("Nearby" if d.get("online") else "Not nearby"), False, False, 0)
        row.pack_start(text, True, True, 0)
        policy = Gtk.ComboBoxText(valign=Gtk.Align.CENTER)
        policy.set_tooltip_text("Files from this device")
        for key, label in POLICIES:
            policy.append(key, label)
        policy.set_active_id(d.get("files", "ask"))
        policy.connect("changed", lambda c: self._run(lambda: link.allow(d["id"], c.get_active_id()),
                                                      f"Files from “{d['name']}”: {c.get_active_text().lower()}."))
        row.pack_start(policy, False, False, 0)
        send = _button("Send file…", self._on_send, d["id"], d["name"])
        ring = _button("Ring", lambda: self._run(lambda: link.ring(d["id"]), f"“{d['name']}” is ringing."))
        for b in (send, ring):
            b.set_sensitive(bool(d.get("online")))
        row.pack_start(send, False, False, 0)
        row.pack_start(ring, False, False, 0)
        row.pack_start(_button("Forget", self._on_forget, d), False, False, 0)
        return row

    def _run(self, work, success):
        self._say("Working…")
        _bg(work, lambda _r, e: (self._say(str(e), error=True) if e else self._say(success), self.refresh()))

    def _on_drop(self, switch, _pspec):
        minutes = DROP_MINUTES if switch.get_active() else 0
        self._run(lambda: link.set_drop(minutes),
                  f"Nexa Drop is on for {DROP_MINUTES} minutes." if minutes else "Nexa Drop is off.")

    def _on_rename(self):
        dialog = Gtk.Dialog(title="Rename this device", transient_for=self.win, modal=True)
        entry = Gtk.Entry(text=self.me_label.get_text().removeprefix("This device: "))
        entry.set_activates_default(True)
        box = dialog.get_content_area()
        box.set_border_width(16)
        box.pack_start(Gtk.Label(label="The name your other devices see", xalign=0), False, False, 6)
        box.pack_start(entry, False, False, 0)
        dialog.add_button("Cancel", Gtk.ResponseType.CANCEL)
        dialog.add_button("Rename", Gtk.ResponseType.OK).set_name("actionbtn")
        dialog.set_default_response(Gtk.ResponseType.OK)
        dialog.show_all()
        if dialog.run() == Gtk.ResponseType.OK and entry.get_text().strip():
            name = entry.get_text().strip()
            self._run(lambda: link.rename(name), f"This device is now “{name}”.")
        dialog.destroy()

    def _on_pair(self):
        d = PairDialog(self.win)
        d.run()
        d.destroy()
        self._last = None
        self.refresh()

    def _on_code(self):
        d = CodeDialog(self.win)
        code = d.entry.get_text().strip() if d.run() == Gtk.ResponseType.OK else ""
        d.destroy()
        if code:
            p = PairDialog(self.win, code=code)
            p.run()
            p.destroy()
            self._last = None
            self.refresh()

    def _on_forget(self, d):
        dialog = Gtk.MessageDialog(transient_for=self.win, modal=True, message_type=Gtk.MessageType.QUESTION,
                                   buttons=Gtk.ButtonsType.NONE, text=f"Forget “{d['name']}”?")
        dialog.format_secondary_text("It's cut off at once: it can't find or reach this device until you pair again.")
        dialog.add_button("Cancel", Gtk.ResponseType.CANCEL)
        dialog.add_button("Forget", Gtk.ResponseType.OK)
        if dialog.run() == Gtk.ResponseType.OK:
            self._run(lambda: link.forget(d["id"]), f"“{d['name']}” forgotten.")
        dialog.destroy()

    def _on_send(self, target, name):
        chooser = Gtk.FileChooserDialog(title=f"Send to {name}", transient_for=self.win, action=Gtk.FileChooserAction.OPEN)
        chooser.set_select_multiple(True)
        chooser.add_button("Cancel", Gtk.ResponseType.CANCEL)
        chooser.add_button("Send", Gtk.ResponseType.OK)
        chooser.set_current_folder(os.path.expanduser("~"))
        files = chooser.get_filenames() if chooser.run() == Gtk.ResponseType.OK else []
        chooser.destroy()
        if files:
            send_files(target, name, files, self._say)


