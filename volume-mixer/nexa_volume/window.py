import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib

from nexa_common import audioctl
from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#volumelabel { color: #e0e0e0; font-size: 28pt; font-weight: bold; }
#mutebtn { background-image: none; background-color: #2f2f2f; color: #d4d4d4; border-radius: 8px; padding: 8px 16px; font-size: 11pt; }
#mutebtn:hover { background-image: none; background-color: #3a3a3a; }
#mutebtn:checked { background-image: none; background-color: #f7768e; color: #1e1e1e; }
#errorlabel { color: #f7768e; font-size: 10pt; }
"""


class VolumeWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Volume")
        self.set_icon_name("nexa-volume")
        attach_titlebar(self, "Volume")
        self.set_default_size(280, 420)
        # Guards the feedback loop: _refresh()/_poll() write into the scale
        # and mute button, which would otherwise re-trigger their own
        # "value-changed"/"toggled" handlers and call wpctl right back.
        self._updating = False

        self._build_ui()
        self._refresh()
        GLib.timeout_add(1000, self._poll)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
        root.set_border_width(24)
        self.add(root)

        self.label = Gtk.Label(label="--%")
        self.label.set_name("volumelabel")
        root.pack_start(self.label, False, False, 0)

        self.scale = Gtk.Scale.new_with_range(Gtk.Orientation.VERTICAL, 0, 100, 1)
        self.scale.set_inverted(True)
        self.scale.set_vexpand(True)
        self.scale.set_draw_value(False)
        self.scale.connect("value-changed", self._on_scale_changed)
        scale_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        scale_row.set_halign(Gtk.Align.CENTER)
        scale_row.pack_start(self.scale, True, True, 0)
        root.pack_start(scale_row, True, True, 0)

        self.mute_btn = Gtk.ToggleButton(label="Mute")
        self.mute_btn.set_name("mutebtn")
        self.mute_btn.connect("toggled", self._on_mute_toggled)
        root.pack_start(self.mute_btn, False, False, 0)

        self.error_label = Gtk.Label(label="")
        self.error_label.set_name("errorlabel")
        self.error_label.set_line_wrap(True)
        root.pack_start(self.error_label, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _apply_state(self, state):
        self._updating = True
        self.scale.set_value(state["volume"])
        self.mute_btn.set_active(state["muted"])
        self._updating = False
        self.label.set_text(f"{state['volume']}%")
        self.error_label.set_text("")

    def _refresh(self):
        try:
            self._apply_state(audioctl.get_volume())
        except audioctl.AudioError as e:
            self.error_label.set_text(str(e))

    def _poll(self):
        if not self._updating:
            self._refresh()
        return True

    def _on_scale_changed(self, scale):
        if self._updating:
            return
        try:
            state = audioctl.set_volume(scale.get_value())
        except audioctl.AudioError as e:
            self.error_label.set_text(str(e))
            return
        self.label.set_text(f"{state['volume']}%")
        self.error_label.set_text("")

    def _on_mute_toggled(self, btn):
        if self._updating:
            return
        try:
            audioctl.set_mute(btn.get_active())
            self.error_label.set_text("")
        except audioctl.AudioError as e:
            self.error_label.set_text(str(e))


def main():
    Gtk.init()
    win = VolumeWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
