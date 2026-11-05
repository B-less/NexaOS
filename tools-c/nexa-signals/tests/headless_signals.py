"""Signals end to end in a headless Sway: notifications sent over D-Bus
(as notify-send does) to the real nexa-signald, drawn by nexa-signals,
clicked with a real pointer (nexa-vpointer). Run by scripts/test-signals.sh;
screenshots go to $OUT."""
import json
import os
import subprocess
import sys
import time

T, VP, OUT = "/tmp/sig", "/src/build/test/nexa-vpointer", os.environ.get("OUT", "/tmp")
STATE, MON = "/tmp/signals.state", "/tmp/monitor.txt"
PREFS = os.path.expanduser("~/.config/nexa/notifications.json")
W, H = 1280, 800
fails = 0


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"))
    fails += not ok


def state():
    try:
        lines = open(STATE).read().splitlines()
        return json.loads(lines[0]), [json.loads(l) for l in lines[1:]]
    except (OSError, ValueError, IndexError):
        return {}, []


def tabs():
    return state()[1]


def tab(i):
    t = [x for x in tabs() if x["id"] == i]
    return t[0] if t else None


def wait(cond, secs=5.0):
    end = time.time() + secs
    while time.time() < end:
        try:
            if cond():
                return True
        except (KeyError, TypeError, IndexError):
            pass
        time.sleep(0.1)
    return False


def settle():
    last, still = None, 0
    for _ in range(40):
        now = [(x["id"], x.get("rect")) for x in tabs()]
        still = still + 1 if now == last else 0
        if still >= 4:
            return
        last = now
        time.sleep(0.1)


def notify(app, summary, body="", actions=(), timeout=0, urgency=1, replaces=0):
    acts = "[" + ",".join(json.dumps(a) for a in actions) + "]"
    out = subprocess.run(["gdbus", "call", "--session", "-d", "org.freedesktop.Notifications", "-o", "/org/freedesktop/Notifications",
                          "-m", "org.freedesktop.Notifications.Notify", app, str(replaces), "", summary, body, acts,
                          "{'urgency': <byte %d>}" % urgency, str(timeout)], capture_output=True, text=True).stdout
    return int(out.strip().strip("(),").split()[-1])


def ctl(*args):
    return subprocess.run([f"{T}/nexa-signalctl", *args], capture_output=True, text=True).stdout


def on_screen(r):
    """Window coordinates to the screen's: the window is only as big as its
    tabs, 16 px from the side it is anchored to (top: right under the bar,
    which this test doesn't have)."""
    head = state()[0]
    ww, wh = head["window"]
    pos = head.get("position", "top-right")
    x0 = 16 if pos.endswith("-left") else (W - ww) // 2 if pos.endswith("-center") else W - 16 - ww
    y0 = H - wh if pos.startswith("bottom-") else 0
    return [r[0] + x0, r[1] + y0, r[2], r[3]]


def centre(r):
    r = on_screen(r)
    return int(r[0] + r[2] / 2), int(r[1] + r[3] / 2)


def click_at(x, y, button="click"):
    subprocess.run([VP, str(W), str(H), "move", str(x), str(y), "wait", "120", button], check=True, timeout=20)


def click(i, part, button="click"):
    settle()
    x, y = centre(tab(i)[part])
    click_at(x, y, button)


def monitored(text):
    try:
        return text in open(MON).read()
    except OSError:
        return False


def shot(name):
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def start_tabs():
    return subprocess.Popen(["nexa-signals"], executable=f"{T}/nexa-signals", env=dict(os.environ, NEXA_SIGNALS_STATE=STATE),
                            stderr=open("/tmp/signals.err", "a"))


signald = subprocess.Popen(["nexa-signald"], executable=f"{T}/nexa-signald", stderr=open("/tmp/signald.err", "a"))
time.sleep(0.5)
mon = subprocess.Popen(["gdbus", "monitor", "--session", "-d", "org.freedesktop.Notifications"], stdout=open(MON, "w"))
ui = start_tabs()
check(wait(lambda: state()[0].get("connected") == 1, 5), "the tabs connect to nexa-signald")

files = notify("Files", "Copied 3 files", "to <b>Documents</b>", ["default", "", "open", "Open folder"])
check(wait(lambda: tab(files) is not None, 3), "a notification shows as a tab")
check(tab(files)["icon"] == 1, "with an icon (the app's, or the Signals one)")
settle()
r = on_screen(tab(files)["rect"])
check(r[1] == 0 and abs(r[0] + r[2] - (W - 16)) <= 2, "it hangs from the top edge on the right", str(r))
m1 = notify("Mail", "Ama: lunch?", "Are you free at 1?", ["reply", "Reply"])
m2 = notify("Mail", "Kofi: the report", "Attached, have a look")
check(wait(lambda: len(tabs()) == 2 and tab(m2) and tab(m2)["count"] == 2, 3), "two from one app: one tab, newest, counted", str(tabs()))
time.sleep(0.4)
shot("signals-1.png")

