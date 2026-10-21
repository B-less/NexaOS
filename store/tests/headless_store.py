"""The Nexa Store window against the real nexa-store engine and real signed
catalogues (scripts/test-store-app.sh builds them), with a stand-in broker
that records install/remove and updates the registry like the real one.
Screenshots of each view go to $OUT."""
import json
import os
import socket
import subprocess
import sys
import threading
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "store"), os.path.join(REPO, "common")]

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

from nexa_store import window as store  # noqa: E402

W = os.environ["STORE_TEST_DIR"]
OUT = os.environ.get("OUT", "/tmp")
failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def pump(seconds=0.3):
    end = time.time() + seconds
    while time.time() < end:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        time.sleep(0.01)


def wait(cond, seconds=15):
    end = time.time() + seconds
    while time.time() < end:
        pump(0.05)
        if cond():
            return True
    return False


# ---- the stand-in broker ---------------------------------------------------
asked = []
sock_path = os.environ["NEXA_BROKER_SOCKET"]
srv = socket.socket(socket.AF_UNIX)
srv.bind(sock_path)
srv.listen(8)


def answer(req):
    action, args = req["action"], req.get("args", {})
    asked.append((action, args))
    if action == "install_app":
        path = args["path"]
        app_id, version = os.path.basename(path)[:-5].rsplit("-", 1)
        if app_id == "hexes" and not args.get("allow_unverified"):
            return {"ok": False, "unverified": True, "error": "Hexes isn't signed by a publisher you trust."}
        if not os.path.isfile(path):
            return {"ok": False, "error": "the package isn't there"}
        with open(os.path.join(W, "registry", f"{app_id}.json"), "w") as f:
            json.dump({"id": app_id, "version": version, "trust": "trusted", "permissions": [], "paths": []}, f)
        return {"ok": True, "result": {"done": f"Installed {app_id} {version}"}}
    if action == "uninstall_app":
        os.remove(os.path.join(W, "registry", f"{args['id']}.json"))
        return {"ok": True, "result": {"done": f"Removed {args['id']}"}}
    return {"ok": False, "error": "not this action"}


def serve():
    while True:
        c, _ = srv.accept()
        data = b""
        while not data.endswith(b"\n"):
            chunk = c.recv(65536)
            if not chunk:
                break
            data += chunk
        c.sendall((json.dumps(answer(json.loads(data))) + "\n").encode())
        c.close()


threading.Thread(target=serve, daemon=True).start()

# ---- helpers ----------------------------------------------------------------


def shot(name):
    pump(0.6)
    stack = [json.loads(subprocess.run(["swaymsg", "-t", "get_tree"], capture_output=True, text=True).stdout)]
    while stack:
        n = stack.pop()
        if n.get("pid") == os.getpid() and n.get("visible"):
            r = n["rect"]
            subprocess.run(["grim", "-g", f"{r['x']},{r['y']} {r['width']}x{r['height']}", os.path.join(OUT, name)])
            return
        stack.extend(n.get("nodes", []) + n.get("floating_nodes", []))


def cards():
    return [c.get_child() for c in win.grid.get_children()]


def card_names():
    names = []
    for btn in cards():
        def walk(w):
            if isinstance(w, Gtk.Label) and w.get_name() == "card-name":
                names.append(w.get_text())
            if isinstance(w, Gtk.Container):
                w.foreach(walk)
        walk(btn)
    return names


def side(key):
    for row in win.side.get_children():
        if getattr(row, "key", None) == key:
            win.side.select_row(row)
            pump(0.3)
            return True
    return False


def page_buttons():
    found = {}

    def walk(w):
        if isinstance(w, Gtk.Button) and w.get_label():
            found[w.get_label()] = w
        if isinstance(w, Gtk.Container):
            w.foreach(walk)
    walk(win.app_scroll)
    return found


def page_texts():
    texts = []

    def walk(w):
        if isinstance(w, Gtk.Label):
            texts.append(w.get_text())
        if isinstance(w, Gtk.Container):
            w.foreach(walk)
    walk(win.app_scroll)
    return texts


# ---- the test ---------------------------------------------------------------
confirms = []
store.StoreWindow._confirm = lambda self, text, secondary: confirms.append(text) or True

win = store.StoreWindow()
win.connect("destroy", Gtk.main_quit)
win.show_all()
check("first start fetches the catalogues", wait(lambda: not win.busy and win.apps, 30), win.status.get_text())
check("every app from both catalogues", sorted(a["id"] for a in win.apps) == ["chat", "clock", "hexes", "notes-pro", "sketch", "video-player"],
      str([a["id"] for a in win.apps]))
