"""Nexa Browser in the real system image, driven like a person would
(scripts/test-browser.sh): tabs, blocking, HTTPS-only, find, zoom,
bookmarks, permissions, new-window links, session restore, video."""
import json
import os
import sys

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

from nexa_browser import blocker, prefs as P, window as W  # noqa: E402

BASE = "http://localhost:8765"
failures = []
OUT = os.environ.get("OUT", "/tmp")


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
        GLib.usleep(20000)  # never block waiting for an event: there may be none
    return until() if until else True


def js(view, code, timeout=5):
    box = {}

    def done(v, res):
        try:
            box["v"] = json.loads(v.evaluate_javascript_finish(res).to_json(0) or "null")
        except Exception as e:
            box["e"] = str(e)
    view.evaluate_javascript(code, -1, None, None, None, done)
    pump(timeout, lambda: box)
    return box.get("v", box.get("e"))


def loaded(view):
    return lambda: not view.is_loading() and view.get_uri() not in (None, "about:blank")


def shot(name):
    os.system(f"grim {OUT}/{name}.png")


# A test filter list as if downloaded from Settings (element hiding).
os.makedirs(blocker.lists_dir(), exist_ok=True)
URL = "https://lists.test/extra.txt"
with open(blocker.list_file(URL), "w") as f:
    f.write("##.ad-box\n")
W.PREFS.set("filter_lists", [{"name": "test", "url": URL}])

ready = {}
W.refresh_blocking(lambda ok: ready.setdefault("ok", ok))
pump(30, lambda: "ok" in ready)
check("the blocker compiles and WebKit accepts the rules", ready.get("ok") is True, ready)

win = W._track(W.BrowserWindow())
pump(2)
check("starts on the new-tab page", win.current().newtab and win.url_entry.get_text() == "")

# ---- blocking
win.load(BASE + "/news.html")
pump(10, loaded(win.view()))
pump(2)
r = js(win.view(), "JSON.stringify(window.R)")
r = json.loads(r) if isinstance(r, str) else {}
check("an ad server from the built-in list is blocked", r.get("ad") == "blocked", r)
check("the page's own image still loads", r.get("own") == "loaded", r)
check("element hiding from an added list hides .ad-box", r.get("adbox") == "none", r)
check("the shield shows blocking is on", win.shield_btn.get_name() == "shield-on")
shot("browser-1-blocking")
win.toggle_site_blocking()
pump(15, lambda: not W.PREFS.blocking_on(BASE) and win.shield_btn.get_sensitive())
pump(10, loaded(win.view()))
pump(2)
r = js(win.view(), "JSON.stringify(window.R)")
r = json.loads(r) if isinstance(r, str) else {}
check("blocking off for this site: the ad loads", r.get("ad") == "loaded", r)
check("...and the shield says so", win.shield_btn.get_name() == "shield-off")
win.toggle_site_blocking()
pump(15, lambda: W.PREFS.blocking_on(BASE) and win.shield_btn.get_sensitive())

# ---- find
win.load(BASE + "/find.html")
pump(10, loaded(win.view()))
win.show_find()
win.find_entry.set_text("needle")
pump(4, lambda: "match" in win.find_count.get_text())
check("find in page counts the matches", win.find_count.get_text() == "3 matches", win.find_count.get_text())
win.hide_find()

# ---- zoom per site
win.zoom(0.2)
check("zoom changes the page", abs(win.view().get_zoom_level() - 1.2) < 0.01)
t2 = win.new_tab(BASE + "/news.html")
pump(10, loaded(t2.view))
check("zoom is remembered for the site (new tab, same site)", abs(t2.view.get_zoom_level() - 1.2) < 0.01, t2.view.get_zoom_level())
win.zoom(None)

# ---- tabs
n = len(win.tabs)
win.close_tab(win.current())
check("closing a tab", len(win.tabs) == n - 1)
win.reopen_closed()
pump(10, loaded(win.view()))
check("Ctrl+Shift+T brings it back", len(win.tabs) == n and win.view().get_uri().endswith("/news.html"))

