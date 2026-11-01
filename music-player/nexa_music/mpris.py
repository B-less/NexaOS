"""MPRIS: the media keys (playerctl, bound in the desktop's config) and any
player controls can play, pause and skip in Music, and see what's playing.
org.mpris.MediaPlayer2.nexa_music on the session bus."""
from gi.repository import Gio, GLib

NAME = "org.mpris.MediaPlayer2.nexa_music"
PATH = "/org/mpris/MediaPlayer2"
XML = """
<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/><method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/><property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/><property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/><property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/><method name="Previous"/><method name="Pause"/><method name="PlayPause"/>
    <method name="Stop"/><method name="Play"/>
    <method name="Seek"><arg direction="in" name="Offset" type="x"/></method>
    <method name="SetPosition"><arg direction="in" name="TrackId" type="o"/><arg direction="in" name="Position" type="x"/></method>
    <method name="OpenUri"><arg direction="in" name="Uri" type="s"/></method>
    <signal name="Seeked"><arg name="Position" type="x"/></signal>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="LoopStatus" type="s" access="readwrite"/>
    <property name="Rate" type="d" access="readwrite"/>
    <property name="Shuffle" type="b" access="readwrite"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read"/>
    <property name="MinimumRate" type="d" access="read"/><property name="MaximumRate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/><property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/><property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/><property name="CanControl" type="b" access="read"/>
  </interface>
</node>
"""
ROOT, PLAYER = "org.mpris.MediaPlayer2", "org.mpris.MediaPlayer2.Player"


class Mpris:
    """`app` provides: play(), pause(), play_pause(), stop(), next(),
    previous(), seek_to(seconds), position(), status() ("Playing" /
    "Paused" / "Stopped"), current() (a track dict or None), loop()
    ("None" / "Playlist" / "Track"), set_loop(s), shuffle(),
    set_shuffle(b), volume(), set_volume(v), present(), quit()."""

    def __init__(self, app):
        self.app = app
        self.conn = None
        self.ids = []
        self.info = Gio.DBusNodeInfo.new_for_xml(XML)
        self.owner = Gio.bus_own_name(Gio.BusType.SESSION, NAME, Gio.BusNameOwnerFlags.NONE, self._on_bus, None, None)

    def _on_bus(self, conn, _name):
        self.conn = conn
        for iface in self.info.interfaces:
            self.ids.append(conn.register_object_with_closures2(PATH, iface, self._call, self._get, self._set))

    def close(self):
        if self.conn:
            for i in self.ids:
                self.conn.unregister_object(i)
        Gio.bus_unown_name(self.owner)

    # --- what it says ---

    def _metadata(self):
        t = self.app.current()
        if not t:
            return GLib.Variant("a{sv}", {"mpris:trackid": GLib.Variant("o", "/org/mpris/MediaPlayer2/TrackList/NoTrack")})
        md = {
            "mpris:trackid": GLib.Variant("o", f"/org/nexaos/Music/Track/{abs(hash(t['path'])) % (1 << 62)}"),
            "xesam:title": GLib.Variant("s", t.get("title") or ""),
            "xesam:url": GLib.Variant("s", Gio.File.new_for_path(t["path"]).get_uri()),
        }
        if t.get("artist"):
            md["xesam:artist"] = GLib.Variant("as", [t["artist"]])
        if t.get("album"):
            md["xesam:album"] = GLib.Variant("s", t["album"])
        if t.get("duration"):
            md["mpris:length"] = GLib.Variant("x", int(t["duration"] * 1_000_000))
        if t.get("cover"):
            md["mpris:artUrl"] = GLib.Variant("s", Gio.File.new_for_path(t["cover"]).get_uri())
        return GLib.Variant("a{sv}", md)

    def _value(self, iface, prop):
        a = self.app
        if iface == ROOT:
            return {
                "CanQuit": GLib.Variant("b", True), "CanRaise": GLib.Variant("b", True),
                "HasTrackList": GLib.Variant("b", False), "Identity": GLib.Variant("s", "Music"),
                "DesktopEntry": GLib.Variant("s", "nexa-music"),
                "SupportedUriSchemes": GLib.Variant("as", ["file"]),
                "SupportedMimeTypes": GLib.Variant("as", ["audio/mpeg", "audio/flac", "audio/ogg", "audio/opus", "audio/mp4", "audio/x-wav"]),
            }.get(prop)
        has = a.current() is not None
        return {
            "PlaybackStatus": GLib.Variant("s", a.status()), "LoopStatus": GLib.Variant("s", a.loop()),
            "Rate": GLib.Variant("d", 1.0), "Shuffle": GLib.Variant("b", a.shuffle()),
            "Metadata": self._metadata(), "Volume": GLib.Variant("d", a.volume()),
            "Position": GLib.Variant("x", int((a.position() or 0) * 1_000_000)),
            "MinimumRate": GLib.Variant("d", 1.0), "MaximumRate": GLib.Variant("d", 1.0),
            "CanGoNext": GLib.Variant("b", has), "CanGoPrevious": GLib.Variant("b", has),
            "CanPlay": GLib.Variant("b", has), "CanPause": GLib.Variant("b", has),
            "CanSeek": GLib.Variant("b", has), "CanControl": GLib.Variant("b", True),
        }.get(prop)

    def changed(self, *props):
        """Tell listeners that these Player properties changed."""
        if not self.conn:
            return
        values = {p: self._value(PLAYER, p) for p in props}
        self.conn.emit_signal(None, PATH, "org.freedesktop.DBus.Properties", "PropertiesChanged",
                              GLib.Variant("(sa{sv}as)", (PLAYER, values, [])))

    def seeked(self, seconds):
        if self.conn:
            self.conn.emit_signal(None, PATH, PLAYER, "Seeked", GLib.Variant("(x)", (int(seconds * 1_000_000),)))

    # --- what it's asked ---

    def _call(self, _conn, _sender, _path, iface, method, params, invocation):
        a = self.app
        acts = {"Next": a.next, "Previous": a.previous, "Pause": a.pause, "PlayPause": a.play_pause, "Stop": a.stop,
                "Play": a.play, "Raise": a.present, "Quit": a.quit}
        if method in acts:
            GLib.idle_add(lambda: acts[method]() and False)
        elif method == "Seek":
            offset = params.unpack()[0] / 1_000_000
            GLib.idle_add(lambda: a.seek_to((a.position() or 0) + offset) and False)
        elif method == "SetPosition":
            pos = params.unpack()[1] / 1_000_000
            GLib.idle_add(lambda: a.seek_to(pos) and False)
        elif method == "OpenUri":
            path = Gio.File.new_for_uri(params.unpack()[0]).get_path()
            if path:
                GLib.idle_add(lambda: a.open_paths([path]) and False)
        invocation.return_value(None)

    def _get(self, _conn, _sender, _path, iface, prop):
        return self._value(iface, prop)

    def _set(self, _conn, _sender, _path, iface, prop, value):
        v = value.unpack()
        if prop == "LoopStatus" and v in ("None", "Playlist", "Track"):
            self.app.set_loop(v)
        elif prop == "Shuffle":
            self.app.set_shuffle(bool(v))
        elif prop == "Volume":
            self.app.set_volume(max(0.0, min(1.0, float(v))))
        else:
            return False
        return True