# An action button: the sender hears it, then the Signal closes.
click(files, "action1")
check(wait(lambda: monitored(f"ActionInvoked (uint32 {files}, 'open')") and monitored(f"NotificationClosed (uint32 {files}, uint32 2)"), 3),
      "an action reaches the sender (ActionInvoked), then it closes")
check(wait(lambda: tab(files) is None, 2), "and its tab slides away")

# The count shows all of one app.
click(m2, "count_rect")
check(wait(lambda: tab(m1) is not None and tab(m2) is not None, 3), "a click on the count shows all of that app")
time.sleep(0.4)
shot("signals-2-mail.png")
click(m1, "close")
check(wait(lambda: monitored(f"NotificationClosed (uint32 {m1}, uint32 2)") and tab(m1) is None, 3), "x dismisses (reason 2: by the person)")

# A click on the tab itself: the default action.
click(m2, "rect")
check(wait(lambda: monitored(f"NotificationClosed (uint32 {m2}, uint32 2)"), 3), "a click on a tab with no default action just closes it")
d = notify("Store", "Update ready", "Files 3.4", ["default", ""])
wait(lambda: tab(d) is not None, 3)
click(d, "rect")
check(wait(lambda: monitored(f"ActionInvoked (uint32 {d}, 'default')"), 3), "a click on the tab sends its default action")

# A timer that waits while the pointer is on it.
t = notify("Timer", "Short one", timeout=1500)
wait(lambda: tab(t) is not None, 3)
settle()
x, y = centre(tab(t)["rect"])
held = subprocess.Popen([VP, str(W), str(H), "move", str(x), str(y), "wait", "3000", "move", "600", "500", "wait", "300"])
time.sleep(2.6)
check(tab(t) is not None and state()[0]["held"] == 1, "a pointer on it holds its timer (still there after 2.6 s of 1.5)")
held.wait()
check(wait(lambda: tab(t) is None and monitored(f"NotificationClosed (uint32 {t}, uint32 1)"), 4), "it expires once the pointer leaves (reason 1)")

# Quiet: held back; critical ones still show.
ctl("mode", "-a", "do-not-disturb")
q = notify("Chat", "Quiet please")
time.sleep(0.6)
check(tab(q) is None and f'"id":{{"data":{q}' in ctl("list").replace(" ", ""), "Quiet holds a Signal back, but it is in the list")
c = notify("Battery", "Battery at 4 %", "Plug in now", urgency=2)
check(wait(lambda: tab(c) is not None and tab(c)["critical"] == 1, 3), "a critical one shows even in Quiet")
time.sleep(0.4)
shot("signals-3-critical.png")
ctl("mode", "-r", "do-not-disturb")
ctl("dismiss", "--all")
check(wait(lambda: tabs() == [], 3), "dismiss --all clears them")

# Many apps: four tabs, then "+N more".
ids = [notify(f"App{k}", f"Hello from {k}") for k in range(6)]
check(wait(lambda: len(tabs()) == 4 and state()[0]["more"] == 2, 3), "at most four apps, then +2 more", str(state()[0]))
ctl("dismiss", "--all")
wait(lambda: tabs() == [], 3)

# Settings > Notifications: a muted app and the bottom-left corner.
os.makedirs(os.path.dirname(PREFS), exist_ok=True)
with open(PREFS, "w") as f:
    json.dump({"timeout": 6, "position": "bottom-left", "muted_apps": ["Chat"]}, f)
ctl("reload")
mu = notify("Chat", "muted")
b = notify("Files", "Bottom left now", "Settings > Notifications")
check(wait(lambda: tab(b) is not None, 3) and tab(mu) is None, "a muted app doesn't pop up")
settle()
r = on_screen(tab(b)["rect"])
check(abs(r[0] - 16) <= 2 and abs(r[1] + r[3] - H) <= 2, "bottom-left: the tabs move there", str(r))
time.sleep(0.4)
shot("signals-4-bottom-left.png")
check(f'"Bottom left now"' in ctl("history"), "the history has it (for Deck)")

# The service restarts: the tabs find it again.
signald.kill()
signald.wait()
check(wait(lambda: state()[0]["connected"] == 0, 3), "the tabs notice the service went away")
signald = subprocess.Popen(["nexa-signald"], executable=f"{T}/nexa-signald", stderr=open("/tmp/signald.err", "a"))
check(wait(lambda: state()[0]["connected"] == 1, 5), "and connect again when it is back")

for name, p in (("nexa-signald", signald), ("nexa-signals", ui)):
    try:
        pss = next(int(l.split()[1]) for l in open(f"/proc/{p.pid}/smaps_rollup") if l.startswith("Pss:"))
        print(f"INFO {name} PSS {pss / 1024:.1f} MB")
    except (OSError, StopIteration):
        pass
check(ui.poll() is None, "the tabs are still running")
err = "\n".join(l for l in open("/tmp/signals.err").read().splitlines() if "Glycin running without sandbox" not in l)
check("CRITICAL" not in err and "WARNING" not in err, "no GTK warnings", err[-400:])
print(("PASS" if not fails else "FAIL") + f": {fails} failed")
sys.exit(1 if fails else 0)
