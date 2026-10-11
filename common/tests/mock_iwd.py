"""Minimal fake of iwd's D-Bus API (session bus), just enough for test_wifi.py."""
import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

V = GLib.Variant
NAME = "net.connman.iwd"
ROOT = "/net/connman/iwd"
DEV = ROOT + "/0/3"

NETS = {
    "HomeWiFi": {"type": "psk", "password": "correcthorse", "known": False, "dbm": -45},
    "CafeOpen": {"type": "open", "password": None, "known": False, "dbm": -70},
    "CorpNet": {"type": "8021x", "password": None, "known": False, "dbm": -55},
    "SavedNet": {"type": "psk", "password": "savedpass123", "known": True, "dbm": -60},
}
state = {"scanning": False, "connected": None, "agent": None}


def net_path(name):
    return f"{DEV}/{name.encode().hex()}_{NETS[name]['type']}"


def known_path(name):
    return f"{ROOT}/{name.encode().hex()}_{NETS[name]['type']}"


PATH_TO_NET = {net_path(n): n for n in NETS}
PATH_TO_KNOWN = {known_path(n): n for n in NETS}


def managed_objects():
    dev_station = {
        "State": V("s", "connected" if state["connected"] else "disconnected"),
        "Scanning": V("b", state["scanning"]),
    }
    if state["connected"]:
        dev_station["ConnectedNetwork"] = V("o", net_path(state["connected"]))
    objs = {
        DEV: {
            "net.connman.iwd.Device": {
                "Name": V("s", "wlan0"), "Address": V("s", "02:00:00:00:00:00"),
                "Powered": V("b", True), "Mode": V("s", "station"),
            },
            "net.connman.iwd.Station": dev_station,
        }
    }
    for name, n in NETS.items():
        props = {
            "Name": V("s", name), "Type": V("s", n["type"]),
            "Connected": V("b", state["connected"] == name), "Device": V("o", DEV),
        }
        if n["known"]:
            props["KnownNetwork"] = V("o", known_path(name))
            objs[known_path(name)] = {"net.connman.iwd.KnownNetwork": {
                "Name": V("s", name), "Type": V("s", n["type"]), "AutoConnect": V("b", True),
            }}
        objs[net_path(name)] = {"net.connman.iwd.Network": props}
    return objs


def xml(iface, methods):
    body = "".join(methods)
    return Gio.DBusNodeInfo.new_for_xml(f'<node><interface name="{iface}">{body}</interface></node>').interfaces[0]


def handle(conn, sender, path, iface, method, params, inv):
    if method == "GetManagedObjects":
        inv.return_value(V("(a{oa{sa{sv}}})", (managed_objects(),)))
    elif method == "RegisterAgent":
        if state["agent"] is not None:
            inv.return_dbus_error("net.connman.iwd.AlreadyExists", "agent exists")
            return
        state["agent"] = (sender, params.unpack()[0])
        inv.return_value(None)
    elif method == "UnregisterAgent":
        state["agent"] = None
        inv.return_value(None)
    elif method == "Scan":
        if state["scanning"]:
            inv.return_dbus_error("net.connman.iwd.Busy", "busy")
            return
        state["scanning"] = True
        GLib.timeout_add(800, lambda: state.update(scanning=False) or False)
        inv.return_value(None)
    elif method == "Disconnect":
        if not state["connected"]:
            inv.return_dbus_error("net.connman.iwd.NotConnected", "not connected")
            return
        state["connected"] = None
        inv.return_value(None)
    elif method == "GetOrderedNetworks":
        ordered = sorted(NETS, key=lambda n: (state["connected"] != n, -NETS[n]["dbm"]))
        inv.return_value(V("(a(on))", ([(net_path(n), NETS[n]["dbm"] * 100) for n in ordered],)))
    elif method == "Connect":
        connect(conn, path, inv)
    elif method == "Forget":
        name = PATH_TO_KNOWN[path]
        NETS[name]["known"] = False
        if state["connected"] == name:
            state["connected"] = None
        inv.return_value(None)


def connect(conn, path, inv):
    name = PATH_TO_NET[path]
    n = NETS[name]

    def succeed():
        n["known"] = True
        state["connected"] = name
        inv.return_value(None)
        return False

    if n["type"] == "8021x":
        inv.return_dbus_error("net.connman.iwd.NotSupported", "unsupported")
    elif n["type"] == "open" or n["known"]:
        GLib.timeout_add(300, succeed)
    elif state["agent"] is None:
        inv.return_dbus_error("net.connman.iwd.NoAgent", "no agent")
    else:
        agent_sender, agent_path = state["agent"]

        def on_passphrase(source, res):
            try:
                pw = source.call_finish(res).unpack()[0]
            except GLib.Error:
                inv.return_dbus_error("net.connman.iwd.Aborted", "agent failed")
                return
            if pw == n["password"]:
                GLib.timeout_add(300, succeed)
            else:
                inv.return_dbus_error("net.connman.iwd.Failed", "Operation failed")

        conn.call(
            agent_sender, agent_path, "net.connman.iwd.Agent", "RequestPassphrase",
            V("(o)", (path,)), GLib.VariantType("(s)"), Gio.DBusCallFlags.NONE, 30000, None,
            on_passphrase,
        )


def on_bus(conn, _name):
    conn.register_object("/", xml("org.freedesktop.DBus.ObjectManager", [
        '<method name="GetManagedObjects"><arg type="a{oa{sa{sv}}}" direction="out"/></method>']), handle, None, None)
    conn.register_object(ROOT, xml("net.connman.iwd.AgentManager", [
        '<method name="RegisterAgent"><arg type="o" direction="in"/></method>',
        '<method name="UnregisterAgent"><arg type="o" direction="in"/></method>']), handle, None, None)
    conn.register_object(DEV, xml("net.connman.iwd.Station", [
        '<method name="Scan"/>', '<method name="Disconnect"/>',
        '<method name="GetOrderedNetworks"><arg type="a(on)" direction="out"/></method>']), handle, None, None)
    for p in PATH_TO_NET:
        conn.register_object(p, xml("net.connman.iwd.Network", ['<method name="Connect"/>']), handle, None, None)
    for p in PATH_TO_KNOWN:
        conn.register_object(p, xml("net.connman.iwd.KnownNetwork", ['<method name="Forget"/>']), handle, None, None)


Gio.bus_own_name(Gio.BusType.SESSION, NAME, Gio.BusNameOwnerFlags.NONE, on_bus, None, None)
GLib.MainLoop().run()
