"""Icons shift with the theme: with the NexaHexa sets installed, an open app
(Archive) uses NexaHexa-<theme>, and changing the theme switches it live.
Run by scripts/test-theme-live.sh."""
import os, subprocess, sys, time
sys.path[:0] = ["/src/archive", "/src/common"]
import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk
from nexa_archive.window import ArchiveWindow

def pump(s):
    end = time.time() + s
    while time.time() < end:
        while Gtk.events_pending(): Gtk.main_iteration_do(False)
        time.sleep(0.02)

win = ArchiveWindow(None); win.show_all(); pump(1.0)
settings = Gtk.Settings.get_default()
first = settings.get_property("gtk-icon-theme-name")
subprocess.run([sys.executable, "-c", "import sys; sys.path.insert(0, '/src/common'); from nexa_common import themes; themes.apply('aurora', reload=False)"], check=True)
pump(2.0)
after = settings.get_property("gtk-icon-theme-name")
info = Gtk.IconTheme.get_default().lookup_icon("nexa-files", 128, 0)
path = info.get_filename() if info else ""
ok = first == "NexaHexa-horizon" and after == "NexaHexa-aurora" and "NexaHexa-aurora" in path
print(("PASS" if ok else "FAIL") + f" icons follow the theme: {first} -> {after} ({path})")
sys.exit(0 if ok else 1)
