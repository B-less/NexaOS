"""Saved passwords: noticing a sign-in, offering to save and to fill, and the
manager window. The passwords themselves live in nexa-webdata's encrypted
vault; this module only shows them.

The page side is a small script in its own world ("nexa-pw"), which the
page's own scripts can't see or call: it reports a submitted password and
fills one when the person asks. Nothing is filled without a click."""
import json
import time

from gi.repository import Gdk, GLib, Gtk, Pango

from . import webdata

WORLD = "nexa-pw"
HANDLER = "nexaPasswords"

SCRIPT = r"""
(() => {
  if (window.__nexaPw) return; window.__nexaPw = true;
  const post = (m) => { try { window.webkit.messageHandlers.nexaPasswords.postMessage(JSON.stringify(m)); } catch (e) {} };
  const userFor = (pw) => {
    const scope = pw.form || document;
    const inputs = Array.from(scope.querySelectorAll('input'));
    for (let k = inputs.indexOf(pw) - 1; k >= 0; k--) {
      const t = (inputs[k].type || 'text').toLowerCase();
      if (['text', 'email', 'tel', ''].includes(t) && inputs[k].value) return inputs[k].value;
    }
    return '';
  };
  const capture = (scope) => {
    const pw = Array.from(scope.querySelectorAll('input[type=password]')).find(p => p.value);
    if (pw) post({kind: 'submit', origin: location.origin, username: userFor(pw), password: pw.value});
  };
  document.addEventListener('submit', (e) => capture(e.target), true);
  document.addEventListener('click', (e) => {
    const b = e.target && e.target.closest && e.target.closest('button, input[type=submit]');
    if (b) capture(b.form || b.closest('form') || document);
  }, true);
  const announce = () => { if (document.querySelector('input[type=password]')) { post({kind: 'form', origin: location.origin}); return true; } return false; };
  if (!announce()) setTimeout(announce, 1500);
})();
"""


def fill_script(username, password):
    return """
(() => {
  const u = %s, p = %s;
  const pw = document.querySelector('input[type=password]');
  if (!pw) return false;
  const set = (el, v) => { el.focus(); el.value = v; el.dispatchEvent(new Event('input', {bubbles: true})); el.dispatchEvent(new Event('change', {bubbles: true})); };
  const scope = pw.form || document;
  const inputs = Array.from(scope.querySelectorAll('input'));
  for (let k = inputs.indexOf(pw) - 1; k >= 0 && u; k--) {
    const t = (inputs[k].type || 'text').toLowerCase();
    if (['text', 'email', 'tel', ''].includes(t)) { set(inputs[k], u); break; }
  }
  set(pw, p);
  return true;
})();""" % (json.dumps(username), json.dumps(password))


class Keeper:
    """The primary password, typed once per run of the browser."""

    def __init__(self):
        self.primary = None

    def run(self, parent, op, **args):
        """A vault op; asks for the primary password when it's needed."""
        while True:
            try:
                return webdata.run(op, primary=self.primary, **args)
            except webdata.EngineError as e:
                if e.code != "locked" or parent is None:
                    raise
                wrong = self.primary is not None
                self.primary = ask_password(parent, "Saved passwords", "That primary password isn't right. Try again:" if wrong
                                            else "Type your primary password to use your saved passwords:")
                if self.primary is None:
                    raise

    def locked(self):
        try:
            s = webdata.run("vault_status")
        except webdata.EngineError:
            return False
        return s.get("primary") and self.primary is None


KEEPER = Keeper()


