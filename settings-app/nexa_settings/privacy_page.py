"""Settings > Privacy: each installed app, what it may use, and switches to
take permissions away. The data and the rules are in nexa_common/privacy.py;
nexa-run (Rust) applies them every time an app starts."""
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf, GLib, Gtk, Pango

from nexa_common import actionui, privacy

CSS = b"""
.nx-perm-row { padding: 6px 0; }
.nx-perm-label { color: @nexa_text; }
.nx-tag { color: @nexa_muted; border: 1px solid @nexa_border; border-radius: 10px; padding: 0 8px; font-size: 8.5pt; }
.nx-intro { color: @nexa_muted; }
switch { border-radius: 14px; background-image: none; background-color: @nexa_hover; border: 1px solid @nexa_border; }
switch:checked { background-color: @nexa_accent; border-color: @nexa_accent; }
switch slider { border-radius: 50%; background-image: none; background-color: @nexa_text_strong; min-width: 20px; min-height: 20px; }
switch:disabled { opacity: 0.4; }
"""


def _label(text, *classes, wrap=True):
    lbl = Gtk.Label(label=text, xalign=0)
    lbl.set_line_wrap(wrap)
    lbl.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
    for c in classes:
        lbl.get_style_context().add_class(c)
    return lbl


class PrivacyPage:
    def __init__(self, window, registry=privacy.REGISTRY, apps_dir=privacy.APPS_DIR, icon_dir=privacy.ICON_DIR, home=None):
        self.window, self.home = window, home
        self.where = dict(registry=registry, apps_dir=apps_dir, icon_dir=icon_dir, home=home)
        actionui.ensure_css()
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(window.get_screen(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 1)
        self.page = window._page("Privacy")
        self.page.pack_start(_label(
            "What each app you installed may use. NexaOS holds apps to this: switch something off and the app "
            "can't use it from the next time it starts.", "nx-intro"), False, False, 0)
        self.status = _label("", "nx-card-status")
        self.page.pack_start(self.status, False, False, 0)
        self.list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        self.page.pack_start(self.list, False, False, 0)
        self.reload()

    def reload(self):
        for child in self.list.get_children():
            child.destroy()
        apps = privacy.list_apps(**self.where)
        if not apps:
            self.list.pack_start(_label("No apps installed yet. Apps you install from .nexa packages appear here, "
                                        "with what each may use.", "nx-empty"), False, False, 8)
        for app in apps:
            self.list.pack_start(self._card(app), False, False, 0)
        self.list.show_all()

    def _icon(self, app):
        if app["icon"]:
            try:
                pix = GdkPixbuf.Pixbuf.new_from_file_at_size(app["icon"], 40, 40)
                return Gtk.Image.new_from_pixbuf(pix)
            except GLib.Error:
                pass
        return actionui.HexBadge("app", "change", size=40)

    def _card(self, app):
        card = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=14)
        card.get_style_context().add_class("nx-card")
        icon = self._icon(app)
        icon.set_valign(Gtk.Align.START)
        card.pack_start(icon, False, False, 0)
        body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        card.pack_start(body, True, True, 0)
        body.pack_start(_label(app["name"], "nx-card-title"), False, False, 0)
        body.pack_start(_label(f"Version {app['version']} · {app['signed']}", "nx-card-status"), False, False, 0)
        summary = _label(privacy.summary(app), "nx-card-line")
        body.pack_start(summary, False, False, 4)
        for perm in app["permissions"]:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
            row.get_style_context().add_class("nx-perm-row")
            row.pack_start(_label(perm["label"], "nx-perm-label"), True, True, 0)
            if not perm["enforced"]:
                tag = _label("not enforced yet", "nx-tag", wrap=False)
                tag.set_tooltip_text("NexaOS can't hold apps to this one yet, so switching it off would change nothing.")
                row.pack_start(tag, False, False, 0)
            switch = Gtk.Switch()
            switch.set_active(perm["allowed"])
            switch.set_sensitive(perm["enforced"])
            switch.set_valign(Gtk.Align.CENTER)
            switch.connect("notify::active", self._toggled, app, perm, summary)
            row.pack_start(switch, False, False, 0)
            body.pack_start(row, False, False, 0)
        return card

    def _toggled(self, switch, _pspec, app, perm, summary):
        allowed = switch.get_active()
        try:
            privacy.set_allowed(app["id"], perm["id"], allowed, self.home)
        except OSError as e:
            self.status.set_text(f"Couldn't save that: {e}")
            return
        perm["allowed"] = allowed
        summary.set_text(privacy.summary(app))
        what = perm["label"][0].lower() + perm["label"][1:]
        self.status.set_text(f"{app['name']} {'can use' if allowed else 'can no longer use'} {what} "
                             f"from the next time it starts.")
