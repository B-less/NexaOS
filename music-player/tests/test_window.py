"""The Music window, driven like a person would, against the real engine
(build/native/nexa-musiclib) and real songs made with GStreamer: the
library (songs, albums with covers, artists), gapless playback, Up Next,
shuffle and repeat, playlists, search, MPRIS over D-Bus, a song that can't
be played, and the queue coming back after a restart.

Needs a display and a session bus: run it with scripts/test-music-app.sh."""
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time

HOME = tempfile.mkdtemp(prefix="music-home-")
os.environ["HOME"] = HOME
for v in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"):
    os.environ.pop(v, None)
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "music-player"), os.path.join(REPO, "common")]

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
gi.require_version("Gst", "1.0")
from gi.repository import GdkPixbuf, Gio, GLib, GObject, Gst, Gtk  # noqa: E402

from nexa_music import library, window as W  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def pump(seconds=0.2, until=None):
    end = time.time() + seconds
    while time.time() < end or (until and not until() and time.time() < end + 15):
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        if until and until() and time.time() >= end:
            return
        time.sleep(0.01)


def make_song(path, title, artist, album, track, enc, seconds=2.0, freq=440):
    pipe = Gst.parse_launch(f"audiotestsrc num-buffers={int(seconds * 48000 / 1024)} samplesperbuffer=1024 freq={freq} "
                            f"! audioconvert ! audioresample ! {enc} name=e ! oggmux ! filesink name=out")
    pipe.get_by_name("out").set_property("location", path)
    tags = Gst.TagList.new_empty()
    for name, value in (("title", title), ("artist", artist), ("album", album)):
        tags.add_value(Gst.TagMergeMode.REPLACE, name, value)
    tags.add_value(Gst.TagMergeMode.REPLACE, "track-number", GObject.Value(GObject.TYPE_UINT, track))
    pipe.get_by_name("e").merge_tags(tags, Gst.TagMergeMode.REPLACE)
    pipe.set_state(Gst.State.PLAYING)
    pipe.get_bus().timed_pop_filtered(20 * Gst.SECOND, Gst.MessageType.EOS | Gst.MessageType.ERROR)
    pipe.set_state(Gst.State.NULL)


M = os.path.join(HOME, "Music")
hexa = os.path.join(M, "Nexa Band", "Hexagons")
glass = os.path.join(M, "Other Artist", "Glass")
os.makedirs(hexa)
os.makedirs(glass)
for i, name in enumerate(("Song One", "Song Two", "Song Three"), 1):
    make_song(os.path.join(hexa, f"0{i} {name}.opus"), name, "Nexa Band", "Hexagons", i, "opusenc", freq=300 + 100 * i)
for i, name in enumerate(("Clear", "Frosted"), 1):
    make_song(os.path.join(glass, f"{i}.ogg"), name, "Other Artist", "Glass", i, "vorbisenc", freq=700 + 100 * i)
pix = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, False, 8, 200, 200)
pix.fill(0x4da3ffff)
pix.savev(os.path.join(hexa, "cover.png"), "png", [], [])
# Hostile: an MP3 with a tag but no playable sound (and no MP3 decoder here), an empty file.
tag = b"TIT2" + struct.pack(">I", 12) + b"\0\0" + b"\0Broken Song"
open(os.path.join(M, "broken.mp3"), "wb").write(b"ID3\x03\0\0" + bytes([0, 0, 0, len(tag)]) + tag + b"\xff\xfb\x90\0" + bytes(413) * 3)
open(os.path.join(M, "empty.flac"), "wb").close()

win = W.MusicWindow([])
silent = Gst.ElementFactory.make("fakesink")
silent.set_property("sync", True)
win.player.playbin.set_property("audio-sink", silent)
win.show_all()
pump(1.0, until=lambda: len(win.tracks) == 7)
titles = sorted(t["title"] for t in win.tracks)
check("the library lists every song (tags read by the engine)", titles == ["Broken Song", "Clear", "Frosted", "Song One", "Song Three", "Song Two", "empty"], str(titles))
song1 = win.by_path[os.path.join(hexa, "01 Song One.opus")]
check("lengths read", abs((song1.get("duration") or 0) - 2.0) < 0.2, str(song1.get("duration")))
check("the folder's cover.png is the album's cover", song1.get("cover") == os.path.join(hexa, "cover.png"))
check("albums: Hexagons, Glass, and the loose songs", sorted(k[1] for k in win.albums) == ["Glass", "Hexagons", "Other songs"], str(list(win.albums)))
check("artists listed", len(win.artists_list.get_children()) == 3)
check("the status says how much music there is", win.status_label.get_text() == "7 songs, 2 albums", win.status_label.get_text())

# An album: its page, played in track order, gapless.
win.show_album(("Nexa Band", "Hexagons"))
check("the album's page lists its songs in order", [r[1] for r in win.detail_store] == ["Song One", "Song Two", "Song Three"])
started = []
orig = win._on_next_started
win._on_next_started = lambda: (started.append(win.pos), orig())
win.player.on_next_started = win._on_next_started
win.play_list(win.detail_songs, 0)
pump(1.0, until=lambda: win.status() == "Playing")
check("Play: playing the first song", win.status() == "Playing" and win.now_title.get_text() == "Song One")
pump(0.1, until=lambda: win.pos == 1)  # each song is 2 s
check("the next song follows without a gap", win.pos == 1 and started and win.now_title.get_text() == "Song Two", f"pos {win.pos}, gapless {started}")
win.next()
pump(0.5)
check("Next", win.pos == 2 and win.now_title.get_text() == "Song Three")
win.previous()
pump(0.5)
check("Previous", win.pos == 1)