check("Discover shows them as cards", len(cards()) == 6, str(card_names()))
check("an unreachable online catalogue is a quiet note, not an error",
      "Couldn't reach Online right now" in win.status.get_text() and not win.status.get_style_context().has_class("error"),
      win.status.get_text())
rows = [getattr(r, "key", None) for r in win.side.get_children()]
check("categories come from the apps", {"Games", "Graphics", "Internet", "Music & video", "Office", "Utilities"} <= set(rows), str(rows))
check("Updates says how many", any("Updates (1)" in (r.get_child().get_text() if isinstance(r.get_child(), Gtk.Label) else "") for r in win.side.get_children()))
shot("store-discover.png")

win.search.set_text("paint")
pump(0.6)
check("search finds by description", card_names() == ["Sketch"], str(card_names()))
win.search.set_text("zzzz")
pump(0.6)
check("no match: a friendly empty state", win.empty.get_visible() and "No apps match" in win.empty.get_text(), win.empty.get_text())
shot("store-search-empty.png")
win.search.set_text("")
pump(0.4)

# An app's page: permissions in plain words, verified publisher, Install.
win._open_app("chat")
pump(0.5)
texts = page_texts()
check("the app page lists what it can do", "Use the internet" in texts and "Use the microphone" in texts and "Show notifications" in texts, str(texts))
check("...and who vouches for it", any(t.startswith("✓ Verified publisher") for t in texts), str(texts))
shot("store-app-page.png")
page_buttons()["Install"].clicked()
check("Install: downloaded, checked, then installed through the broker", wait(lambda: any(a == "install_app" for a, _ in asked) and not win.busy, 30), str(asked))
path = next(args["path"] for a, args in asked if a == "install_app")
check("...from the engine's verified download", path.endswith("/.cache/nexa-store/packages/chat-0.9.1.nexa") and os.path.isfile(path), path)
check("...and the page now offers Open and Remove", wait(lambda: {"Open", "Remove"} <= set(page_buttons()), 10), str(list(page_buttons())))

# An update.
side("updates")
check("Updates lists Notes Pro", card_names() == ["Notes Pro"], str(card_names()))
win._open_app("notes-pro")
pump(0.4)
check("its page says what is installed", any("installed 1.0.0" in t for t in page_texts()), str(page_texts()))
page_buttons()["Update"].clicked()
check("Update installs the new version", wait(lambda: any(a == "install_app" and "notes-pro-1.2.0" in args["path"] for a, args in asked) and not win.busy, 30), str(asked))
check("...and Updates is empty afterwards", wait(lambda: not any(a.get("state") == "update" for a in win.installed), 10))

# From an unverified catalogue: the broker refuses, the Store asks, then installs.
win._open_app("hexes")
pump(0.4)
check("an unsigned app says so", any(t.startswith("Not verified") for t in page_texts()), str(page_texts()))
shot("store-unverified.png")
page_buttons()["Install"].clicked()
check("unverified: asked first, then installed with permission",
      wait(lambda: any(a == "install_app" and args.get("allow_unverified") and "hexes" in args["path"] for a, args in asked) and not win.busy, 30),
      str((confirms, asked[-2:])))

# Installed and Remove.
side("installed")
check("Installed lists everything installed", sorted(card_names()) == ["Chat", "Clock", "Hexes", "Notes Pro"], str(card_names()))
shot("store-installed.png")
win._open_app("clock")
pump(0.4)
page_buttons()["Remove"].clicked()
check("Remove asks, then removes through the broker", wait(lambda: ("uninstall_app", {"id": "clock"}) in asked and not win.busy, 15), str(asked[-1:]))
check("...and it's gone from Installed", wait(lambda: "clock" not in [a["id"] for a in win.installed], 10))

# The broker isn't running: a plain message, nothing crashes.
os.environ["NEXA_BROKER_SOCKET"] = "/nonexistent.sock"
from nexa_common import broker  # noqa: E402
broker.SOCKET = "/nonexistent.sock"
win._open_app("sketch")
pump(0.4)
page_buttons()["Install"].clicked()
check("no broker: says the system service isn't running", wait(lambda: not win.busy and "system service" in win.status.get_text(), 30), win.status.get_text())
shot("store-error.png")

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
