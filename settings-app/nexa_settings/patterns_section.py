"""Settings > Privacy > Patterns: switch Nexa Patterns on or off, choose
whether suggestions appear on the desktop and where, see what it has
learned per app, stop it learning from an app or suggesting one, forget,
export. The learning itself is nexa-patternd (Rust); see
nexa_common/patterns.py and docs/NEXA_PATTERNS.md."""
import json
import os
import time

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Pango

from nexa_common import desktopapps, patterns


def _label(text, *classes, wrap=True):
    lbl = Gtk.Label(label=text, xalign=0)
    lbl.set_line_wrap(wrap)
    lbl.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
    for c in classes:
        lbl.get_style_context().add_class(c)
    return lbl


class PatternsSection:
    def __init__(self, window):
        self.window = window
        self.box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.box.get_style_context().add_class("nx-card")
        self.box.pack_start(_label("Patterns", "nx-card-title"), False, False, 0)
        self.box.pack_start(_label(
            "NexaOS can learn which apps you use when and after what, on this computer only, and suggest what "
            "you'll open next - on the desktop and in Nexus. Window titles, file contents and anything you type "
            "are never recorded, nor are private browser windows. Nothing leaves this computer.", "nx-card-line"), False, False, 0)
        self._building = True
        self.enabled = self._switch_row("Learn how I use NexaOS", lambda on: self._set(enabled=on))
        self.desktop = self._switch_row("Show suggestions on the desktop", lambda on: self._set(desktop_card=on))
        pos_row = Gtk.Box(spacing=10)
        pos_row.pack_start(_label("Where on the desktop", "nx-perm-label"), True, True, 0)
        self.position = Gtk.ComboBoxText()
        for key, text in patterns.POSITIONS.items():
            self.position.append(key, text)
        self.position.connect("changed", lambda c: self._building or self._set(card_position=c.get_active_id()))
        pos_row.pack_start(self.position, False, False, 0)
        self.box.pack_start(pos_row, False, False, 0)
        self.status = _label("", "nx-card-status")
        self.box.pack_start(self.status, False, False, 0)
        self.apps_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.box.pack_start(self.apps_box, False, False, 0)
        buttons = Gtk.Box(spacing=8)
        for text, handler in (("Forget the last hour", self._forget_hour), ("Forget everything", self._forget_all),
                              ("Export…", self._export)):
            b = Gtk.Button(label=text)
            b.connect("clicked", handler)
            buttons.pack_start(b, False, False, 0)
        self.box.pack_start(buttons, False, False, 4)
        self._building = False
        self.reload()

    def _switch_row(self, text, on_change):
        row = Gtk.Box(spacing=10)
        row.get_style_context().add_class("nx-perm-row")
        row.pack_start(_label(text, "nx-perm-label"), True, True, 0)
        sw = Gtk.Switch()
        sw.set_valign(Gtk.Align.CENTER)
        sw.connect("notify::active", lambda s, _p: self._building or on_change(s.get_active()))
        row.pack_start(sw, False, False, 0)
        self.box.pack_start(row, False, False, 0)
        return sw

    def _set(self, **changes):
        try:
            patterns.update_config(**changes)
        except OSError as e:
            self.status.set_text(f"Couldn't save that: {e}")
            return
        self.reload()

    def reload(self):
        cfg = patterns.load_config()
        self._building = True
        self.enabled.set_active(cfg["enabled"])
        self.desktop.set_active(cfg["desktop_card"])
        self.desktop.set_sensitive(cfg["enabled"])
        self.position.set_active_id(cfg["card_position"] if cfg["card_position"] in patterns.POSITIONS else "top-right")
        self.position.set_sensitive(cfg["enabled"] and cfg["desktop_card"])
        self._building = False
        for child in self.apps_box.get_children():
            child.destroy()
        try:
            st = patterns.request("status", timeout=1)
            learned = patterns.request("apps", timeout=1)["apps"]
        except patterns.PatternsError:
            self.status.set_text("Patterns isn't running right now (it starts with the desktop).")
            return
        if not cfg["enabled"]:
            self.status.set_text("Off: nothing is being learned." + (f" {len(learned)} apps are still remembered - "
                                 "Forget everything to clear them." if learned else ""))
        elif not st["ready"]:
            self.status.set_text("Learning: suggestions start after a few days of use.")
        else:
            self.status.set_text(f"Learning for {st['days']} days · {st['apps']} apps known.")
        apps = desktopapps.load_apps()
        for a in learned[:15]:
            entry = patterns.resolve(a["app"], apps)
            self.apps_box.pack_start(self._app_row(a, entry["name"] if entry else a["app"]), False, False, 0)
        self.apps_box.show_all()

    def _app_row(self, a, name):
        row = Gtk.Box(spacing=8)
        row.pack_start(_label(f"{name}  ·  used {a['times']} times", "nx-perm-label", wrap=False), True, True, 0)
        no_learn = Gtk.CheckButton(label="Don't learn")
        no_learn.set_active(a["excluded"])
        no_learn.connect("toggled", lambda c: (patterns.exclude(a["app"], c.get_active()), self.reload()))
        row.pack_start(no_learn, False, False, 0)
        no_suggest = Gtk.CheckButton(label="Don't suggest")
        no_suggest.set_active(a["never_suggest"])
        no_suggest.connect("toggled", lambda c: (patterns.never_suggest(a["app"], c.get_active()), self.reload()))
        row.pack_start(no_suggest, False, False, 0)
        forget = Gtk.Button(label="Forget")
        forget.connect("clicked", lambda _b: self._forget(app=a["app"]))
        row.pack_start(forget, False, False, 0)
        return row

    def _forget(self, **what):
        try:
            patterns.request("forget", **what)
        except patterns.PatternsError as e:
            self.status.set_text(str(e))
            return
        self.reload()

    def _forget_hour(self, _b):
        self._forget(since=int(time.time()) - 3600)

    def _forget_all(self, _b):
        dialog = Gtk.MessageDialog(transient_for=self.window, flags=0, message_type=Gtk.MessageType.QUESTION,
                                   buttons=Gtk.ButtonsType.YES_NO, text="Forget everything Patterns has learned?")
        dialog.format_secondary_text("Suggestions start again from nothing.")
        ok = dialog.run() == Gtk.ResponseType.YES
        dialog.destroy()
        if ok:
            self._forget(all=True)

    def _export(self, _b):
        try:
            data = patterns.request("export", timeout=3)
        except patterns.PatternsError as e:
            self.status.set_text(str(e))
            return
        path = os.path.join(os.path.expanduser("~"), "Documents", "nexa-patterns-export.json")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            json.dump(data, f, indent=1)
        self.status.set_text(f"Saved what Patterns knows to {path}")
