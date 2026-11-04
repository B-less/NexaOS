"""Deck (nexa-deck) in a headless Sway against fake iwd, BlueZ, MPRIS,
broker, wpctl and makoctl, clicked with a real pointer (nexa-vpointer).
Run by scripts/test-deck.sh; screenshots go to $OUT."""
import json
import os
import subprocess
import sys
import time

DECK, VP, OUT = "/tmp/nexa-deck", "/src/build/test/nexa-vpointer", os.environ.get("OUT", "/tmp")
STATE, LOG = "/tmp/deck.state", "/tmp/fakes.log"
W, H = 1280, 800
fails = 0


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"))
    fails += not ok


def state():
    try:
        lines = open(STATE).read().splitlines()
        return json.loads(lines[0]), {d["card"]: d for d in map(json.loads, lines[1:])}
    except (OSError, ValueError, IndexError):
        return {}, {}


def card(k):
    return state()[1].get(k, {})


def wait(cond, secs=6.0):
    end = time.time() + secs
    while time.time() < end:
        try:
            if cond():
                return True
        except (KeyError, TypeError, IndexError):
            pass
        time.sleep(0.1)
    return False


def centre(r, fx=0.5):
    return int(r[0] + r[2] * fx), int(r[1] + r[3] / 2)


def click_at(x, y, button="click"):
    subprocess.run([VP, str(W), str(H), "move", str(x), str(y), "wait", "120", button], check=True, timeout=20)


def settle():
    """Until the cards stop moving (a list rebuilt, a card opening)."""
    last, still = None, 0
    for _ in range(40):
        now = [card(k).get("rect") for k in ALL]
        still = still + 1 if now == last else 0
        if still >= 4:
            return
        last = now
        time.sleep(0.1)


def click(k, part, fx=0.5):
    settle()
    x, y = centre(card(k)[part], fx)
    assert 0 <= x < W and 0 <= y < H, f"{k} {part} is off the screen at {x},{y}"
    click_at(x, y)


def logged(text):
    try:
        return text in open(LOG).read()
    except OSError:
        return False


def shot(name):
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def deck(*args, wait_s=None):
    env = dict(os.environ, NEXA_DECK_STATE=STATE)
    return subprocess.Popen(["nexa-deck", *args], executable=DECK, env=env, stderr=open("/tmp/deck.err", "a"))


def sway_outputs():
    return subprocess.run(["swaymsg", "-t", "get_outputs"], capture_output=True, text=True).stdout


def running():
    return subprocess.run(["pgrep", "-x", "nexa-deck"], capture_output=True).returncode == 0


# Deck fills the screen: Sway must have its output before Deck maps (in a
# session it always has).
wait(lambda: '"active": true' in sway_outputs(), 10)
time.sleep(1.0)
p = deck()
ALL = ["now", "wifi", "bluetooth", "sound", "brightness", "vpn", "quiet", "playing", "signals"]
check(wait(lambda: all(card(k).get("shown") for k in ALL), 8), "every card appears (Wi-Fi, Bluetooth, sound, brightness, VPN, Quiet, "
      "Playing, Signals and Now)", str({k: card(k).get("shown") for k in ALL}))
# the slide-in: done when the last card has reached the edge
wait(lambda: all(card(k)["rect"][0] + card(k)["rect"][2] >= W - 12 for k in ALL if "rect" in card(k)), 4)
time.sleep(0.3)
r = card("now").get("rect", [0, 0, 0, 0])
check(r[0] + r[2] >= W - 12 and r[1] <= 12, "the cards sit at the right edge, at the top", str(r))
if os.environ.get("DECK_DEBUG"):
    print(open(STATE).read()[:400])
    print(subprocess.run(["sh", "-c", "ps -o pid,args | grep -v grep | grep -i deck"], capture_output=True, text=True).stdout)
check(card("now")["sub"] == "" and wait(lambda: state()[0]["notes"] == 2), "two notifications read from mako", str(state()[0]))
check(card("wifi")["sub"] == "Not connected" and state()[0]["networks"] == 4, "Wi-Fi: not connected, 4 networks", str(card("wifi")))
check(card("bluetooth")["sub"] == "Off" and not card("bluetooth")["lit"], "Bluetooth: off")
check(card("sound")["sub"] == "45 %", "volume 45 %", card("sound").get("sub", ""))
check(card("brightness")["sub"] == "50 %", "brightness 50 %", card("brightness").get("sub", ""))
check(card("vpn")["sub"] == "Off · Sweden", "VPN: off, Sweden first", card("vpn").get("sub", ""))
check(card("playing")["title"] == "Café Nights" and card("playing")["sub"] == "Nexa Ensemble", "Playing: title and artist")
check(card("signals")["sub"].startswith("2 · Store: Update ready"), "Signals: count and the newest", card("signals").get("sub", ""))
shot("deck-1-open.png")

