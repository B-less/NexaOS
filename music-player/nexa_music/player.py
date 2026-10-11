import os

import gi
gi.require_version("Gst", "1.0")
from gi.repository import Gst

Gst.init(None)


class Player:
    def __init__(self, on_state_changed=None, on_eos=None, on_error=None):
        self.playbin = Gst.ElementFactory.make("playbin", "player")
        bus = self.playbin.get_bus()
        bus.add_signal_watch()
        bus.connect("message::eos", self._on_eos)
        bus.connect("message::error", self._on_error)
        bus.connect("message::state-changed", self._on_state_changed)
        self.on_state_changed = on_state_changed
        self.on_eos = on_eos
        self.on_error = on_error
        self.uri = None

    def load(self, path):
        self.stop()
        self.uri = path if "://" in path else Gst.filename_to_uri(os.path.abspath(path))
        self.playbin.set_property("uri", self.uri)

    def play(self):
        self.playbin.set_state(Gst.State.PLAYING)

    def pause(self):
        self.playbin.set_state(Gst.State.PAUSED)

    def stop(self):
        self.playbin.set_state(Gst.State.NULL)

    def set_volume(self, vol):
        self.playbin.set_property("volume", max(0.0, min(1.0, vol)))

    def get_volume(self):
        return self.playbin.get_property("volume")

    def seek(self, seconds):
        self.playbin.seek_simple(
            Gst.Format.TIME, Gst.SeekFlags.FLUSH | Gst.SeekFlags.KEY_UNIT, int(seconds * Gst.SECOND),
        )

    def get_position(self):
        ok, pos = self.playbin.query_position(Gst.Format.TIME)
        return pos / Gst.SECOND if ok else None

    def get_duration(self):
        ok, dur = self.playbin.query_duration(Gst.Format.TIME)
        return dur / Gst.SECOND if ok else None

    def _on_eos(self, _bus, _msg):
        if self.on_eos:
            self.on_eos()

    def _on_error(self, _bus, msg):
        err, _debug = msg.parse_error()
        if self.on_error:
            self.on_error(str(err))

    def _on_state_changed(self, _bus, msg):
        if msg.src == self.playbin and self.on_state_changed:
            _old, new, _pending = msg.parse_state_changed()
            self.on_state_changed(new)
