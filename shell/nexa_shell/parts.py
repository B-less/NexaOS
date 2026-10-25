"""The always-on desktop parts in one process: the Dock, the "For you"
card, the drives on the desktop and Nexa Link's pop-ups.

Each used to be its own Python + GTK process, about 25 MB (PSS) apiece in
the 4 GB VM (docs/MICROBRAIN.md); in one process they share the
interpreter, GTK and the theme. A part that fails to start is logged and
the others carry on. Each can still run alone (python3 -m nexa_shell dock)."""
import sys
import traceback

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gio, Gtk

from . import desktop, dock, drives, linknotify


def main():
    app = Gtk.Application(application_id="os.nexa.DesktopParts", flags=Gio.ApplicationFlags.FLAGS_NONE)
    started = {}

    def activate(application):
        if started:
            return  # already running: a second start is a no-op
        application.hold()  # the card and the pop-ups are often hidden
        for name, make in (("dock", lambda: dock.Dock(application)),
                           ("foryou", desktop.DesktopCard),
                           ("drives", drives.DesktopDrives),
                           ("linknotify", linknotify.Notifier)):
            try:
                win = make()
                if name == "dock":
                    win.show_all()
                else:
                    application.add_window(win)
                started[name] = win
            except Exception:  # one part's failure must not take the desktop down
                print(f"nexa-parts: {name} didn't start", file=sys.stderr)
                traceback.print_exc()

    app.connect("activate", activate)
    return app.run([])
