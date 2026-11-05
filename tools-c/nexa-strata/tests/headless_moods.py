"""Layer moods end to end in a headless Sway (scripts/test-strata.sh): the
Settings page writes strata.conf; the real nexa-signald holds notifications
back by layer; Living darkens and restyles the background; Orbit changes
power through a stand-in broker and puts it back; the stack shows the
layers' names. Screenshots go to $OUT."""
import json
import os
import socket
import subprocess
import sys
import threading
import time
import types

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf, GLib, Gtk  # noqa: E402

OUT = os.environ.get("OUT", "/tmp")
STATE, CONF = "/tmp/strata.state", os.path.expanduser("~/.config/nexa/strata.conf")
BROKER = "/tmp/broker.sock"
SPOT = (1180, 40, 20, 20)
fails = 0


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"), flush=True)
    fails += not ok


def wait(cond, secs=5.0):
    end = time.time() + secs
    while time.time() < end:
        try:
            if cond():
                return True
        except (KeyError, TypeError, IndexError, ValueError, OSError):
            pass
        time.sleep(0.05)
    return False


def stack():
    try:
        return json.load(open(STATE))
    except (OSError, ValueError):
        return {}


def go(n):
    subprocess.run(["swaymsg", "workspace", "number", str(n)], capture_output=True)
    wait(lambda: stack().get("current") == n, 3)
    time.sleep(0.2)


def pixel():
    x, y, w, h = SPOT
    subprocess.run(["grim", "-g", f"{x},{y} {w}x{h}", "/tmp/patch.png"], check=True)
    p = GdkPixbuf.Pixbuf.new_from_file("/tmp/patch.png")
    data, n, rs = p.get_pixels(), p.get_n_channels(), p.get_rowstride()
    tot = [0, 0, 0]
    for j in range(h):
        for i in range(w):
            for c in range(3):
                tot[c] += data[j * rs + i * n + c]
    return [t / (w * h) for t in tot]


def notify(app, summary, critical=False):
    hints = "{'urgency': <byte 2>}" if critical else "{}"
    subprocess.run(["gdbus", "call", "--session", "--dest", "org.freedesktop.Notifications", "--object-path",
                    "/org/freedesktop/Notifications", "--method", "org.freedesktop.Notifications.Notify",
                    app, "0", "", summary, "", "[]", hints, "20000"], capture_output=True, check=True)
    time.sleep(0.15)


def popups():
    """The Signals that pop up now, from nexa-signald itself."""
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(3)
    s.connect(os.path.join(os.environ["XDG_RUNTIME_DIR"], "nexa-signals.sock"))
    s.sendall(b'{"cmd": "subscribe"}\n')
    buf = b""
    while True:
        buf += s.recv(65536)
        for line in buf.split(b"\n"):
            if line.startswith(b'{"event"'):
                s.close()
                return [x["summary"] for x in json.loads(line)["signals"]]


def dismiss_all():
    subprocess.run(["nexa-signalctl", "dismiss", "--all"], capture_output=True)


# ---- a stand-in for the system broker: it notes what Orbit asks
asked = []


def broker():
    srv = socket.socket(socket.AF_UNIX)
    if os.path.exists(BROKER):
        os.remove(BROKER)
    srv.bind(BROKER)
    srv.listen(4)
    current = "schedutil"
    while True:
        c, _ = srv.accept()
        req = json.loads(c.makefile().readline())
        g = req["args"]["governor"]
        asked.append(g)
        c.sendall((json.dumps({"ok": True, "result": {"done": f"CPU governor: {g}", "previous": current}}) + "\n").encode())
        current = g
        c.close()


threading.Thread(target=broker, daemon=True).start()

signald = subprocess.Popen(["nexa-signald"], stderr=open("/tmp/signald.err", "w"))
wait(lambda: os.path.exists(os.path.join(os.environ["XDG_RUNTIME_DIR"], "nexa-signals.sock")), 5)
living = subprocess.Popen(["nexa-living"], stderr=open("/tmp/living.err", "w"))
orbit = subprocess.Popen(["nexa-orbit"], env=dict(os.environ, NEXA_STRATA_STATE=STATE, NEXA_BROKER_SOCKET=BROKER),
                         stderr=open("/tmp/orbit.err", "w"))
check(wait(lambda: stack().get("current") == 1, 8), "Orbit, Living and Signals running, on layer 1")
notify("Chat", "hello from chat")  # so Settings lists Chat
dismiss_all()

# ---- Settings > Layers writes the moods
sys.path[:0] = ["/src/settings-app", "/src/common"]
from nexa_settings import window as SW  # noqa: E402
from nexa_settings.layers_page import LayersPage  # noqa: E402

