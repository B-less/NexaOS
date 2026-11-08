"""Settings > Assistant > Voice: the speech models (whisper hears, Kokoro
speaks; both downloaded through nexa-mindd and checked against their
SHA-256), the voice, its speed and the helix's colours. Changes are saved at
once; nexa-voice picks them up while running."""
import shutil
import subprocess

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib

from nexa_common import mind, voiceconf

MODELS = (
    ("whisper-base.en", "Hearing", "turns what you say into text"),
    ("kokoro-82m", "Voice", "how Nexa sounds"),
)
SAMPLE = "Hi, I'm Nexa. This is how I sound when I answer you."


class VoiceSection(Gtk.Box):
    def __init__(self, field_label, set_label):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self._set_label = set_label
        self.pack_start(field_label("Voice: talk to Nexa, and hear it answer"), False, False, 0)
        intro = Gtk.Label(label="Press the microphone in the assistant (or Ctrl+M) and speak. Both models run on "
                                "this computer; what you say never leaves it.", xalign=0)
        intro.set_line_wrap(True)
        intro.set_name("value")
        self.pack_start(intro, False, False, 0)

        self.rows = {}
        for name, title, what in MODELS:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
            label = Gtk.Label(label=f"{title}: {what}", xalign=0)
            label.set_line_wrap(True)
            row.pack_start(label, True, True, 0)
            button = Gtk.Button(label="Download")
            button.set_name("actionbtn")
            button.set_no_show_all(True)
            button.connect("clicked", self._download, name)
            row.pack_start(button, False, False, 0)
            self.pack_start(row, False, False, 0)
            bar = Gtk.ProgressBar()
            bar.set_no_show_all(True)
            self.pack_start(bar, False, False, 0)
            self.rows[name] = (title, what, label, button, bar)

        conf = voiceconf.load()
        grid = Gtk.Grid(column_spacing=12, row_spacing=6)
        self.voice_combo = Gtk.ComboBoxText()
        for key, label in voiceconf.VOICES:
            self.voice_combo.append(key, label)
        self.voice_combo.set_active_id(conf["voice"])
        self.voice_combo.connect("changed", lambda c: self._save(voice=c.get_active_id()))
        self.speed = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0.7, 1.5, 0.05)
        self.speed.set_value(conf["speed"])
        self.speed.set_hexpand(True)
        self.speed.add_mark(1.0, Gtk.PositionType.BOTTOM, None)
        self.speed.connect("value-changed", lambda s: self._save(speed=s.get_value()))
        self.colours_combo = Gtk.ComboBoxText()
        for key, label in voiceconf.COLOURS:
            self.colours_combo.append(key, label)
        self.colours_combo.set_active_id(conf["colours"])
        self.colours_combo.connect("changed", lambda c: self._save(colours=c.get_active_id()))
        for i, (text, widget) in enumerate((("Voice", self.voice_combo), ("Speed", self.speed),
                                            ("Helix colours", self.colours_combo))):
            grid.attach(Gtk.Label(label=text, xalign=0), 0, i, 1, 1)
            grid.attach(widget, 1, i, 1, 1)
        self.pack_start(grid, False, False, 4)

        self.try_button = Gtk.Button(label="Try the voice")
        self.try_button.set_name("actionbtn")
        self.try_button.set_halign(Gtk.Align.START)
        self.try_button.connect("clicked", self._try)
        self.pack_start(self.try_button, False, False, 0)
        self.status = Gtk.Label(label="", xalign=0)
        self.status.set_line_wrap(True)
        self.pack_start(self.status, False, False, 0)
        self.refresh()
        self._timer = GLib.timeout_add_seconds(2, self._tick)
        self.connect("destroy", lambda _w: GLib.source_remove(self._timer))

    def _tick(self):
        if self.get_mapped():
            self.refresh()
        return True

    def refresh(self):
        st = mind.status()
        catalogue = {m["name"]: m for m in mind.models()} if st else {}
        installed = set((st or {}).get("installed") or [])
        downloads = (st or {}).get("downloads") or {}
        for name, (title, what, label, button, bar) in self.rows.items():
            size = catalogue.get(name, {}).get("size", 0) / 1e6
            dl = downloads.get(name)
            if st is None:
                label.set_text(f"{title}: the Nexa Mind service isn't running.")
                button.hide()
                bar.hide()
            elif name in installed:
                label.set_text(f"{title}: ready ({what}).")
                button.hide()
                bar.hide()
            elif dl and dl.get("phase") in ("Downloading", "Verifying"):
                total = max(dl.get("total") or 1, 1)
                bar.set_fraction(min(dl.get("bytes", 0) / total, 1.0))
                bar.show()
                verb = "checking the download" if dl["phase"] == "Verifying" else "downloading"
                label.set_text(f"{title}: {verb}, {dl.get('bytes', 0) / 1e6:.0f} of {total / 1e6:.0f} MB.")
                button.hide()
            else:
                failed = dl["phase"].get("Failed") if dl and isinstance(dl.get("phase"), dict) else None
                label.set_text(f"{title}: {what}. " + (f"Last download failed: {failed}." if failed else "Not downloaded yet."))
                button.set_label(f"Download ({size:.0f} MB)")
                button.show()
                bar.hide()
        return False

    def _download(self, _button, name):
        try:
            mind.download(name)
        except (mind.MindError, mind.MindUnreachable) as e:
            self._set_label(self.status, f"Couldn't start the download: {e}", error=True)
            return
        self.refresh()

    def _save(self, **change):
        voiceconf.save(**change)
        self._set_label(self.status, "Saved.", error=False)

    def _try(self, _button):
        prog = shutil.which("nexa-voice") or "/usr/local/bin/nexa-voice"
        try:
            subprocess.Popen([prog, "say", SAMPLE], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except OSError:
            self._set_label(self.status, "nexa-voice isn't installed.", error=True)
