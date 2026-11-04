"""A stand-in app for Orbit's test: one window with the given app id.
    python3 fakeapp.py APP_ID [busy]   (busy: keeps one core working)"""
import sys
import threading

from gi.repository import GLib
GLib.set_prgname(sys.argv[1])  # the Wayland app_id
import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk

if "busy" in sys.argv[2:]:
    def spin():
        x = 0
        while True:
            x += 1
    threading.Thread(target=spin, daemon=True).start()
w = Gtk.Window(title=sys.argv[1])
w.set_default_size(420, 300)
w.connect("destroy", Gtk.main_quit)
w.show_all()
Gtk.main()
