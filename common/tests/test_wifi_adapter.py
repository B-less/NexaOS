"""Run with: python3 common/tests/test_wifi_adapter.py

Adapter choice with mac80211_hwsim's fake radios present, as seen on an HP
EliteBook 840 G6: hwsim's wlan0 (02:00:00:00:00:00) sorted ahead of the
real Intel card (wlan2), so the Network app scanned a radio with nothing
around it."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from nexa_common import wifi

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

def device(name, address):
    return {wifi.IFACE_DEVICE: {"Name": name, "Address": address, "Powered": True},
            wifi.IFACE_STATION: {"State": "disconnected", "Scanning": False}}

objects = {
    "/net/connman/iwd/0/1": device("wlan0", "02:00:00:00:00:00"),
    "/net/connman/iwd/1/1": device("wlan1", "02:00:00:00:01:00"),
    "/net/connman/iwd/2/1": device("wlan2", "28:7f:cf:ab:38:7b"),
}
check("real card is picked over hwsim radios", wifi._pick_device(objects)["name"] == "wlan2",
      wifi._pick_device(objects)["name"])
check("real card is listed first", [d["name"] for d in wifi._station_devices(objects)] == ["wlan2", "wlan0", "wlan1"])
del objects["/net/connman/iwd/2/1"]
check("with only simulated radios (VM tests), one is still used", wifi._pick_device(objects)["name"] == "wlan0")
check("a named adapter is still honoured", wifi._pick_device(objects, "wlan1")["name"] == "wlan1")

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
