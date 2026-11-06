"""A stand-in window for Constellation's test.
    python3 star.py APP_ID TITLE [--child DIR] [--hold FILE]
--child: a child process working in DIR (as a terminal's shell would);
--hold: keeps FILE open (as an app holding its document)."""
import subprocess
import sys

from gi.repository import GLib
GLib.set_prgname(sys.argv[1])  # the Wayland app_id
import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk

args = sys.argv[3:]
child = held = None
if "--child" in args:
    child = subprocess.Popen(["sleep", "600"], cwd=args[args.index("--child") + 1])
if "--hold" in args:
    held = open(args[args.index("--hold") + 1], "rb")
w = Gtk.Window(title=sys.argv[2])
w.set_default_size(420, 300)
w.connect("destroy", Gtk.main_quit)
w.show_all()
Gtk.main()
if child:
    child.kill()
