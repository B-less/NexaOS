"""Nexa Browser's tab, bookmark, reader and password features in the real
system image (scripts/test-browser-features.sh): pinned and muted tabs, the
tab list and recently closed tabs, the bookmark manager with an imported
Firefox file, Reader view, and saving and filling a password, against the
real engine (nexa-webdata)."""
import json
import os
import sys

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

from nexa_browser import passwords, session, webdata, window as W  # noqa: E402

BASE = "http://localhost:8765"
OUT = os.environ.get("OUT", "/tmp")
failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""), flush=True)
    if not cond:
        failures.append(label)


def pump(seconds, until=None):
    end = GLib.get_monotonic_time() + int(seconds * 1e6)
    ctx = GLib.MainContext.default()
    while GLib.get_monotonic_time() < end:
        while ctx.pending():
            ctx.iteration(False)
        if until is not None and until():
            return True
        GLib.usleep(20000)
    return until() if until else True


def js(view, code, world=None, timeout=5):
    box = {}

    def done(v, res):
        try:
            box["v"] = json.loads(v.evaluate_javascript_finish(res).to_json(0) or "null")
        except Exception as e:
            box["e"] = str(e)
    view.evaluate_javascript(code, -1, world, None, None, done)
    pump(timeout, lambda: box)
    return box.get("v", box.get("e"))


def loaded(view, ending=None):
    return lambda: not view.is_loading() and (view.get_uri() or "").endswith(ending or "") and view.get_uri() not in (None, "about:blank")


def info_buttons(tab):
    return {b.get_label(): b for b in tab.infobar.get_action_area().get_children()}


def shot(name):
    os.system(f"grim {OUT}/{name}.png")


win = W._track(W.BrowserWindow())
win.resize(1280, 800)
pump(2)

# ---- tabs: pin, mute, the tab menu, the tab list, recently closed
a = win.current()
a.view.load_uri(BASE + "/find.html")
b = win.new_tab(BASE + "/news.html")
c = win.new_tab(BASE + "/links.html")
pump(8, lambda: all(loaded(t.view)() for t in (a, b, c)))
win.pin(c, True)
check("Pin: the tab goes first, shows only its icon", win._in_order()[0] is c and not c.title.get_visible() and not c.close_btn.get_visible())
win.notebook.set_current_page(win.notebook.page_num(c.page))
win.close_tab(c)
check("Ctrl+W (or a middle click) doesn't close a pinned tab", c in win.tabs)
labels = [i.get_label() for i in win.tab_menu(b).get_children() if isinstance(i, Gtk.MenuItem) and i.get_label()]
check("right-click on a tab: its menu", labels[:4] == ["Reload", "Duplicate", "Pin", "Mute"] and "Close Other Tabs" in labels, str(labels))
b.toggle_mute()
check("Mute a tab (before it plays anything)", b.view.get_is_muted() and b.audio.get_visible())
b.toggle_mute()
win.notebook.set_current_page(win.notebook.page_num(a.page))
win.close_tab(b)
check("a closed tab is remembered with its title", win.closed and win.closed[-1]["url"].endswith("/news.html"), str(win.closed))
pop = win.show_tab_list()
pump(0.5)
lst = pop.get_child().get_children()[1].get_child().get_child()
rows = [r for r in lst.get_children() if hasattr(r, "activate_fn")]  # (the heading is a row too)
check("the tab list: open tabs, then recently closed", len(rows) == 3, str(len(rows)))
pop.popdown()
win.reopen_closed(win.closed[-1])
pump(5, lambda: any((t.view.get_uri() or "").endswith("/news.html") for t in win.tabs))
check("reopen a closed tab from the list", any((t.view.get_uri() or "").endswith("/news.html") for t in win.tabs) and not win.closed)
win._save_session()
tabs, _active = session.load()
pinned, _closed = session.load_extra()
check("pinned tabs come back next time", pinned == [BASE + "/links.html"] and tabs[0] == BASE + "/links.html", f"{tabs} {pinned}")

# ---- bookmark manager: import a Firefox file, folders, edit, export
firefox = os.path.join(os.environ["HOME"], "ff-bookmarks.html")
with open(firefox, "w") as f:
    f.write("""<!DOCTYPE NETSCAPE-Bookmark-file-1><TITLE>Bookmarks</TITLE><H1>Bookmarks Menu</H1>
<DL><p><DT><A HREF="https://nexaos.example/" ADD_DATE="1700000000">NexaOS</A>
<DT><H3 PERSONAL_TOOLBAR_FOLDER="true">Bookmarks Toolbar</H3>
<DL><p><DT><A HREF="https://news.example/">News</A><DT><H3>Work</H3>
<DL><p><DT><A HREF="https://mail.example/">Mail</A><DT><A HREF="javascript:alert(1)">Bad</A></DL><p></DL><p></DL>""")
m = win.show_bookmark_manager()
pump(0.5)
added = m.import_file(firefox)
check("import bookmarks from another browser's HTML file", added == 3 and "Bookmarks Toolbar/Work" in W.BOOKMARKS.folders()
      and "1 couldn't be used" in m.status.get_text(), m.status.get_text())
