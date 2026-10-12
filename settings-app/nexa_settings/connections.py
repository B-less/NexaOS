"""Settings pages for Bluetooth and VPN (WireGuard)."""
import os
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib

from . import privileged
from nexa_common import bluetooth as bt
from nexa_common import wgconf

SEARCH_SECONDS = 30


def _in_background(work, done):
    """Run work() off the main thread, then done(result, error) on it."""
    def run():
        try:
            result, error = work(), None
        except Exception as e:  # shown to the user, never raised into GTK
            result, error = None, e
        GLib.idle_add(lambda: done(result, error) and False)
    threading.Thread(target=run, daemon=True).start()


def _button(label, handler, *args):
    b = Gtk.Button(label=label, valign=Gtk.Align.CENTER)
    b.set_name("rowbtn")
    b.connect("clicked", lambda *_: handler(*args))
    return b


class BluetoothPage:
    def __init__(self, win):
        self.win = win
        self.page = win._page("Bluetooth")
        self.status = Gtk.Label(xalign=0)
        self.status.set_line_wrap(True)
        self._shown = None
        self._busy = False

        row = Gtk.Box(spacing=12)
        row.pack_start(win._field_label("Bluetooth"), True, True, 0)
        self.switch = Gtk.Switch()
        self.switch_handler = self.switch.connect("notify::active", self._on_power)
        row.pack_start(self.switch, False, False, 0)
        self.page.pack_start(row, False, False, 0)

        self.search_btn = Gtk.Button(label="Search for devices")
        self.search_btn.set_name("actionbtn")
        self.search_btn.set_halign(Gtk.Align.START)
        self.search_btn.connect("clicked", self._on_search)
        self.page.pack_start(self.search_btn, False, False, 0)
        self.page.pack_start(win._field_label("Devices"), False, False, 0)
        self.list = Gtk.ListBox()
        self.list.set_selection_mode(Gtk.SelectionMode.NONE)
        self.page.pack_start(self.list, False, False, 0)
        self.page.pack_start(self.status, False, False, 0)

        try:
            bt.register_agent(self._ask)
        except bt.BluetoothError:
            pass  # BlueZ not running yet; refresh() says so
        self.refresh()
        GLib.timeout_add_seconds(2, self._tick)

    def _tick(self):
        if self.win.stack.get_visible_child_name() == "Bluetooth" and not self._busy:
            self.refresh()
        return True

    def refresh(self):
        _in_background(lambda: (bt.adapter(), bt.devices() if bt.adapter() else []), self._show)

    def _show(self, result, error):
        if error:
            self._set_enabled(False)
            self.win._set_label(self.status, str(error), error=True)
            return
        adapter, devices = result
        if adapter is None:
            self._set_enabled(False)
            self.win._set_label(self.status, "No Bluetooth adapter found.", error=True)
            return
        self.switch.handler_block(self.switch_handler)
        self.switch.set_active(adapter["powered"])
        self.switch.handler_unblock(self.switch_handler)
        self.search_btn.set_sensitive(adapter["powered"] and not adapter["discovering"])
        self.search_btn.set_label("Searching…" if adapter["discovering"] else "Search for devices")
        self.switch.set_sensitive(True)
        key = (adapter["powered"], tuple((d["path"], d["name"], d["paired"], d["connected"]) for d in devices))
        if key == self._shown:
            return
        self._shown = key
        for child in self.list.get_children():
            child.destroy()
        if not adapter["powered"]:
            self._placeholder("Bluetooth is off.")
        elif not devices:
            self._placeholder("No devices yet. Put your device in pairing mode and press Search.")
        for d in devices if adapter["powered"] else []:
            self.list.add(self._device_row(d))
        self.list.show_all()

    def _set_enabled(self, on):
        self.switch.set_sensitive(on)
        self.search_btn.set_sensitive(on)

    def _placeholder(self, text):
        label = self.win._field_label(text)
        label.set_margin_top(6)
        self.list.add(label)

    def _device_row(self, d):
        row = Gtk.Box(spacing=10)
        row.set_margin_top(6)
        row.set_margin_bottom(6)
        row.pack_start(Gtk.Image.new_from_icon_name(d["icon"] or "bluetooth", Gtk.IconSize.LARGE_TOOLBAR), False, False, 0)
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        name = Gtk.Label(label=d["name"], xalign=0)
        name.set_name("value")
        state = "Connected" if d["connected"] else "Paired" if d["paired"] else "Not paired"
        text.pack_start(name, False, False, 0)
        text.pack_start(self.win._field_label(state), False, False, 0)
        row.pack_start(text, True, True, 0)
        if d["paired"]:
            row.pack_end(_button("Forget", self._act, bt.forget, d, f"Forgot {d['name']}."), False, False, 0)
            if d["connected"]:
                row.pack_end(_button("Disconnect", self._act, bt.disconnect, d, f"Disconnected {d['name']}."), False, False, 0)
            else:
                row.pack_end(_button("Connect", self._act, bt.connect, d, f"Connected to {d['name']}."), False, False, 0)
        else:
            row.pack_end(_button("Pair", self._act, bt.pair, d, f"Paired with {d['name']}."), False, False, 0)
        return row

    def _act(self, fn, device, success):
        self._busy = True
        self.win._set_label(self.status, f"Working on {device['name']}…", error=False)

        def done(_result, error):
            self._busy = False
            self.win._set_label(self.status, str(error) if error else success, error=bool(error))
            self._shown = None
            self.refresh()
        _in_background(lambda: fn(device["path"]), done)

    def _on_power(self, switch, _pspec):
        on = switch.get_active()

        def work():
            try:
                bt.set_powered(on)
            except bt.Blocked:
                privileged.call("bluetooth_unblock")
                bt.set_powered(on)

        def done(_r, error):
            if error:
                self.win._set_label(self.status, str(error), error=True)
            else:
                self.status.set_text("")
            self.refresh()
        _in_background(work, done)

    def _on_search(self, _btn):
        def done(_r, error):
            if error:
                self.win._set_label(self.status, str(error), error=True)
            else:
                GLib.timeout_add_seconds(SEARCH_SECONDS, lambda: _in_background(bt.stop_discovery, lambda *a: self.refresh()) and False)
            self.refresh()
        _in_background(bt.start_discovery, done)

    def _ask(self, kind, device, value):
        if kind == "display":
            text, buttons = f"Type {value} on {device}, then press Enter on it.", Gtk.ButtonsType.OK
        elif kind == "confirm" and value:
            text, buttons = f"Does {device} show the code {value}?", Gtk.ButtonsType.YES_NO
        elif kind == "confirm":
            text, buttons = f"Allow {device} to pair with this computer?", Gtk.ButtonsType.YES_NO
        else:
            return self._ask_pin(device)
        dialog = Gtk.MessageDialog(transient_for=self.win, modal=True, message_type=Gtk.MessageType.QUESTION,
                                   buttons=buttons, text="Bluetooth pairing")
        dialog.format_secondary_text(text)
        answer = dialog.run()
        dialog.destroy()
        return answer in (Gtk.ResponseType.YES, Gtk.ResponseType.OK)

    def _ask_pin(self, device):
        dialog = Gtk.Dialog(title="Bluetooth pairing", transient_for=self.win, modal=True)
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Pair", Gtk.ResponseType.OK)
        box = dialog.get_content_area()
        box.set_border_width(12)
        box.set_spacing(8)
        box.add(Gtk.Label(label=f"Enter the PIN for {device} (often 0000 or 1234):", xalign=0))
        entry = Gtk.Entry(activates_default=True)
        box.add(entry)
        dialog.set_default_response(Gtk.ResponseType.OK)
        dialog.show_all()
        ok = dialog.run() == Gtk.ResponseType.OK
        pin = entry.get_text().strip()
        dialog.destroy()
        return pin if ok and pin else None


