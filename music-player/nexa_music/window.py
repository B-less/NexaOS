import os

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Gst", "1.0")
from gi.repository import Gtk, Gdk, GLib, Gst

from .player import Player
from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#transport { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 20px; padding: 6px 14px; font-size: 13pt; }
#transport:hover { background-image: none; background-color: @nexa_hover; }
#playbtn { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 22px; padding: 6px 16px; font-size: 15pt; font-weight: bold; }
#playbtn:hover { background-image: none; background-color: @nexa_accent_hover; }
#nowplaying { color: @nexa_text; font-weight: bold; }
#timelabel { color: @nexa_muted; font-size: 9pt; }
treeview { background-image: none; background-color: @nexa_bg; color: @nexa_text; }
treeview:selected { background-image: none; background-color: @nexa_selected; color: @nexa_selected_text; }
#playerbar { background-image: none; background-color: @nexa_surface; }
"""

AUDIO_EXTS = (".mp3", ".wav", ".ogg", ".flac", ".m4a", ".opus")


class MusicWindow(Gtk.Window):
    def __init__(self, paths=None):
        super().__init__(title="Nexa Music")
        self.set_icon_name("nexa-music")
        attach_titlebar(self, "Nexa Music")
        self.set_default_size(640, 520)
        self.player = Player(
            on_state_changed=self._on_player_state, on_eos=self._on_eos, on_error=self._on_player_error,
        )
        self.current_index = -1
        self._seeking = False

        self._build_ui()
        self.connect("destroy", lambda *_: self.player.stop())
        GLib.timeout_add(500, self._tick)

        for p in paths or []:
            self._add_path(p)
        if paths:
            self._play_index(0)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        for label, handler in (
            ("Open Files", self._on_open_files), ("Open Folder", self._on_open_folder),
            ("Clear", self._on_clear),
        ):
            btn = Gtk.Button(label=label)
            btn.set_name("toolbtn")
            btn.connect("clicked", handler)
            toolbar.pack_start(btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.store = Gtk.ListStore(str, str)
        self.view = Gtk.TreeView(model=self.store)
        self.view.set_headers_visible(False)
        self.view.connect("row-activated", self._on_row_activated)
        col = Gtk.TreeViewColumn("Track", Gtk.CellRendererText(), text=0)
        col.set_expand(True)
        self.view.append_column(col)
        scroller.add(self.view)
        root.pack_start(scroller, True, True, 0)

        player_bar = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        player_bar.set_name("playerbar")
        player_bar.set_border_width(10)

        self.now_playing = Gtk.Label(label="Nothing playing")
        self.now_playing.set_name("nowplaying")
        self.now_playing.set_xalign(0)
        player_bar.pack_start(self.now_playing, False, False, 0)

        seek_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        self.elapsed_label = Gtk.Label(label="0:00")
        self.elapsed_label.set_name("timelabel")
        self.seek = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 1, 1)
        self.seek.set_draw_value(False)
        self.seek.connect("button-press-event", self._on_seek_start)
        self.seek.connect("button-release-event", self._on_seek_end)
        self.duration_label = Gtk.Label(label="0:00")
        self.duration_label.set_name("timelabel")
        seek_row.pack_start(self.elapsed_label, False, False, 0)
        seek_row.pack_start(self.seek, True, True, 0)
        seek_row.pack_start(self.duration_label, False, False, 0)
        player_bar.pack_start(seek_row, False, False, 0)

        controls = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        controls.set_halign(Gtk.Align.CENTER)
        # Icons from the icon theme, not characters like "⏮": those depend on
        # the font and showed up as empty boxes.
        self.prev_btn = self._transport_btn("media-skip-backward-symbolic", "Previous", self._on_prev)
        self.play_btn = self._transport_btn("media-playback-start-symbolic", "Play", self._on_play_pause, playbtn=True)
        self.stop_btn = self._transport_btn("media-playback-stop-symbolic", "Stop", self._on_stop)
        self.next_btn = self._transport_btn("media-skip-forward-symbolic", "Next", self._on_next)
        for b in (self.prev_btn, self.play_btn, self.stop_btn, self.next_btn):
            controls.pack_start(b, False, False, 0)

        vol_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        vol_label = Gtk.Image.new_from_icon_name("audio-volume-high-symbolic", Gtk.IconSize.BUTTON)
        self.volume = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 100, 1)
        self.volume.set_value(80)
        self.volume.set_draw_value(False)
        self.volume.set_size_request(120, -1)
        self.volume.connect("value-changed", self._on_volume_changed)
        self.player.set_volume(0.8)
        vol_box.pack_start(vol_label, False, False, 0)
        vol_box.pack_start(self.volume, False, False, 0)

        bottom_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        bottom_row.pack_start(Gtk.Box(), True, True, 0)
        bottom_row.set_center_widget(controls)
        bottom_row.pack_end(vol_box, False, False, 0)
        player_bar.pack_start(bottom_row, False, False, 4)

        root.pack_start(player_bar, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _transport_btn(self, icon, tooltip, handler, playbtn=False):
        btn = Gtk.Button()
        btn.set_image(Gtk.Image.new_from_icon_name(icon, Gtk.IconSize.LARGE_TOOLBAR if playbtn else Gtk.IconSize.BUTTON))
        btn.set_tooltip_text(tooltip)
        btn.set_name("playbtn" if playbtn else "transport")
        btn.connect("clicked", handler)
        return btn

    def _show_playing(self, playing):
        icon = "media-playback-pause-symbolic" if playing else "media-playback-start-symbolic"
        self.play_btn.set_image(Gtk.Image.new_from_icon_name(icon, Gtk.IconSize.LARGE_TOOLBAR))
        self.play_btn.set_tooltip_text("Pause" if playing else "Play")

    def _add_path(self, path):
        if os.path.isdir(path):
            for name in sorted(os.listdir(path)):
                full = os.path.join(path, name)
                if os.path.isfile(full) and full.lower().endswith(AUDIO_EXTS):
                    self.store.append([name, full])
        elif os.path.isfile(path) and path.lower().endswith(AUDIO_EXTS):
            self.store.append([os.path.basename(path), path])

    def _on_open_files(self, _btn):
        dialog = Gtk.FileChooserDialog(
            title="Open Files", transient_for=self, action=Gtk.FileChooserAction.OPEN,
        )
        dialog.set_select_multiple(True)
        dialog.add_buttons(Gtk.STOCK_CANCEL, Gtk.ResponseType.CANCEL, Gtk.STOCK_OPEN, Gtk.ResponseType.OK)
        if dialog.run() == Gtk.ResponseType.OK:
            was_empty = len(self.store) == 0
            for f in dialog.get_filenames():
                self._add_path(f)
            if was_empty and len(self.store):
                self._play_index(0)
        dialog.destroy()

    def _on_open_folder(self, _btn):
        dialog = Gtk.FileChooserDialog(
            title="Open Folder", transient_for=self, action=Gtk.FileChooserAction.SELECT_FOLDER,
        )
        dialog.add_buttons(Gtk.STOCK_CANCEL, Gtk.ResponseType.CANCEL, "Select", Gtk.ResponseType.OK)
        if dialog.run() == Gtk.ResponseType.OK:
            was_empty = len(self.store) == 0
            self._add_path(dialog.get_filename())
            if was_empty and len(self.store):
                self._play_index(0)
        dialog.destroy()

    def _on_clear(self, _btn):
        self.player.stop()
        self.store.clear()
        self.current_index = -1
        self.now_playing.set_text("Nothing playing")
        self._show_playing(False)

    def _on_row_activated(self, _view, tree_path, _column):
        self._play_index(tree_path.get_indices()[0])

    def _play_index(self, index):
        if index < 0 or index >= len(self.store):
            return
        self.current_index = index
        name, path = self.store[index][0], self.store[index][1]
        self.player.load(path)
        self.player.play()
        self.now_playing.set_text(name)
        self.view.set_cursor(Gtk.TreePath(index))

    def _on_play_pause(self, _btn):
        if self.current_index < 0:
            if len(self.store):
                self._play_index(0)
            return
        state = self.player.playbin.get_state(0)[1]
        if state == Gst.State.PLAYING:
            self.player.pause()
        else:
            self.player.play()

    def _on_stop(self, _btn):
        self.player.stop()
        self._show_playing(False)

    def _on_prev(self, _btn):
        if self.current_index > 0:
            self._play_index(self.current_index - 1)

    def _on_next(self, _btn):
        if self.current_index + 1 < len(self.store):
            self._play_index(self.current_index + 1)
        else:
            self.player.stop()

    def _on_volume_changed(self, scale):
        self.player.set_volume(scale.get_value() / 100.0)

    def _on_seek_start(self, _scale, _event):
        self._seeking = True

    def _on_seek_end(self, _scale, _event):
        self.player.seek(self.seek.get_value())
        self._seeking = False

    def _on_player_state(self, state):
        GLib.idle_add(self._apply_player_state, state)

    def _apply_player_state(self, state):
        self._show_playing(state == Gst.State.PLAYING)
        return False

    def _on_eos(self):
        GLib.idle_add(self._on_next, None)

    def _on_player_error(self, message):
        GLib.idle_add(self._show_error, message)

    def _show_error(self, message):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.ERROR,
            buttons=Gtk.ButtonsType.OK, text=f"Playback error: {message}",
        )
        dialog.run()
        dialog.destroy()
        return False

    def _tick(self):
        if not self._seeking:
            pos = self.player.get_position()
            dur = self.player.get_duration()
            if pos is not None and dur is not None and dur > 0:
                self.seek.set_range(0, dur)
                self.seek.set_value(pos)
                self.elapsed_label.set_text(_fmt_time(pos))
                self.duration_label.set_text(_fmt_time(dur))
        return True


def _fmt_time(seconds):
    seconds = int(seconds)
    return f"{seconds // 60}:{seconds % 60:02d}"


def main(argv):
    Gst.init(None)
    Gtk.init()
    win = MusicWindow(argv)
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