# Repeat and shuffle.
win.set_repeat("one")
check("repeat one: the same song again", win._next_index() == win.pos)
win.set_repeat("all")
win.pos = 2
check("repeat all: back to the start after the last", win._next_index() == 0)
win.set_repeat("off")
check("repeat off: stops after the last", win._next_index() is None)
win.play_index(0)
before = list(win.queue)
win.set_shuffle(True)
check("shuffle keeps the song playing first and every song", win.queue[0] == before[0] and sorted(win.queue) == sorted(before) and win.pos == 0)
win.set_shuffle(False)
check("shuffle off: the order comes back", win.queue == before and win.pos == 0)

# Up Next.
clear = win.by_path[os.path.join(glass, "1.ogg")]
frosted = win.by_path[os.path.join(glass, "2.ogg")]
win.play_next([clear])
check("Play Next puts it right after this song", win.queue[win.pos + 1] == clear["path"])
win.add_to_queue([frosted])
check("Add to Up Next puts it at the end", win.queue[-1] == frosted["path"])
win.move_in_queue(frosted["path"], -1)
check("moving a song up", win.queue[-2] == frosted["path"])
win.remove_from_queue([clear])
check("removing a song", clear["path"] not in win.queue)
win.nav.select_row(win.nav.get_row_at_index(4))
pump(0.2)
check("the Up Next page shows the queue, this song in bold", len(win.queue_store) == len(win.queue) and win.queue_store[win.pos][6] == 700)

# Playlists.
check("a new playlist", win.save_playlist("Road trip", win.albums[("Nexa Band", "Hexagons")]))
pl = os.path.join(HOME, ".local/share/nexa-music/playlists/Road trip.m3u")
check("saved as an .m3u file", os.path.exists(pl) and open(pl).read().count(".opus") == 3)
win.add_to_playlist("Road trip", [frosted])
check("adding a song to it", [t["title"] for t in win._playlist_songs("Road trip")][-1] == "Frosted")
library.run("playlist_rename", name="Road trip", to="Long drive")
win._fill_playlists()
check("renaming it", win.playlist_names == ["Long drive"])
win.show_playlist("Long drive")
check("its page lists its songs", len(win.detail_store) == 4)

# Search.
win.search.set_text("glass")
pump(0.3)
visible = [c.key[1] for c in win.albums_box.get_children() if win._album_visible(c)]
check("search finds albums by name", visible == ["Glass"], str(visible))
win.search.set_text("")

# MPRIS: a second connection plays the media keys' part.
addr = Gio.dbus_address_get_for_bus_sync(Gio.BusType.SESSION, None)
other = Gio.DBusConnection.new_for_address_sync(addr, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
pump(0.5)


def call(iface, method, args=None):
    box = []
    other.call("org.mpris.MediaPlayer2.nexa_music", "/org/mpris/MediaPlayer2", iface, method, args, None, Gio.DBusCallFlags.NONE, 3000, None,
               lambda c, r: box.append(c.call_finish(r)))
    pump(0.1, until=lambda: box)
    return box[0] if box else None


meta = call("org.freedesktop.DBus.Properties", "Get", GLib.Variant("(ss)", ("org.mpris.MediaPlayer2.Player", "Metadata")))
title = meta.unpack()[0].get("xesam:title") if meta else None
check("MPRIS: what's playing can be seen", title == win.current()["title"], str(title))
call("org.mpris.MediaPlayer2.Player", "PlayPause")
pump(0.8, until=lambda: win.status() == "Paused")
check("MPRIS: the play/pause key pauses", win.status() == "Paused", win.status())
call("org.mpris.MediaPlayer2.Player", "Next")
pump(0.8)
check("MPRIS: the next key goes to the next song", win.pos == 1, str(win.pos))

# A song that can't be played.
broken = win.by_path[os.path.join(M, "broken.mp3")]
win.play_list([broken, clear], 0)
pump(2.0, until=lambda: "can't be played" in win.status_label.get_text())
check("a song that can't be played: said, and it goes on", "Broken Song can't be played" in win.status_label.get_text() and win.pos == 1, win.status_label.get_text())

# An .m3u opened from Files.
m3u = os.path.join(HOME, "mix.m3u")
open(m3u, "w").write("#EXTM3U\nMusic/Other Artist/Glass/2.ogg\nMusic/Nexa Band/Hexagons/02 Song Two.opus\n")
win.open_paths([m3u])
pump(0.5)
check("an .m3u file plays its songs", [os.path.basename(p) for p in win.queue] == ["2.ogg", "02 Song Two.opus"], str(win.queue))

# Closing and coming back.
win.play_index(1)
pump(1.2)
win.player.seek(1.0)
pump(0.5)
saved_at = win.player.get_position()
win._save_state()
win.player.stop()
win.destroy()
win2 = W.MusicWindow([])
silent2 = Gst.ElementFactory.make("fakesink")
silent2.set_property("sync", True)
win2.player.playbin.set_property("audio-sink", silent2)
win2.show_all()
pump(1.5, until=lambda: win2.tracks and win2.status() == "Paused")
check("next time: Up Next and the song come back, paused", [os.path.basename(p) for p in win2.queue] == ["2.ogg", "02 Song Two.opus"]
      and win2.pos == 1 and win2.status() == "Paused", f"{win2.queue} {win2.pos} {win2.status()}")
pump(1.0)
check("... at the same place in the song", abs((win2.player.get_position() or 0) - saved_at) < 0.2, f"{win2.player.get_position()} vs {saved_at}")

win2.nav.select_row(win2.nav.get_row_at_index(1))
win2.play_index(1)
pump(1.0)
subprocess.run(["grim", os.environ.get("SHOT", "/tmp/music.png")], check=False)
win2.player.stop()
shutil.rmtree(HOME, ignore_errors=True)
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
