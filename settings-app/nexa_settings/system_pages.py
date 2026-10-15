"""Settings > Users, Updates and Startup. The work is done by the system
broker (nexa-brokerd, Rust): these pages only show what is there and ask.
Every request runs off the GTK thread; the broker asks for the password
itself when an action needs one (we show the prompt)."""
import threading
import time

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk, Pango

from nexa_common import accounts, actionui, askpass, broker

CSS = b"""
.nx-intro { color: @nexa_muted; }
.nx-tag { color: @nexa_muted; border: 1px solid @nexa_border; border-radius: 10px; padding: 0 8px; font-size: 8.5pt; }
.nx-tag.accent { color: @nexa_accent; border-color: alpha(@nexa_accent, 0.5); }
.nx-tag.ok { color: @nexa_ok; border-color: alpha(@nexa_ok, 0.5); }
.nx-row { padding: 8px 4px; border-bottom: 1px solid alpha(@nexa_border, 0.6); }
.nx-mono { font-family: "JetBrains Mono", monospace; font-size: 9pt; color: @nexa_text; }
"""
_css_done = set()


def _setup_css(window):
    actionui.ensure_css()
    screen = window.get_screen()
    if id(screen) not in _css_done:
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(screen, provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 1)
        _css_done.add(id(screen))


def _label(text, *classes, wrap=True):
    lbl = Gtk.Label(label=text, xalign=0)
    lbl.set_line_wrap(wrap)
    lbl.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
    for c in classes:
        lbl.get_style_context().add_class(c)
    return lbl


def _button(text, *classes):
    b = Gtk.Button(label=text)
    for c in ("nx-btn",) + classes:
        b.get_style_context().add_class(c)
    return b


def ask_broker(window, action, args, done, why=None):
    """broker.run on a thread; done(result, error) on the GTK thread."""
    def ask_password(what, error):
        return askpass.ask(why or f"{what or 'This change'} needs your password.", window)(error is not None)

    def work():
        try:
            result, error = broker.run(action, args, ask_password), None
        except broker.BrokerError as e:
            result, error = None, ("" if str(e) == "cancelled" else str(e))
        GLib.idle_add(lambda: done(result, error) and False)
    threading.Thread(target=work, daemon=True).start()


def _form_dialog(window, title, fields, check, ok_label):
    """A small form. fields: [(key, label, secret)] plus ("admin", label,
    "check") for a checkbox. Returns the values or None."""
    dlg = Gtk.Dialog(title=title, transient_for=window, modal=True)
    dlg.add_buttons("Cancel", Gtk.ResponseType.CANCEL, ok_label, Gtk.ResponseType.OK)
    dlg.set_default_response(Gtk.ResponseType.OK)
    box = dlg.get_content_area()
    box.set_spacing(8)
    box.set_border_width(16)
    widgets = {}
    for key, label, kind in fields:
        if kind == "check":
            w = Gtk.CheckButton(label=label)
        else:
            box.pack_start(_label(label, "nx-card-status"), False, False, 0)
            w = Gtk.Entry()
            w.set_visibility(kind != "secret")
            w.set_activates_default(True)
        box.pack_start(w, False, False, 0)
        widgets[key] = w
    problem = _label("", "nx-card-status", "bad")
    box.pack_start(problem, False, False, 0)
    dlg.show_all()
    while True:
        if dlg.run() != Gtk.ResponseType.OK:
            dlg.destroy()
            return None
        values = {k: (w.get_active() if isinstance(w, Gtk.CheckButton) else w.get_text()) for k, w in widgets.items()}
        msg = check(values)
        if not msg:
            dlg.destroy()
            return values
        problem.set_text(msg)


def _confirm(window, title, text, ok_label, danger=False):
    dlg = Gtk.MessageDialog(transient_for=window, modal=True, message_type=Gtk.MessageType.OTHER,
                            buttons=Gtk.ButtonsType.NONE, text=title)
    dlg.format_secondary_text(text)
    dlg.add_buttons("Cancel", Gtk.ResponseType.CANCEL, ok_label, Gtk.ResponseType.OK)
    ok = dlg.get_widget_for_response(Gtk.ResponseType.OK)
    ok.get_style_context().add_class("nx-btn")
    ok.get_style_context().add_class("danger" if danger else "primary")
    answer = dlg.run() == Gtk.ResponseType.OK
    dlg.destroy()
    return answer


# ---- Users -------------------------------------------------------------------