# A link with target=_blank becomes a new tab (clicked by a pretend mouse).
win.load(BASE + "/links.html")
pump(10, loaded(win.view()))
pump(1)
n = len(win.tabs)
rect = js(win.view(), "JSON.stringify(document.getElementById('blank').getBoundingClientRect())")
rect = json.loads(rect) if isinstance(rect, str) else None
if rect:
    # Where the window is (Sway knows; Wayland apps don't), then the page inside it.
    import subprocess
    tree = json.loads(subprocess.run(["swaymsg", "-t", "get_tree"], capture_output=True, text=True).stdout)
    def focused(n):
        if n.get("focused"):
            return n
        for c in n.get("nodes", []) + n.get("floating_nodes", []):
            f = focused(c)
            if f:
                return f
    wr = focused(tree)["rect"]
    vx, vy = win.view().translate_coordinates(win, 0, 0)
    tx, ty = win.get_titlebar().translate_coordinates(win, 0, 0)
    gx = wr["x"] + (vx - tx) + rect["x"] + 10
    gy = wr["y"] + (vy - ty) + rect["y"] + rect["height"] / 2
    # In the background, with this loop running: the browser has to see the
    # new pointer appear before the click reaches it.
    mouse = subprocess.Popen(["/src/build/test/nexa-vpointer", "1280", "800", "move", f"{gx:.0f}", f"{gy:.0f}",
                              "wait", "700", "click"])
    pump(5, lambda: mouse.poll() is not None)
    pump(8, lambda: len(win.tabs) == n + 1)
    where = js(win.tabs[n - 1].view, "JSON.stringify(window.CLICK || null)")
    check("a target=_blank link opens a new tab", len(win.tabs) == n + 1,
          f"{len(win.tabs)} tabs; aimed at {gx:.0f},{gy:.0f} (link {rect}); the page saw {where}")
    if len(win.tabs) == n + 1:
        pump(8, loaded(win.view()))
        check("...showing the linked page", (win.view().get_uri() or "").endswith("/find.html"), win.view().get_uri())
else:
    check("links page measured", False, str(rect))
n = len(win.tabs)
win.load(BASE + "/popup.html")
pump(8, lambda: (win.view().get_title() or "") in ("opened", "blocked"))
pump(1)
check("a pop-up the page opens by itself (no click) is blocked", len(win.tabs) == n and win.view().get_title() == "blocked",
      f"{len(win.tabs)} tabs (was {n}), page says {win.view().get_title()}")

# ---- bookmarks
win.load(BASE + "/find.html")
pump(10, loaded(win.view()))
win.toggle_bookmark()
check("Ctrl+D bookmarks the page", W.BOOKMARKS.has(BASE + "/find.html") and win.star_btn.get_label() == "★")

# ---- permissions: the site asks, the browser asks you
win.load(BASE + "/geo.html")
pump(10, loaded(win.view()))
pump(6, lambda: win.current().infobar.get_visible())
check("a site asking for your location gets a question, not an answer", win.current().infobar.get_visible()
      and "know your location" in win.current().info_label.get_text(), win.current().info_label.get_text())
shot("browser-2-permission")
btns = [b for b in win.current().infobar.get_action_area().get_children() if b.get_label() == "Block"]
if btns:
    btns[0].clicked()
pump(2)
check("...and the answer is remembered for the site", W.PREFS.permission(BASE, "location") is False)

# ---- HTTPS-only
win.load("http://plain.test:8765/find.html")
pump(12, lambda: win.current().infobar.get_visible())
check("HTTPS-only: no secure version -> you're asked, not silently sent over http",
      win.current().infobar.get_visible() and "no secure" in win.current().info_label.get_text(), win.current().info_label.get_text())
shot("browser-3-https-only")
go = [b for b in win.current().infobar.get_action_area().get_children() if "insecure" in b.get_label()]
if go:
    go[0].clicked()
pump(10, lambda: (win.view().get_uri() or "").startswith("http://plain.test") and not win.view().is_loading())
check("...and choosing to go there loads the http page", (win.view().get_uri() or "").startswith("http://plain.test"), win.view().get_uri())

# ---- video in the browser itself (HLS needs no test-side switch now)
win.load(BASE + "/media/index.html")
pump(60, lambda: (win.view().get_title() or "").startswith("RESULTS:"))
t = win.view().get_title() or ""
res = json.loads(t[len("RESULTS:"):]) if t.startswith("RESULTS:") else {}
for name, r in sorted(res.get("files", {}).items()):
    check(f"video plays: {name}", r.get("ok"), r)
for name, r in sorted(res.get("mse", {}).items()):
    check(f"streamed video (MSE) plays: {name}", r.get("ok"), r)
shot("browser-4-video")

# ---- session
win._save_session()
before = [t.view.get_uri() for t in win.tabs if not t.newtab]
win2 = W.BrowserWindow(restore=True)
W._open_windows.append(win2)
win2.show_all()
pump(3)
after = [t.view.get_uri() for t in win2.tabs]
check("the tabs come back next time", after == before, f"{before} vs {after}")
shot("browser-5-tabs")
# ---- the menu and Settings open, and Settings changes real preferences
win.menu_btn.set_active(True)
pump(1)
shot("browser-6-menu")
win.menu_btn.set_active(False)
from nexa_browser.settings_window import SettingsWindow  # noqa: E402
sw = SettingsWindow(win)
sw.show_all()
pump(2)
shot("browser-7-settings")
combo = [w for c in sw.box.get_children() if isinstance(c, Gtk.Box)
         for w in c.get_children() if isinstance(w, Gtk.ComboBoxText)]
if combo:
    combo[0].set_active_id("startpage")
check("Settings: the search engine choice is saved", P.Prefs()["search"] == "startpage")
check("Settings: lists the site with a remembered permission", "localhost" in " ".join(
    lab.get_text() for row in sw.perm_box.get_children() for lab in row.get_children() if isinstance(lab, Gtk.Label)))
sw.destroy()
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