fake = types.SimpleNamespace(_page=lambda t: SW.SettingsWindow._page(None, t),
                             _field_label=lambda t: SW.SettingsWindow._field_label(None, t),
                             _set_label=lambda l, t, e: SW.SettingsWindow._set_label(None, l, t, e))
css = Gtk.CssProvider()
css.load_from_data(SW.CSS)
from gi.repository import Gdk  # noqa: E402
Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
page = LayersPage(fake)
win = Gtk.Window(title="Layers")
win.set_default_size(820, 760)
sc = Gtk.ScrolledWindow()
sc.add(page.page)
win.add(sc)
win.show_all()


def pump(secs):
    end = time.time() + secs
    while time.time() < end:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        time.sleep(0.02)


pump(1)
check("Chat" in page.cards[1].widgets["mute"], "the page lists apps that sent notifications", list(page.cards[1].widgets["mute"]))
page.preset(2, "focus")                      # layer 3: Work (quiet, fast)
page.cards[1].widgets["mute"]["Chat"].set_active(True)  # layer 2: Chat kept quiet
page.preset(4, "night")                      # layer 5: Night (quiet, darker, no style, saving)
pump(1)
subprocess.run(["grim", os.path.join(OUT, "moods-settings.png")])
text = open(CONF).read()
check("layer.3.name=Work" in text and "layer.3.signals=quiet" in text and "layer.3.power=performance" in text,
      "Work preset saved for layer 3", text)
check("layer.2.mute=Chat" in text, "Chat kept quiet on layer 2", text)
check("layer.5.name=Night" in text and "layer.5.dim=0.45" in text and "layer.5.style=none" in text and
      "layer.5.power=powersave" in text, "Night preset saved for layer 5", text)
win.destroy()
pump(0.3)
check(wait(lambda: stack()["layers"][2]["name"] == "Work" and stack()["layers"][4]["name"] == "Night", 3),
      "Orbit follows the file live (layer names)")

# ---- notifications by layer, from the real service
p1 = pixel()
go(3)
check(wait(lambda: asked == ["performance"], 3), "layer 3 (Work): power set to fast through the broker", asked)
check(wait(lambda: stack()["power_before"] == "schedutil"), "what it was before is remembered", stack().get("power_before"))
subprocess.run(["grim", os.path.join(OUT, "moods-stack.png")])
notify("Chat", "lunch?")
notify("Battery", "5% left", critical=True)
shown = popups()
check(shown == ["5% left"], "on Work only the important one pops up", shown)
dismiss_all()
go(2)
notify("Chat", "again")
notify("Files", "copy done")
shown = popups()
check("copy done" in shown and "again" not in shown, "on layer 2 Chat is kept quiet, Files isn't", shown)
dismiss_all()
go(1)
notify("Chat", "on layer 1")
check(popups() == ["on layer 1"], "on layer 1 everything pops up, as before")
dismiss_all()
check(wait(lambda: asked == ["performance", "schedutil"], 3), "leaving Work: power back to what it was", asked)

# ---- Night: darker, wallpaper only, saving power
go(5)
check(wait(lambda: asked[-1:] == ["powersave"], 3), "layer 5 (Night): power saving", asked)
time.sleep(0.8)
p5 = pixel()
check(sum(p5) < sum(p1) * 0.7, "Night: the background is darker", f"{p1} -> {p5}")
stats = os.path.join(os.environ["XDG_RUNTIME_DIR"], "nexa-living.stats")
check(wait(lambda: abs(json.load(open(stats))["dim"] - 0.45) < 0.01 and json.load(open(stats))["desk_style"] == -1, 7),
      "Living: dim 0.45 and no style on this layer", open(stats).read() if os.path.exists(stats) else "")
time.sleep(1.5)
subprocess.run(["grim", os.path.join(OUT, "moods-night.png")])
go(1)
time.sleep(0.8)
check(abs(sum(pixel()) - sum(p1)) < 30, "back on layer 1: as bright as before")
check(wait(lambda: asked[-1:] == ["schedutil"], 3), "and power back again", asked)

# ---- a mood changed while on its layer applies at once
go(3)
wait(lambda: asked[-1:] == ["performance"], 3)
conf = open(CONF).read().replace("layer.3.power=performance\n", "")
with open(CONF + ".tmp", "w") as f:
    f.write(conf)
os.replace(CONF + ".tmp", CONF)
check(wait(lambda: asked[-1:] == ["schedutil"], 3), "power taken off Work while on it: put back at once", asked)

for p in (orbit, living, signald):
    p.terminate()
err = open("/tmp/orbit.err").read() + open("/tmp/living.err").read() + open("/tmp/signald.err").read()
bad = [l for l in err.splitlines() if ("CRITICAL" in l or "WARNING" in l or "panicked" in l) and "Glycin" not in l]
check(not bad, "no warnings, criticals or panics", "\n".join(bad[:5]))
print(f"{fails} failed")
sys.exit(1 if fails else 0)