def ask_password(parent, title, text, confirm=False):
    d = Gtk.Dialog(title=title, transient_for=parent, modal=True)
    d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "OK", Gtk.ResponseType.OK)
    d.set_default_response(Gtk.ResponseType.OK)
    box = d.get_content_area()
    box.set_border_width(16)
    box.set_spacing(8)
    label = Gtk.Label(label=text, xalign=0)
    label.set_line_wrap(True)
    box.pack_start(label, False, False, 0)
    entries = []
    for placeholder in (("New primary password", "Type it again") if confirm else ("Password",)):
        e = Gtk.Entry(visibility=False, activates_default=True, placeholder_text=placeholder)
        box.pack_start(e, False, False, 0)
        entries.append(e)
    error = Gtk.Label(xalign=0)
    box.pack_start(error, False, False, 0)
    d.show_all()
    while True:
        if d.run() != Gtk.ResponseType.OK:
            d.destroy()
            return None
        values = [e.get_text() for e in entries]
        if confirm and values[0] != values[1]:
            error.set_text("The two don't match.")
            continue
        if confirm and len(values[0]) < 8:
            error.set_text("Use at least 8 characters.")
            continue
        d.destroy()
        return values[0]


class PasswordManager(Gtk.Window):
    def __init__(self, parent=None):
        super().__init__(title="Saved Passwords")
        self.set_default_size(760, 520)
        if parent:
            self.set_transient_for(parent)
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        root.set_border_width(12)
        self.add(root)
        bar = Gtk.Box(spacing=8)
        self.search = Gtk.SearchEntry(placeholder_text="Search sites and names")
        self.search.connect("search-changed", lambda *_: self.fill())
        bar.pack_start(self.search, False, False, 0)
        self.primary_btn = Gtk.Button()
        self.primary_btn.set_name("toolbtn")
        self.primary_btn.connect("clicked", lambda *_: self.primary_menu())
        bar.pack_end(self.primary_btn, False, False, 0)
        root.pack_start(bar, False, False, 0)
        scroll = Gtk.ScrolledWindow()
        self.list = Gtk.ListBox()
        self.list.set_selection_mode(Gtk.SelectionMode.NONE)
        scroll.add(self.list)
        root.pack_start(scroll, True, True, 0)
        self.status = Gtk.Label(xalign=0)
        self.status.set_name("panel-muted")
        self.status.set_line_wrap(True)
        root.pack_start(self.status, False, False, 0)
        self.connect("key-press-event", lambda _w, ev: ev.keyval == Gdk.KEY_Escape and (self.close() or True))
        self.data = {"entries": [], "never": []}
        GLib.idle_add(self.load)

    def load(self):
        try:
            self.data = KEEPER.run(self, "vault_list")
        except webdata.EngineError as e:
            self.status.set_text(str(e))
            self.data = {"entries": [], "never": []}
        self._sync_primary()
        self.fill()
        return False

    def _sync_primary(self):
        try:
            on = webdata.run("vault_status").get("primary")
        except webdata.EngineError:
            on = False
        self.primary_btn.set_label("Primary Password: On" if on else "Set a Primary Password")
        self.primary_btn.set_tooltip_text("Change or remove it" if on else
                                          "Without one, your saved passwords are encrypted with a key kept on this computer. "
                                          "With one, nothing on the disk can open them without it.")

    def fill(self):
        for c in self.list.get_children():
            c.destroy()
        q = self.search.get_text().lower().split()
        rows = [e for e in self.data.get("entries", []) if all(w in (e["origin"] + " " + e["username"]).lower() for w in q)]
        for e in sorted(rows, key=lambda e: (e["origin"], e["username"].lower())):
            row = Gtk.Box(spacing=8)
            row.set_border_width(6)
            col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            site = Gtk.Label(label=e["origin"].split("://", 1)[-1], xalign=0, ellipsize=Pango.EllipsizeMode.END)
            user = Gtk.Label(label=e["username"] or "(no name)", xalign=0, ellipsize=Pango.EllipsizeMode.END)
            user.set_name("panel-muted")
            col.pack_start(site, False, False, 0)
            col.pack_start(user, False, False, 0)
            row.pack_start(col, True, True, 0)
            shown = Gtk.Label(label="••••••••", selectable=True)
            row.pack_start(shown, False, False, 8)
            for label, fn in (("Show", lambda i=e["id"], s=shown: self.reveal(i, s)),
                              ("Copy", lambda i=e["id"]: self.copy(i)),
                              ("Delete", lambda i=e["id"]: self.delete(i))):
                b = Gtk.Button(label=label)
                b.set_name("toolbtn")
                b.connect("clicked", lambda *_a, f=fn: f())
                row.pack_start(b, False, False, 0)
            self.list.add(row)
        for origin in self.data.get("never", []):
            row = Gtk.Box(spacing=8)
            row.set_border_width(6)
            row.pack_start(Gtk.Label(label=f"Never saved for {origin.split('://', 1)[-1]}", xalign=0), True, True, 0)
            b = Gtk.Button(label="Allow Again")
            b.set_name("toolbtn")
            b.connect("clicked", lambda *_a, o=origin: self.allow(o))
            row.pack_end(b, False, False, 0)
            self.list.add(row)
        if not rows and not self.data.get("never"):
            empty = Gtk.Label(label="No saved passwords yet. When you sign in to a site, the browser offers to save the password.")
            empty.set_line_wrap(True)
            empty.set_margin_top(24)
            self.list.add(empty)
        self.list.show_all()

    def _get(self, entry_id):
        try:
            return KEEPER.run(self, "vault_get", id=entry_id)["password"]
        except webdata.EngineError as e:
            self.status.set_text(str(e))
            return None

    def reveal(self, entry_id, label):
        if label.get_text() != "••••••••":
            label.set_text("••••••••")
            return
        p = self._get(entry_id)
        if p is not None:
            label.set_text(p)

    def copy(self, entry_id):
        p = self._get(entry_id)
        if p is not None:
            Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text(p, -1)
            self.status.set_text("Copied. It's on the clipboard until you copy something else.")

    def delete(self, entry_id):
        try:
            KEEPER.run(self, "vault_delete", id=entry_id)
        except webdata.EngineError as e:
            self.status.set_text(str(e))
        self.load()

    def allow(self, origin):
        try:
            KEEPER.run(self, "vault_never", origin=origin, add=False)
        except webdata.EngineError as e:
            self.status.set_text(str(e))
        self.load()

    def primary_menu(self):
        on = webdata.run("vault_status").get("primary")
        if on:
            d = Gtk.MessageDialog(transient_for=self, flags=0, message_type=Gtk.MessageType.QUESTION, buttons=Gtk.ButtonsType.NONE,
                                  text="Your primary password")
            d.format_secondary_text("Change it, or remove it (your passwords stay encrypted with a key kept on this computer).")
            d.add_buttons("Cancel", 0, "Remove", 1, "Change", 2)
            r = d.run()
            d.destroy()
            if r == 0 or r < 0:
                return
            new = None if r == 1 else ask_password(self, "Change Primary Password", "Choose a new primary password. Nobody can recover it if you forget it.", confirm=True)
            if r == 2 and new is None:
                return
        else:
            new = ask_password(self, "Set a Primary Password", "With a primary password, nothing on this computer can open your saved passwords "
                               "without it. Nobody can recover it if you forget it.", confirm=True)
            if new is None:
                return
        try:
            KEEPER.run(self, "vault_primary", new=new)
            KEEPER.primary = new
        except webdata.EngineError as e:
            self.status.set_text(str(e))
            return
        self.status.set_text("Primary password set." if new else "Primary password removed.")
        self.load()


