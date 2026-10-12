import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib

from nexa_common import wifi
from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#heading { color: @nexa_text; font-size: 15pt; font-weight: bold; }
#statuslabel { color: @nexa_muted; font-size: 10pt; }
#connected { color: @nexa_ok; font-size: 10pt; }
#errorlabel { color: @nexa_crit; font-size: 10pt; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 12px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#toolbtn:disabled { color: @nexa_disabled; }
#primarybtn { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 6px; padding: 4px 14px; font-weight: bold; }
#primarybtn:hover { background-image: none; background-color: @nexa_accent_hover; }
#primarybtn:disabled { background-image: none; background-color: @nexa_hover; color: @nexa_disabled; }
list { background-image: none; background-color: @nexa_bg; }
row { color: @nexa_text; padding: 2px 4px; }
row:selected { background-image: none; background-color: @nexa_selected; color: @nexa_selected_text; }
#netname { font-size: 11pt; }
#netmeta { color: @nexa_muted; font-size: 9pt; }
"""

SECURITY_LABEL = {"open": "Open", "psk": "Secured", "8021x": "Enterprise", "wep": "WEP"}
REFRESH_SECONDS = 8


class NetworkWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Network")
        self.set_icon_name("nexa-network")
        attach_titlebar(self, "Network")
        self.set_default_size(420, 520)
        self._busy = False
        self._networks = []

        self._build_ui()
        self._scan()
        GLib.timeout_add_seconds(REFRESH_SECONDS, self._auto_refresh)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        top = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        top.set_name("toolbar")
        top.set_border_width(12)
        heading_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        heading = Gtk.Label(label="WiFi", xalign=0)
        heading.set_name("heading")
        self.status_label = Gtk.Label(label="Looking for a WiFi adapter...", xalign=0)
        self.status_label.set_name("statuslabel")
        heading_box.pack_start(heading, False, False, 0)
        heading_box.pack_start(self.status_label, False, False, 0)
        top.pack_start(heading_box, True, True, 0)
        self.scan_btn = self._button("Scan", self._on_scan_clicked)
        top.pack_end(self.scan_btn, False, False, 0)
        root.pack_start(top, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.listbox = Gtk.ListBox()
        self.listbox.set_selection_mode(Gtk.SelectionMode.SINGLE)
        self.listbox.connect("row-selected", lambda *_a: self._update_buttons())
        self.listbox.connect("row-activated", lambda *_a: self._on_connect_clicked(None))
        scroller.add(self.listbox)
        root.pack_start(scroller, True, True, 0)

        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        actions.set_name("toolbar")
        actions.set_border_width(10)
        self.connect_btn = self._button("Connect", self._on_connect_clicked, primary=True)
        self.disconnect_btn = self._button("Disconnect", self._on_disconnect_clicked)
        self.forget_btn = self._button("Forget", self._on_forget_clicked)
        for b in (self.connect_btn, self.disconnect_btn, self.forget_btn):
            actions.pack_start(b, False, False, 0)
        root.pack_start(actions, False, False, 0)

        self.message = Gtk.Label(label="", xalign=0)
        self.message.set_name("statuslabel")
        self.message.set_line_wrap(True)
        self.message.set_margin_start(12)
        self.message.set_margin_end(12)
        self.message.set_margin_bottom(8)
        root.pack_start(self.message, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )
        self._update_buttons()

    def _button(self, label, handler, primary=False):
        btn = Gtk.Button(label=label)
        btn.set_name("primarybtn" if primary else "toolbtn")
        btn.connect("clicked", handler)
        return btn

    def _set_message(self, text, error=False):
        self.message.set_name("errorlabel" if error else "statuslabel")
        self.message.set_text(text)

    def _selected(self):
        row = self.listbox.get_selected_row()
        return getattr(row, "network", None) if row else None

    def _update_buttons(self):
        net = self._selected()
        idle = not self._busy
        self.scan_btn.set_sensitive(idle)
        self.connect_btn.set_sensitive(idle and net is not None and not net["connected"])
        self.disconnect_btn.set_sensitive(idle and net is not None and net["connected"])
        self.forget_btn.set_sensitive(idle and net is not None and net["known"])

    def _run(self, fn, on_done):
        """Runs a blocking wifi call off the main thread; on_done(result, error)
        is delivered back on it."""
        self._busy = True
        self._update_buttons()

        def work():
            result = error = None
            try:
                result = fn()
            except wifi.WifiError as e:
                error = e
            GLib.idle_add(self._finish, on_done, result, error)

        threading.Thread(target=work, daemon=True).start()

    def _finish(self, on_done, result, error):
        self._busy = False
        on_done(result, error)
        self._update_buttons()
        return False

    def _scan(self):
        self.scan_btn.set_label("Scanning...")
        self._set_message("")

        def work():
            wifi.scan()
            return self._snapshot()

        self._run(work, self._on_snapshot)

    @staticmethod
    def _snapshot():
        return {"status": wifi.status(), "networks": wifi.list_networks()}

    def _on_snapshot(self, snap, error):
        self.scan_btn.set_label("Scan")
        if error:
            self.status_label.set_text("WiFi unavailable")
            self._set_message(str(error), error=True)
            self._populate([])
            return
        self._show_snapshot(snap)

    def _show_snapshot(self, snap):
        status = snap["status"]
        if not status["adapters"]:
            self.status_label.set_text("No WiFi adapter found")
        elif status["connected_to"]:
            self.status_label.set_name("connected")
            self.status_label.set_text(f"Connected to {status['connected_to']}")
        else:
            self.status_label.set_name("statuslabel")
            self.status_label.set_text("Not connected")
        self._populate(snap["networks"])

    def _populate(self, networks):
        keep = (self._selected() or {}).get("ssid")
        for child in self.listbox.get_children():
            self.listbox.remove(child)
        self._networks = networks
        reselect = None
        for net in networks:
            row = Gtk.ListBoxRow()
            row.network = net
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            box.set_border_width(8)
            name = Gtk.Label(label=("✓ " if net["connected"] else "") + (net["ssid"] or ""), xalign=0)
            name.set_name("netname")
            bits = [SECURITY_LABEL.get(net["security"], net["security"] or ""), f"{net['signal']}%"]
            if net["known"]:
                bits.insert(1, "Saved")
            meta = Gtk.Label(label="  ·  ".join(bits), xalign=0)
            meta.set_name("netmeta")
            box.pack_start(name, False, False, 0)
            box.pack_start(meta, False, False, 0)
            row.add(box)
            self.listbox.add(row)
            if net["ssid"] == keep:
                reselect = row
        self.listbox.show_all()
        if reselect:
            self.listbox.select_row(reselect)
        self._update_buttons()

    def _auto_refresh(self):
        if self._busy:
            return True

        def work():
            return self._snapshot()

        def done(snap, error):
            if not error:
                self._show_snapshot(snap)

        self._run(work, done)
        return True

    def _on_scan_clicked(self, _btn):
        self._scan()

    def _on_connect_clicked(self, _btn):
        net = self._selected()
        if not net or net["connected"] or self._busy:
            return
        passphrase = None
        if net["security"] == "psk" and not net["known"]:
            passphrase = self._ask_passphrase(net["ssid"])
            if passphrase is None:
                return
        self._set_message(f"Connecting to {net['ssid']}...")

        def work():
            wifi.connect(net["ssid"], passphrase)
            return self._snapshot()

        def done(snap, error):
            if error:
                self._set_message(str(error), error=True)
                return
            self._set_message(f"Connected to {net['ssid']}.")
            self._show_snapshot(snap)

        self._run(work, done)

    def _on_disconnect_clicked(self, _btn):
        def work():
            wifi.disconnect()
            return self._snapshot()

        def done(snap, error):
            if error:
                self._set_message(str(error), error=True)
            else:
                self._set_message("Disconnected.")
                self._show_snapshot(snap)

        self._run(work, done)

    def _on_forget_clicked(self, _btn):
        net = self._selected()
        if not net or not self._confirm(
            f"Forget '{net['ssid']}'?", "Its saved password will be deleted.",
        ):
            return

        def work():
            wifi.forget(net["ssid"])
            return self._snapshot()

        def done(snap, error):
            if error:
                self._set_message(str(error), error=True)
            else:
                self._set_message(f"Forgot {net['ssid']}.")
                self._show_snapshot(snap)

        self._run(work, done)

    def _ask_passphrase(self, ssid):
        dialog = Gtk.Dialog(title=f"Connect to {ssid}", transient_for=self, modal=True)
        dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Connect", Gtk.ResponseType.OK)
        dialog.set_default_response(Gtk.ResponseType.OK)
        box = dialog.get_content_area()
        box.set_border_width(12)
        box.set_spacing(8)
        box.add(Gtk.Label(label=f"Password for {ssid}:", xalign=0))
        entry = Gtk.Entry()
        entry.set_visibility(False)
        entry.set_activates_default(True)
        box.add(entry)
        show = Gtk.CheckButton(label="Show password")
        show.connect("toggled", lambda b: entry.set_visibility(b.get_active()))
        box.add(show)
        dialog.show_all()
        response = dialog.run()
        text = entry.get_text()
        dialog.destroy()
        return text if response == Gtk.ResponseType.OK and text else None

    def _confirm(self, text, secondary):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
            buttons=Gtk.ButtonsType.YES_NO, text=text,
        )
        dialog.format_secondary_text(secondary)
        response = dialog.run()
        dialog.destroy()
        return response == Gtk.ResponseType.YES


def main():
    Gtk.init()
    win = NetworkWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