# Wi-Fi: open the card in place, then a saved network connects there.
click("wifi", "head")
check(wait(lambda: card("wifi")["open"] == 1, 3), "a click on Wi-Fi opens the card in place")
time.sleep(0.5)
shot("deck-2-wifi.png")
settle()
row = next(r for r in card("wifi")["rows"] if r["name"] == "SavedNet")
click_at(*centre(row["rect"]))
check(wait(lambda: card("wifi")["sub"] == "SavedNet" and card("wifi")["lit"], 6), "a saved network connects from Deck",
      card("wifi").get("sub", ""))
click("wifi", "head")  # and closed again
check(wait(lambda: card("wifi")["open"] == 0, 3), "a second click closes the Wi-Fi card")

# Bluetooth switch: the adapter powers on; the paired mouse shows as connected.
click("bluetooth", "switch")
check(wait(lambda: card("bluetooth")["sub"] == "MX Master connected" and card("bluetooth")["lit"], 5), "Bluetooth switch: on, mouse connected",
      card("bluetooth").get("sub", "") + " " + str(card("bluetooth").get("switch")) + " " + state()[0].get("error", ""))

# VPN switch: the broker is asked, with the tunnel's name.
click("vpn", "switch")
check(wait(lambda: logged('broker {"action": "vpn_up", "args": {"name": "Sweden"}}') and card("vpn")["sub"] == "Connected: Sweden", 5),
      "VPN switch: vpn_up Sweden through the broker", card("vpn").get("sub", ""))
check(card("vpn")["lit"] == 1, "VPN card lit while connected")

# Quiet
click("quiet", "switch")
check(wait(lambda: logged("makoctl mode -a do-not-disturb") and state()[0]["quiet"] == 1 and card("quiet")["lit"], 4), "Quiet switch: mako's do-not-disturb")

# Volume slider: a click at a quarter of its width.
settle()
s = card("sound")["slider"]
click_at(int(s[0] + s[2] * 0.25), int(s[1] + s[3] / 2))
check(wait(lambda: logged("wpctl set-volume @DEFAULT_AUDIO_SINK@") and 15 <= state()[0]["volume"] <= 35, 4), "the volume slider sets the volume",
      str(state()[0].get("volume")))

# Brightness slider: through the broker.
settle()
s = card("brightness")["slider"]
click_at(int(s[0] + s[2] * 0.8), int(s[1] + s[3] / 2))
check(wait(lambda: logged('"action": "set_brightness"'), 4), "the brightness slider asks the broker")

# Play
click("playing", "play")
check(wait(lambda: logged("player PlayPause") and state()[0]["playing"] == 1 and card("playing")["lit"], 4), "Play reaches the player",
      str(state()[0].get("playing")))

# A second `nexa-deck signals`: the running Deck opens Signals.
deck("signals").wait(timeout=10)
check(wait(lambda: card("signals")["open"] == 1, 3), "nexa-deck signals opens the Signals card in the running Deck")
settle()
check(card("signals")["rect"][1] + card("signals")["rect"][3] <= H, "the opened card scrolls into view", str(card("signals")["rect"]))
time.sleep(0.6)
shot("deck-3-busy.png")
rect = card("signals")["rect"]
click("signals", "clear")
check(wait(lambda: logged("makoctl dismiss --all") and state()[0]["notes"] == 0, 4), "Clear all empties Signals",
      str(state()[0].get("notes")))
check(card("signals")["sub"] == "You're all caught up", "and says so")

# A third run closes it, and the process ends.
deck().wait(timeout=10)
check(wait(lambda: p.poll() is not None, 4), "running nexa-deck again closes Deck and it exits")

# Open again, click outside the cards: closes.
p = deck("wifi")
check(wait(lambda: card("wifi").get("open") == 1 and state()[0].get("closing") == 0, 6), "nexa-deck wifi opens on Wi-Fi")
time.sleep(0.8)
try:
    pss = next(int(l.split()[1]) for l in open(f"/proc/{p.pid}/smaps_rollup") if l.startswith("Pss:"))
    print(f"INFO nexa-deck PSS {pss / 1024:.1f} MB while open")
except (OSError, StopIteration):
    pass
click_at(300, 500)
check(wait(lambda: p.poll() is not None, 4), "a click outside the cards closes Deck")
# Glycin (the image loader) can't sandbox itself inside this container; that one line is the container's.
err = "\n".join(l for l in open("/tmp/deck.err").read().splitlines() if "Glycin running without sandbox" not in l)
check("CRITICAL" not in err and "WARNING" not in err, "no GTK warnings", err[-500:])
print(("PASS" if not fails else "FAIL") + f": {fails} failed")
sys.exit(1 if fails else 0)
