"""Run with: dbus-run-session -- python3 common/tests/test_wifi.py"""
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio

from nexa_common import wifi

wifi._bus_factory = lambda: Gio.bus_get_sync(Gio.BusType.SESSION, None)

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def raises(exc, fn, *a, **kw):
    try:
        fn(*a, **kw)
    except exc as e:
        return e
    except Exception as e:
        return None if not print("   unexpected", type(e).__name__, e) else None
    return None


mock = subprocess.Popen([sys.executable, os.path.join(HERE, "mock_iwd.py")])
try:
    for _ in range(60):
        try:
            wifi._objects()
            break
        except wifi.WifiError:
            time.sleep(0.1)
    else:
        raise SystemExit("mock iwd never came up")

    st = wifi.status()
    check("status: one adapter wlan0", [a["name"] for a in st["adapters"]] == ["wlan0"])
    check("status: not connected", st["connected_to"] is None)

    nets = wifi.list_networks(do_scan=True)
    by = {n["ssid"]: n for n in nets}
    check("scan lists all 4 networks", set(by) == {"HomeWiFi", "CafeOpen", "CorpNet", "SavedNet"}, str(list(by)))
    check("saved network flagged known", by["SavedNet"]["known"] and not by["HomeWiFi"]["known"])
    check("signal -45dBm -> 100%", by["HomeWiFi"]["signal"] == 100, str(by["HomeWiFi"]["signal"]))
    check("signal -70dBm -> 60%", by["CafeOpen"]["signal"] == 60, str(by["CafeOpen"]["signal"]))
    check("strongest network listed first", nets[0]["ssid"] == "HomeWiFi")

    check("secured+unknown without password -> PassphraseRequired",
          raises(wifi.PassphraseRequired, wifi.connect, "HomeWiFi") is not None)
    e = raises(wifi.WifiError, wifi.connect, "HomeWiFi", "short")
    check("too-short password rejected client-side", e is not None and "8-63" in str(e), str(e))
    e = raises(wifi.WifiError, wifi.connect, "HomeWiFi", "wrongpassword1")
    check("wrong password -> friendly failure", e is not None and "password" in str(e).lower(), str(e))
    check("wrong password did not connect", wifi.status()["connected_to"] is None)

    r = wifi.connect("HomeWiFi", "correcthorse")
    check("correct password connects (agent callback works)", r == {"connected_to": "HomeWiFi"}, str(r))
    check("status shows connection", wifi.status()["connected_to"] == "HomeWiFi")
    now = {n["ssid"]: n for n in wifi.list_networks()}
    check("network now known+connected", now["HomeWiFi"]["known"] and now["HomeWiFi"]["connected"])

    check("forget saved network", wifi.forget("HomeWiFi") == {"forgot": "HomeWiFi"})
    check("forgetting again fails cleanly", raises(wifi.WifiError, wifi.forget, "HomeWiFi") is not None)
    r = wifi.connect("HomeWiFi", "correcthorse")
    check("agent can be re-registered after a previous connect", r["connected_to"] == "HomeWiFi")

    result = {}
    def worker():
        try:
            result["r"] = wifi.connect("SavedNet")
        except Exception as ex:
            result["e"] = ex
    t = threading.Thread(target=worker)
    t.start(); t.join(20)
    check("connect from a worker thread (GUI use)", result.get("r") == {"connected_to": "SavedNet"}, str(result))

    check("open network connects with no password", wifi.connect("CafeOpen")["connected_to"] == "CafeOpen")
    e = raises(wifi.WifiError, wifi.connect, "CorpNet")
    check("enterprise network refused with clear message", e is not None and "Enterprise" in str(e), str(e))
    e = raises(wifi.WifiError, wifi.connect, "NoSuchNet")
    check("unknown network -> not found", e is not None and "wasn't found" in str(e), str(e))

    check("disconnect works", wifi.disconnect() == {"disconnected": "wlan0"})
    check("status after disconnect", wifi.status()["connected_to"] is None)
    check("list_known reports saved networks", "SavedNet" in [k["ssid"] for k in wifi.list_known()])

    mock.terminate(); mock.wait(5)
    e = raises(wifi.WifiError, wifi.status)
    check("iwd not running -> friendly error", e is not None and "isn't running" in str(e), str(e))
finally:
    if mock.poll() is None:
        mock.kill()

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
