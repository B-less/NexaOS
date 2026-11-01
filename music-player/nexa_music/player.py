"""GStreamer playback: play, pause, seek, volume, and gapless: the next song
is handed to playbin while the current one is ending (about-to-finish), so
there's no silence between them."""
import os
import threading

import gi
gi.require_version("Gst", "1.0")
from gi.repository import GLib, Gst

Gst.init(None)


def uri_of(path):
    return path if "://" in path else Gst.filename_to_uri(os.path.abspath(path))


class Player:
    def __init__(self, on_state_changed=None, on_eos=None, on_error=None, on_next_started=None):
        self.playbin = Gst.ElementFactory.make("playbin", "player")
        # Audio only: no video window for a song with a cover picture.
        flags = self.playbin.get_property("flags")
        self.playbin.set_property("flags", flags & ~(1 << 0) & ~(1 << 2))
        bus = self.playbin.get_bus()
        bus.add_signal_watch()
        bus.connect("message::eos", self._on_eos)
        bus.connect("message::error", self._on_error)
        bus.connect("message::state-changed", self._on_state_changed)
        bus.connect("message::stream-start", self._on_stream_start)
        self.playbin.connect("about-to-finish", self._on_about_to_finish)
        self.on_state_changed = on_state_changed
        self.on_eos = on_eos
        self.on_error = on_error
        self.on_next_started = on_next_started
        self.uri = None
        self._lock = threading.Lock()
        self._next = None       # the song to go on to without a gap
        self._switched = False  # playbin took _next; stream-start will say when it plays

    def set_next(self, path):
        with self._lock:
            self._next = uri_of(path) if path else None

    def load(self, path):
        self.stop()
        self.uri = uri_of(path)
        self.playbin.set_property("uri", self.uri)

    def play(self):
        self.playbin.set_state(Gst.State.PLAYING)

    def pause(self):
        self.playbin.set_state(Gst.State.PAUSED)

    def stop(self):
        self.playbin.set_state(Gst.State.NULL)
        with self._lock:
            self._switched = False

    def state(self):
        return self.playbin.get_state(0)[1]

    def set_volume(self, vol):
        self.playbin.set_property("volume", max(0.0, min(1.0, vol)))

    def get_volume(self):
        return self.playbin.get_property("volume")

    def seek(self, seconds):
        # Exact, not to the nearest key frame: cheap for sound, and a song
        # comes back where it was left.
        self.playbin.seek_simple(Gst.Format.TIME, Gst.SeekFlags.FLUSH | Gst.SeekFlags.ACCURATE, int(max(0, seconds) * Gst.SECOND))

    def get_position(self):
        ok, pos = self.playbin.query_position(Gst.Format.TIME)
        return pos / Gst.SECOND if ok else None

    def get_duration(self):
        ok, dur = self.playbin.query_duration(Gst.Format.TIME)
        return dur / Gst.SECOND if ok and dur > 0 else None

    def _on_about_to_finish(self, playbin):
        # GStreamer's own thread: only hand over the next song here.
        with self._lock:
            if self._next:
                playbin.set_property("uri", self._next)
                self.uri = self._next
                self._next = None
                self._switched = True

    def _on_stream_start(self, _bus, _msg):
        with self._lock:
            switched, self._switched = self._switched, False
        if switched and self.on_next_started:
            self.on_next_started()

    def _on_eos(self, _bus, _msg):
        if self.on_eos:
            self.on_eos()

    def _on_error(self, _bus, msg):
        err, _debug = msg.parse_error()
        if self.on_error:
            self.on_error(err.message if hasattr(err, "message") else str(err))

    def _on_state_changed(self, _bus, msg):
        if msg.src == self.playbin and self.on_state_changed:
            _old, new, _pending = msg.parse_state_changed()
            self.on_state_changed(new)