class VpnPage:
    def __init__(self, win):
        self.win = win
        self.page = win._page("VPN")
        intro = win._field_label("Connect through a WireGuard VPN. Get a .conf file from your VPN provider "
                                 "(most offer WireGuard) and add it here. Files that run commands are refused.")
        intro.set_line_wrap(True)
        self.page.pack_start(intro, False, False, 0)
        add = Gtk.Button(label="Add VPN from file…")
        add.set_name("actionbtn")
        add.set_halign(Gtk.Align.START)
        add.connect("clicked", self._on_add)
        self.page.pack_start(add, False, False, 0)
        self.list = Gtk.ListBox()
        self.list.set_selection_mode(Gtk.SelectionMode.NONE)
        self.page.pack_start(self.list, False, False, 0)
        self.status = Gtk.Label(xalign=0)
        self.status.set_line_wrap(True)
        self.page.pack_start(self.status, False, False, 0)
        self.refresh()

    def refresh(self):
        _in_background(lambda: privileged.call("vpn_list")["tunnels"], self._show)

    def _show(self, tunnels, error):
        for child in self.list.get_children():
            child.destroy()
        if error:
            self.win._set_label(self.status, f"Couldn't read VPNs: {error}", error=True)
            return
        if not tunnels:
            label = self.win._field_label("No VPNs added yet.")
            label.set_margin_top(6)
            self.list.add(label)
        for t in tunnels:
            row = Gtk.Box(spacing=10)
            row.set_margin_top(6)
            row.set_margin_bottom(6)
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            name = Gtk.Label(label=t["name"], xalign=0)
            name.set_name("value")
            text.pack_start(name, False, False, 0)
            text.pack_start(self.win._field_label("Connected — all traffic goes through this VPN" if t["active"]
                                                  else "Off"), False, False, 0)
            row.pack_start(text, True, True, 0)
            switch = Gtk.Switch(active=t["active"], valign=Gtk.Align.CENTER)
            switch.connect("notify::active", self._on_toggle, t["name"])
            row.pack_end(_button("Remove", self._on_remove, t["name"]), False, False, 0)
            row.pack_end(switch, False, False, 0)
            self.list.add(row)
        self.list.show_all()

    def _run(self, action, name, success):
        self.win._set_label(self.status, "Working…", error=False)

        def done(_r, error):
            self.win._set_label(self.status, str(error) if error else success, error=bool(error))
            self.refresh()
        _in_background(lambda: privileged.call(action, timeout=45, name=name), done)

    def _on_toggle(self, switch, _pspec, name):
        if switch.get_active():
            self._run("vpn_up", name, f"Connected to {name}.")
        else:
            self._run("vpn_down", name, f"Disconnected from {name}.")

    def _on_remove(self, name):
        self._run("vpn_remove", name, f"Removed {name}.")

    def _on_add(self, _btn):
        chooser = Gtk.FileChooserNative(title="Choose a WireGuard file", transient_for=self.win,
                                        action=Gtk.FileChooserAction.OPEN)
        flt = Gtk.FileFilter()
        flt.set_name("WireGuard files (*.conf)")
        flt.add_pattern("*.conf")
        chooser.add_filter(flt)
        if chooser.run() != Gtk.ResponseType.ACCEPT:
            return
        path = chooser.get_filename()
        try:
            if os.path.getsize(path) > 64 * 1024:
                raise OSError("That file is too big to be a WireGuard file.")
            with open(path) as f:
                text = f.read()
        except (OSError, UnicodeDecodeError) as e:
            self.win._set_label(self.status, f"Couldn't read the file: {e}", error=True)
            return
        name = wgconf.name_from_filename(os.path.basename(path))
        self.win._set_label(self.status, "Adding…", error=False)

        def done(_r, error):
            self.win._set_label(self.status, str(error) if error else f"Added {name}. Turn it on below.", error=bool(error))
            self.refresh()
        _in_background(lambda: privileged.call("vpn_import", name=name, config=text), done)
