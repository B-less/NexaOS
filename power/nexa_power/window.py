import json
import subprocess

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#heading { color: #e0e0e0; font-size: 14pt; font-weight: bold; }
#powerbtn { background-image: none; background-color: #2f2f2f; color: #d4d4d4; border-radius: 8px; padding: 12px 18px; font-size: 11pt; }
#powerbtn:hover { background-image: none; background-color: #3a3a3a; }
#dangerbtn { background-image: none; background-color: #2f2f2f; color: #f7768e; border-radius: 8px; padding: 12px 18px; font-size: 11pt; }
#dangerbtn:hover { background-image: none; background-color: #3a3a3a; }
#armed { background-image: none; background-color: #f7768e; color: #1e1e1e; border-radius: 8px; padding: 12px 18px; font-size: 11pt; font-weight: bold; }
#errorlabel { color: #f7768e; font-size: 10pt; }
"""

CONFIRM_SECONDS = 4


def _call_helper(action):
    try:
        result = subprocess.run(
            ["doas", "/usr/local/bin/nexa-power-helper"],
            input=json.dumps({"action": action}), capture_output=True, text=True, timeout=15,
        )
    except (subprocess.SubprocessError, OSError) as e:
        return False, str(e)
    try:
        response = json.loads(result.stdout)
    except json.JSONDecodeError:
        return False, (result.stderr or "").strip() or "helper produced no output"
    return bool(response.get("ok")), response.get("error", "")


class PowerWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Power")
        self.set_icon_name("nexa-power")
        attach_titlebar(self, "Power")
        self.set_default_size(300, 340)
        self.set_resizable(False)
        self._armed = None
        self._armed_timer = None
        self._build_ui()
        self.connect("key-press-event", self._on_key)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        root.set_border_width(20)
        self.add(root)

        heading = Gtk.Label(label="What would you like to do?")
        heading.set_name("heading")
        root.pack_start(heading, False, False, 6)

        self.buttons = {}
        for key, label, name in (
            ("lock", "Lock screen", "powerbtn"),
            ("logout", "Log out", "powerbtn"),
            ("reboot", "Restart", "dangerbtn"),
            ("poweroff", "Shut down", "dangerbtn"),
        ):
            btn = Gtk.Button(label=label)
            btn.set_name(name)
            btn.connect("clicked", self._on_action, key)
            root.pack_start(btn, False, False, 0)
            self.buttons[key] = (btn, label, name)

        cancel = Gtk.Button(label="Cancel")
        cancel.set_name("powerbtn")
        cancel.connect("clicked", lambda *_a: self.close())
        root.pack_start(cancel, False, False, 0)

        self.error_label = Gtk.Label(label="")
        self.error_label.set_name("errorlabel")
        self.error_label.set_line_wrap(True)
        root.pack_start(self.error_label, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _on_key(self, _win, event):
        if event.keyval == Gdk.KEY_Escape:
            self.close()
            return True
        return False

    def _disarm(self):
        if self._armed_timer:
            GLib.source_remove(self._armed_timer)
            self._armed_timer = None
        if self._armed:
            btn, label, name = self.buttons[self._armed]
            btn.set_label(label)
            btn.set_name(name)
            self._armed = None
        return False

    def _on_action(self, _btn, key):
        if key in ("reboot", "poweroff"):
            # Two clicks: the first arms the button, the second within a few
            # seconds does it, so a stray click can't power the machine off.
            if self._armed != key:
                self._disarm()
                btn = self.buttons[key][0]
                btn.set_label("Click again to confirm")
                btn.set_name("armed")
                self._armed = key
                self._armed_timer = GLib.timeout_add_seconds(CONFIRM_SECONDS, self._disarm)
                return
            self._disarm()
            ok, error = _call_helper(key)
            if not ok:
                self.error_label.set_text(error or "That didn't work.")
            return
        if key == "lock":
            subprocess.Popen(["nexa-lock"], start_new_session=True)
        elif key == "logout":
            subprocess.run(["swaymsg", "exit"], check=False)
        self.close()


def main():
    Gtk.init()
    win = PowerWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