class UsersPage:
    def __init__(self, window, etc="/etc"):
        self.window, self.etc = window, etc
        _setup_css(window)
        self.page = window._page("Users")
        head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        head.pack_start(_label("The people who use this computer. Administrators can change the system "
                               "(software, startup, other people).", "nx-intro"), True, True, 0)
        add = _button("Add a person…", "primary")
        add.set_valign(Gtk.Align.CENTER)
        add.connect("clicked", self._add)
        head.pack_start(add, False, False, 0)
        self.page.pack_start(head, False, False, 0)
        self.status = _label("", "nx-card-status")
        self.page.pack_start(self.status, False, False, 0)
        self.list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.page.pack_start(self.list, False, False, 0)
        self.reload()

    def reload(self):
        for c in self.list.get_children():
            c.destroy()
        for person in accounts.list_people(self.etc):
            self.list.pack_start(self._card(person), False, False, 0)
        self.list.show_all()

    def _card(self, p):
        card = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=14)
        card.get_style_context().add_class("nx-card")
        badge = actionui.HexBadge(p["full_name"][:1].upper() or "?", "change" if p["you"] else "muted", size=40)
        card.pack_start(badge, False, False, 0)
        body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        card.pack_start(body, True, True, 0)
        title = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        title.pack_start(_label(p["full_name"], "nx-card-title", wrap=False), False, False, 0)
        if p["you"]:
            title.pack_start(_label("You", "nx-tag", "accent", wrap=False), False, False, 0)
        if p["admin"]:
            title.pack_start(_label("Administrator", "nx-tag", wrap=False), False, False, 0)
        body.pack_start(title, False, False, 0)
        body.pack_start(_label(f"Signs in as {p['name']}", "nx-card-status"), False, False, 0)
        if p["you"]:
            b = _button("Change my password…")
            b.connect("clicked", self._change_password)
        else:
            b = _button("Remove…")
            b.connect("clicked", self._remove, p)
        b.set_halign(Gtk.Align.START)
        body.pack_start(b, False, False, 6)
        return card

    def _done(self, ok_text):
        def done(result, error):
            if error is None:
                self.status.set_text((result or {}).get("done", ok_text) if isinstance(result, dict) else ok_text)
                self.reload()
            elif error:
                self.status.set_text(f"Couldn't do that: {error}")
        return done

    def _change_password(self, _b):
        v = _form_dialog(self.window, "Change my password",
                         [("new", "New password", "secret"), ("again", "New password again", "secret")],
                         lambda v: accounts.check_new_account("me", v["new"], v["again"]), "Change")
        if v:
            ask_broker(self.window, "change_password", {"new_password": v["new"]}, self._done("Your password was changed"),
                       why="To change your password, first enter your current one.")

    def _add(self, _b):
        v = _form_dialog(self.window, "Add a person",
                         [("name", "User name (to sign in)", "text"), ("full_name", "Full name", "text"),
                          ("password", "Password", "secret"), ("again", "Password again", "secret"),
                          ("admin", "Administrator (can change the system)", "check")],
                         lambda v: accounts.check_new_account(v["name"], v["password"], v["again"]), "Add")
        if v:
            ask_broker(self.window, "add_user", {"name": v["name"], "full_name": v["full_name"], "password": v["password"],
                                                 "admin": v["admin"]}, self._done(f"Added {v['name']}"))

    def _remove(self, _b, p):
        if _confirm(self.window, f"Remove {p['full_name']}?",
                    f"They won't be able to sign in any more. Their files stay in /home/{p['name']}.", "Remove", danger=True):
            ask_broker(self.window, "remove_user", {"name": p["name"]}, self._done(f"Removed {p['name']}"))


# ---- Updates -----------------------------------------------------------------