check("imported again: nothing doubled", m.import_file(firefox) == 0)
m.fill_folders(select="Bookmarks Toolbar/Work")
check("a folder shows its bookmarks", [r.url for r in m.list.get_children() if hasattr(r, "url")] == ["https://mail.example/"])
W.BOOKMARKS.edit("https://mail.example/", title="Work mail", folder="Office")
m.fill_folders(select="Office")
check("edit: new name and folder", [r.url for r in m.list.get_children() if hasattr(r, "url")] == ["https://mail.example/"]
      and W.BOOKMARKS.get("https://mail.example/")["title"] == "Work mail")
W.BOOKMARKS.rename_folder("Bookmarks Toolbar", "Toolbar")
check("rename a folder (and the ones inside)", W.BOOKMARKS.get("https://news.example/")["folder"] == "Toolbar")
out = os.path.join(os.environ["HOME"], "export.html")
check("export for other browsers", m.export(out) and open(out).read().count("<DT><A HREF") == 3)
m.destroy()

# ---- Reader view
art = win.current()
art.view.load_uri(BASE + "/article.html")
pump(8, loaded(art.view, "/article.html"))
check("the page itself runs its scripts", js(art.view, "document.body.dataset.ran || 'no'") == "yes")
win.toggle_reader()
pump(8, lambda: art.reader_url and not art.reader_pending and not art.view.is_loading())
pump(0.5)
r = js(art.view, "JSON.stringify({s: document.scripts.length, ran: document.body.dataset.ran || 'no', menu: !!document.querySelector('nav'),"
                 "share: document.body.innerText.includes('Share this'), text: document.body.innerText.includes('basalt columns'),"
                 "h1: document.querySelector('h1').innerText, img: document.querySelector('img') && document.querySelector('img').src})")
try:
    r = json.loads(r) if isinstance(r, str) else {"raw": r}
except ValueError:
    r = {"raw": r, "uri": art.view.get_uri(), "reader": art.reader_url, "info": art.info_label.get_text()}
check("Reader view: the article, its picture, no scripts, no menu or buttons", r.get("text") and r.get("s") == 0 and r.get("ran") == "no"
      and not r.get("menu") and not r.get("share") and r.get("h1") == "Why Nature Loves Hexagons" and r.get("img") == BASE + "/pixel.png", r)
check("the address bar still shows the article's address", win.url_entry.get_text() == BASE + "/article.html", win.url_entry.get_text())
shot("reader")
win.toggle_reader()
pump(6, loaded(art.view, "/article.html"))
check("Reader view off: the full page again", art.view.get_uri() == BASE + "/article.html" and not art.reader_url)
art.view.load_uri(BASE + "/find.html")
pump(6, loaded(art.view, "/find.html"))
win.toggle_reader()
pump(5, lambda: art.infobar.get_visible())
check("a page that isn't an article: said so", "doesn't look like an article" in art.info_label.get_text(), art.info_label.get_text())
art.infobar.hide()

# ---- saved passwords
tab = win.current()
tab.view.load_uri(BASE + "/login.html")
pump(8, loaded(tab.view, "/login.html"))
check("the page's own scripts can't reach the password handler",
      js(tab.view, "String(!!(window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.nexaPasswords))") == "false")
js(tab.view, "document.getElementById('user').value='ama@example.com'; document.getElementById('pass').value='hex-4-bees!';"
             "document.getElementById('f').requestSubmit(); 1")
pump(6, lambda: "Save the password" in tab.info_label.get_text() and tab.infobar.get_visible())
check("after signing in: Save the password?", "Save the password for <b>ama@example.com</b>" in (tab.info_label.get_label() or ""), tab.info_label.get_label())
shot("password-offer")
info_buttons(tab)["Save"].clicked()
pump(1)
found = webdata.run("vault_find", origin="http://localhost:8765")
check("saved in the encrypted vault", [(e["username"], e["password"]) for e in found["entries"]] == [("ama@example.com", "hex-4-bees!")])
vault_file = os.path.join(os.environ["HOME"], ".local/share/nexa-browser/passwords.vault")
check("...which doesn't hold the password in plain text", b"hex-4-bees" not in open(vault_file, "rb").read() and os.stat(vault_file).st_mode & 0o777 == 0o600)
tab.view.load_uri(BASE + "/login.html")
pump(8, lambda: "saved password" in tab.info_label.get_text() and tab.infobar.get_visible())
check("back on the sign-in page: Fill in?", "You have a saved password" in tab.info_label.get_text(), tab.info_label.get_text())
info_buttons(tab)["Fill In"].clicked()
pump(1)
check("Fill In fills the name and password", js(tab.view, "document.getElementById('user').value + '|' + document.getElementById('pass').value") == "ama@example.com|hex-4-bees!")
pm = passwords.PasswordManager(win)
pm.show_all()
pump(1)
check("the Passwords window lists it", any("localhost:8765" in c.get_text() for row in pm.list.get_children()
                                            for c in row.get_child().get_children()[0].get_children() if isinstance(c, Gtk.Label)))
pm.destroy()
# A primary password: nothing is filled until it's typed.
webdata.run("vault_primary", new="primary-pass-1")
passwords.KEEPER.primary = None
tab.infobar.hide()
tab.view.load_uri(BASE + "/login.html?again")
pump(6, loaded(tab.view, "again"))
pump(2)
check("with a primary password not typed yet: no fill offer", not tab.infobar.get_visible())
priv = W._track(W.BrowserWindow(private=True))
check("private windows never save or fill passwords", priv.passwords is None)
priv.destroy()

print(f"\n{len(failures)} failure(s)", flush=True)
sys.exit(1 if failures else 0)