class Watcher:
    """Hooks one window's pages: offers to save after a sign-in, and to fill
    on a page with a password field. Never in private or Tor windows."""

    def __init__(self, win, ucm):
        self.win = win
        self.recent = {}
        script = WebKit2.UserScript.new_for_world(SCRIPT, WebKit2.UserContentInjectedFrames.TOP_FRAME,
                                                  WebKit2.UserScriptInjectionTime.END, WORLD, None, None)
        ucm.add_script(script)
        ucm.register_script_message_handler_in_world(HANDLER, WORLD)
        ucm.connect(f"script-message-received::{HANDLER}", self._on_message)

    def _tab_for(self, origin):
        cur = self.win.current()
        tabs = ([cur] if cur else []) + [t for t in self.win.tabs if t is not cur]
        return next((t for t in tabs if _origin(t.view.get_uri() or "") == origin), None)

    def _on_message(self, _ucm, value):
        try:
            # WebKitGTK 4.1 hands a JavascriptResult, newer ones the value itself.
            v = value.get_js_value() if hasattr(value, "get_js_value") else value
            m = json.loads(v.to_string())
        except (ValueError, AttributeError, TypeError):
            return
        origin = m.get("origin") if isinstance(m, dict) else None
        if not isinstance(origin, str) or not origin.startswith(("https://", "http://")):
            return
        tab = self._tab_for(origin)
        if tab is None:
            return
        if m.get("kind") == "submit" and isinstance(m.get("password"), str) and m["password"]:
            self.offer_save(tab, origin, str(m.get("username") or "")[:512], m["password"][:4096])
        elif m.get("kind") == "form":
            self.offer_fill(tab, origin)

    def offer_save(self, tab, origin, username, password):
        key = (origin, username, password)
        if time.time() - self.recent.get(key, 0) < 10:
            return  # the same sign-in seen twice (the click, then the submit)
        self.recent[key] = time.time()
        try:
            found = webdata.run("vault_find", origin=origin, primary=KEEPER.primary)
        except webdata.EngineError as e:
            found = {"entries": [], "never": False, "locked": e.code == "locked"}
        if found.get("never"):
            return
        same = [e for e in found.get("entries", []) if e["username"] == username]
        if same and same[0]["password"] == password:
            return
        site = GLib.markup_escape_text(origin.split("://", 1)[-1])
        who = f" for <b>{GLib.markup_escape_text(username)}</b>" if username else ""
        text = f"Update the saved password{who} on <b>{site}</b>?" if same else f"Save the password{who} on <b>{site}</b>?"

        def save():
            try:
                KEEPER.run(self.win, "vault_save", origin=origin, username=username, password=password)
                tab.show_info("Password saved. ☰ › Passwords shows your saved passwords.")
            except webdata.EngineError as e:
                tab.show_info(GLib.markup_escape_text(str(e)), kind=Gtk.MessageType.WARNING)

        def never():
            try:
                KEEPER.run(self.win, "vault_never", origin=origin, add=True)
            except webdata.EngineError:
                pass
        tab.show_info(text, [("Update" if same else "Save", save), ("Never for this site", never)], kind=Gtk.MessageType.QUESTION)

    def offer_fill(self, tab, origin):
        if KEEPER.locked():
            return  # not until the person opens their passwords once this run
        try:
            found = webdata.run("vault_find", origin=origin, primary=KEEPER.primary)
        except webdata.EngineError:
            return
        entries = found.get("entries", [])[:3]
        if not entries:
            return
        site = GLib.markup_escape_text(origin.split("://", 1)[-1])
        buttons = [(f"Fill as {e['username'] or 'saved'}" if len(entries) > 1 else "Fill In", lambda e=e: self.fill(tab, e)) for e in entries]
        tab.show_info(f"You have a saved password for <b>{site}</b>.", buttons)

    def fill(self, tab, entry):
        tab.view.evaluate_javascript(fill_script(entry["username"], entry["password"]), -1, WORLD, None, None, None, None)


def _origin(uri):
    from urllib.parse import urlsplit
    p = urlsplit(uri)
    if p.scheme not in ("http", "https") or not p.hostname:
        return ""
    default = {"http": 80, "https": 443}[p.scheme]
    return f"{p.scheme}://{p.hostname}" + (f":{p.port}" if p.port and p.port != default else "")


from gi.repository import WebKit2  # noqa: E402  (the window module has chosen its version)