class UpdatesPage:
    def __init__(self, window):
        self.window = window
        _setup_css(window)
        self.page = window._page("Updates")
        self.page.pack_start(_label("Security fixes and new versions for NexaOS and your apps, from Alpine's "
                                    "signed servers.", "nx-intro"), False, False, 0)
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        self.check_btn = _button("Check for updates")
        self.check_btn.connect("clicked", self._check)
        self.update_btn = _button("Update now", "primary")
        self.update_btn.set_sensitive(False)
        self.update_btn.connect("clicked", self._update)
        self.spinner = Gtk.Spinner()
        for w in (self.check_btn, self.update_btn, self.spinner):
            row.pack_start(w, False, False, 0)
        self.page.pack_start(row, False, False, 0)
        self.status = _label("Not checked yet.", "nx-card-line")
        self.page.pack_start(self.status, False, False, 0)
        self.list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.page.pack_start(self.list, False, False, 0)

    def _busy(self, on, text=None):
        self.check_btn.set_sensitive(not on)
        self.update_btn.set_sensitive(False)
        (self.spinner.start if on else self.spinner.stop)()
        if text:
            self.status.set_text(text)

    def _check(self, _b=None):
        self._busy(True, "Checking…")
        ask_broker(self.window, "update_index", {}, self._checked)

    def _checked(self, result, error):
        self._busy(False)
        for c in self.list.get_children():
            c.destroy()
        if error is not None:
            self.status.set_text(f"Couldn't check: {error}" if error else "Not checked.")
            return
        pkgs = result.get("packages", [])
        self.status.set_text(f"{result.get('done', '')} · checked at {time.strftime('%H:%M')}")
        for p in pkgs[:60]:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
            row.get_style_context().add_class("nx-row")
            row.pack_start(_label(p["name"], "nx-card-line", wrap=False), True, True, 0)
            row.pack_start(_label(f"{p['from']}  →  {p['to']}", "nx-mono", wrap=False), False, False, 0)
            self.list.pack_start(row, False, False, 0)
        if len(pkgs) > 60:
            self.list.pack_start(_label(f"and {len(pkgs) - 60} more", "nx-card-status"), False, False, 6)
        self.list.show_all()
        self.update_btn.set_sensitive(bool(pkgs))

    def _update(self, _b):
        if not _confirm(self.window, "Update now?", "This can take a few minutes. Keep the computer on and "
                        "plugged in; some apps may need restarting afterwards.", "Update"):
            return
        self._busy(True, "Updating… this can take a few minutes.")
        ask_broker(self.window, "system_update", {}, self._updated)

    def _updated(self, result, error):
        self._busy(False)
        if error is not None:
            self.status.set_text(f"The update didn't finish: {error}" if error else "Not updated.")
            return
        self.status.set_text(result.get("done", "The system is up to date") + ". Restart apps to use their new versions.")
        for c in self.list.get_children():
            c.destroy()


# ---- Startup -----------------------------------------------------------------

class StartupPage:
    def __init__(self, window):
        self.window = window
        _setup_css(window)
        self.page = window._page("Startup")
        self.page.pack_start(_label("Services that start with the computer. A switch changes the next start; "
                                    "it doesn't stop or start anything now. Parts of the system itself aren't listed.",
                                    "nx-intro"), False, False, 0)
        self.status = _label("", "nx-card-status")
        self.page.pack_start(self.status, False, False, 0)
        self.list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.page.pack_start(self.list, False, False, 0)
        self._updating = False

    def reload(self):
        self.status.set_text("Loading…")
        ask_broker(self.window, "list_services", {}, self._show)

    def _show(self, result, error):
        for c in self.list.get_children():
            c.destroy()
        if error is not None:
            self.status.set_text(f"Couldn't list services: {error}")
            return
        services = [s for s in result.get("services", []) if not s.get("protected")]
        hidden = len(result.get("services", [])) - len(services)
        self.status.set_text((f"{hidden} system service is" if hidden == 1 else f"{hidden} system services are")
                             + " always on and not shown." if hidden else "")
        for s in services:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
            row.get_style_context().add_class("nx-row")
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            text.pack_start(_label(s["name"], "nx-card-line", wrap=False), False, False, 0)
            if s.get("description"):
                text.pack_start(_label(s["description"], "nx-card-status"), False, False, 0)
            row.pack_start(text, True, True, 0)
            state = _label("running" if s.get("running") else "stopped", "nx-tag", *(["ok"] if s.get("running") else []), wrap=False)
            row.pack_start(state, False, False, 0)
            sw = Gtk.Switch()
            sw.set_active(bool(s.get("at_startup")))
            sw.set_valign(Gtk.Align.CENTER)
            sw.connect("notify::active", self._toggled, s)
            row.pack_start(sw, False, False, 0)
            self.list.pack_start(row, False, False, 0)
        self.list.show_all()

    def _toggled(self, sw, _pspec, s):
        if self._updating:
            return
        on = sw.get_active()

        def done(result, error):
            if error is None:
                verb = "will" if on else "won't"
                self.status.set_text(f"{s['name']} {verb} start with the computer.")
                s["at_startup"] = on
            else:
                self._updating = True
                sw.set_active(not on)  # put the switch back
                self._updating = False
                self.status.set_text(f"Couldn't change {s['name']}: {error}" if error else "Not changed.")
        ask_broker(self.window, "enable_service" if on else "disable_service", {"name": s["name"]}, done)
