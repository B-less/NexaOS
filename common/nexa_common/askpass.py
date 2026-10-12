"""Password prompt for elevate.run_interactive: a dialog in the desktop
(safe to call from a worker thread; the dialog itself always runs on the
GTK main thread), or a terminal prompt when there is no display."""
import getpass
import os
import threading


def ask(reason, parent=None):
    """Returns ask(retry) -> password or None, for elevate.run_interactive."""
    def _ask(retry):
        if not (os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY")):
            try:
                return getpass.getpass(("Wrong password. " if retry else "") + reason + " Password: ")
            except (EOFError, KeyboardInterrupt):
                return None
        import gi
        gi.require_version("Gtk", "3.0")
        from gi.repository import GLib, Gtk
        result = {}
        done = threading.Event()

        def show():
            dialog = Gtk.Dialog(title="Password required", transient_for=parent, modal=True)
            dialog.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Continue", Gtk.ResponseType.OK)
            dialog.set_default_response(Gtk.ResponseType.OK)
            box = dialog.get_content_area()
            box.set_spacing(10)
            box.set_border_width(16)
            title = Gtk.Label(xalign=0)
            title.set_markup("<b>Wrong password, try again.</b>" if retry else "<b>Your password is needed</b>")
            box.pack_start(title, False, False, 0)
            why = Gtk.Label(label=reason, xalign=0)
            why.set_line_wrap(True)
            box.pack_start(why, False, False, 0)
            entry = Gtk.Entry()
            entry.set_visibility(False)
            entry.set_input_purpose(Gtk.InputPurpose.PASSWORD)
            entry.set_activates_default(True)
            box.pack_start(entry, False, False, 0)
            dialog.show_all()
            response = dialog.run()
            result["password"] = entry.get_text() if response == Gtk.ResponseType.OK else None
            dialog.destroy()
            done.set()
            return False

        if threading.current_thread() is threading.main_thread():
            show()
        else:
            GLib.idle_add(show)
            done.wait()
        return result.get("password")

    return _ask
