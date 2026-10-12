"""Run with: dbus-run-session -- python3 common/tests/test_bluetooth.py

nexa_common.bluetooth against a fake BlueZ (mock_bluez.py) on a session bus."""
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

from nexa_common import bluetooth as bt

bt._bus_factory = lambda: Gio.bus_get_sync(Gio.BusType.SESSION, None)
failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def raises(exc, fn, *a):
    try:
        fn(*a)
    except exc as e:
        return e
    return None


def in_thread(fn, *a):
    """Run a blocking call in a thread while this thread's main loop serves
    the agent (as Settings does: worker thread + GTK main loop)."""
    box = {}

    def run():
        try:
            box["result"] = fn(*a)
        except Exception as e:
            box["error"] = e
    t = threading.Thread(target=run)
    t.start()
    ctx = GLib.MainContext.default()
    while t.is_alive():
        ctx.iteration(False)
        time.sleep(0.01)
    return box


mock = subprocess.Popen([sys.executable, os.path.join(HERE, "mock_bluez.py")])
try:
    bus = bt._bus_factory()
    for _ in range(60):
        try:
            if bt.adapter():
                break
        except bt.BluetoothError:
            time.sleep(0.1)
    a = bt.adapter()
    check("adapter found, starts off", a and a["name"] == "nexa-laptop" and not a["powered"], str(a))

    e = raises(bt.BluetoothError, bt.start_discovery)
    check("searching while off explains why", e and "off" in str(e), str(e))

    bus.call_sync("org.bluez", "/test", "test.Control", "Block", GLib.Variant("(b)", (True,)), None, 0, 5000, None)
    check("rfkill block surfaces as Blocked (Settings then asks the helper to unblock)",
          isinstance(raises(bt.BluetoothError, bt.set_powered, True), bt.Blocked))
    bus.call_sync("org.bluez", "/test", "test.Control", "Block", GLib.Variant("(b)", (False,)), None, 0, 5000, None)
    bt.set_powered(True)
    check("power on", bt.adapter()["powered"])
    bt.start_discovery()
    check("discovery starts", bt.adapter()["discovering"])
    bt.stop_discovery()

    names = [d["name"] for d in bt.devices()]
    check("devices: connected first, nameless beacons hidden", names == ["MX Master", "WH-1000XM5"], str(names))

    asked = []
    answer = {"ok": False}

    def ask(kind, device, value):
        asked.append((kind, device, value))
        return answer["ok"]
    bt.register_agent(ask)
    hp = next(d for d in bt.devices() if d["name"] == "WH-1000XM5")

    box = in_thread(bt.pair, hp["path"])
    check("pairing asks the user to confirm the code, with the device name",
          asked and asked[-1] == ("confirm", "WH-1000XM5", "123456"), str(asked))
    check("saying no fails with a clear message", "error" in box and "code didn't match" in str(box["error"]), str(box))

    answer["ok"] = True
    box = in_thread(bt.pair, hp["path"])
    hp = next(d for d in bt.devices() if d["name"] == "WH-1000XM5")
    check("saying yes pairs, trusts and connects", "error" not in box and hp["paired"] and hp["trusted"] and hp["connected"],
          str(box) + str(hp))
    bt.disconnect(hp["path"])
    check("disconnect", not next(d for d in bt.devices() if d["name"] == "WH-1000XM5")["connected"])
    bt.forget(hp["path"])
    check("forget removes it", [d["name"] for d in bt.devices()] == ["MX Master"])
finally:
    mock.terminate()
    mock.wait()

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
