"""Nexa Link pieces shared by Settings and Files: reading nexa-linkd's
progress lines, sending files one after another, and the "Send to…"
picker (paired devices nearby, and devices taking drops)."""
import json
import os

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk

from . import link


def watch_lines(proc, on_event):
    """Call on_event(dict) for each JSON line `proc` prints; on_event with
    {"event": "ended"} (and its error, if any) when it ends."""
    def ready(source, condition):
        line = source.readline() if condition & GLib.IO_IN else ""
        if line:
            try:
                on_event(json.loads(line))
            except ValueError:
                pass
            return True
        err = (proc.stderr.read() or "").strip() if proc.stderr else ""
        on_event({"event": "ended", "error": err.removeprefix("nexa-linkd: ")})
        return False
    GLib.io_add_watch(proc.stdout, GLib.PRIORITY_DEFAULT, GLib.IO_IN | GLib.IO_HUP, ready)


def send_files(target, name, files, say):
    """Send each file in turn; say(text, error=False) tells how it goes."""
    queue = list(files)

    def next_one():
        if not queue:
            return
        path = queue.pop(0)
        base = os.path.basename(path)
        try:
            proc = link.start("send", target, path)
        except OSError as e:
            say(f"Couldn't send {base}: {e}", True)
            return

        def on_event(ev):
            kind = ev.get("event")
            if kind == "waiting":
                say(f"Waiting for “{name}” to accept {base}…")
            elif kind == "sent":
                say(f"Sent {base} to “{name}”.")
            elif kind == "declined":
                say(f"“{name}” declined {base}.", True)
            elif kind == "ended":
                if ev.get("error"):
                    say(f"{base}: {ev['error']}", True)
                next_one()
        watch_lines(proc, on_event)
    next_one()


def targets():
    """Where files can go now: (target, name) for each paired device that's
    nearby, then each device taking drops."""
    out = [(d["id"], d["name"]) for d in link.devices() if d.get("online")]
    out += [(n["addr"], f"Nexa device at {n['addr']}") for n in link.nearby() if n.get("open_drop")]
    return out


def pick_device(parent, count):
    """Ask where to send `count` files; (target, name) or None."""
    try:
        choices = targets()
    except link.LinkError as e:
        choices, problem = [], str(e)
    else:
        problem = None
    dialog = Gtk.Dialog(title="Send to a device", transient_for=parent, modal=True)
    dialog.set_default_size(360, -1)
    box = dialog.get_content_area()
    box.set_spacing(10)
    box.set_border_width(16)
    what = "this file" if count == 1 else f"these {count} files"
    intro = Gtk.Label(label=f"Send {what} with Nexa Link to:", xalign=0)
    box.pack_start(intro, False, False, 0)
    listbox = Gtk.ListBox()
    for target, name in choices:
        row = Gtk.ListBoxRow()
        label = Gtk.Label(label=name, xalign=0)
        label.set_margin_top(6)
        label.set_margin_bottom(6)
        label.set_margin_start(6)
        row.add(label)
        row.target = (target, name)
        listbox.add(row)
    if choices:
        listbox.select_row(listbox.get_row_at_index(0))
        box.pack_start(listbox, False, False, 0)
    else:
        none = Gtk.Label(label=problem or "No device is nearby. Pair one in Settings > Nexa Link, "
                                          "or ask the other person to turn on Nexa Drop.", xalign=0)
        none.set_line_wrap(True)
        none.set_max_width_chars(44)
        box.pack_start(none, False, False, 0)
    dialog.add_button("Cancel", Gtk.ResponseType.CANCEL)
    send = dialog.add_button("Send", Gtk.ResponseType.OK)
    send.set_sensitive(bool(choices))
    listbox.connect("row-activated", lambda *_a: dialog.response(Gtk.ResponseType.OK))
    dialog.show_all()
    chosen = None
    if dialog.run() == Gtk.ResponseType.OK and listbox.get_selected_row() is not None:
        chosen = listbox.get_selected_row().target
    dialog.destroy()
    return chosen
