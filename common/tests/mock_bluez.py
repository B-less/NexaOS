"""Minimal fake of BlueZ's D-Bus API (session bus), just enough for
test_bluetooth.py. Pair() asks the registered agent to confirm a code."""
import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

V = GLib.Variant
NAME = "org.bluez"
HCI = "/org/bluez/hci0"
HEADPHONES = HCI + "/dev_00_1B_66_AA_BB_CC"
MOUSE = HCI + "/dev_DC_2C_26_11_22_33"
BEACON = HCI + "/dev_7A_11_22_33_44_55"

state = {"blocked": False, "agent": None}
props = {
    HCI: ("org.bluez.Adapter1", {"Address": "AA:AA:AA:AA:AA:01", "Alias": "nexa-laptop", "Name": "nexa-laptop",
                                 "Powered": False, "Discovering": False}),
    HEADPHONES: ("org.bluez.Device1", {"Address": "00:1B:66:AA:BB:CC", "Name": "WH-1000XM5", "Alias": "WH-1000XM5",
                                       "Icon": "audio-headphones", "Paired": False, "Connected": False,
                                       "Trusted": False, "RSSI": -50}),
    MOUSE: ("org.bluez.Device1", {"Address": "DC:2C:26:11:22:33", "Name": "MX Master", "Alias": "MX Master",
                                  "Icon": "input-mouse", "Paired": True, "Connected": True,
                                  "Trusted": True, "RSSI": -60}),
    BEACON: ("org.bluez.Device1", {"Address": "7A:11:22:33:44:55", "Alias": "7A-11-22-33-44-55",
                                   "Paired": False, "Connected": False, "Trusted": False, "RSSI": -80}),
}
SIG = {"Address": "s", "Alias": "s", "Name": "s", "Icon": "s", "Powered": "b", "Discovering": "b",
       "Paired": "b", "Connected": "b", "Trusted": "b", "RSSI": "n"}


def variant_props(path):
    return {k: V(SIG[k], v) for k, v in props[path][1].items()}


def xml(iface, methods):
    return Gio.DBusNodeInfo.new_for_xml(f'<node><interface name="{iface}">{"".join(methods)}</interface></node>').interfaces[0]


PROPS_XML = xml("org.freedesktop.DBus.Properties", [
    '<method name="Get"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>',
    '<method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method>',
    '<method name="Set"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/></method>'])


def handle(conn, sender, path, iface, method, params, inv):
    args = params.unpack()
    if iface == "org.freedesktop.DBus.ObjectManager":
        objs = {p: {props[p][0]: variant_props(p)} for p in props}
        objs["/org/bluez"] = {"org.bluez.AgentManager1": {}}
        return inv.return_value(V("(a{oa{sa{sv}}})", (objs,)))
    if iface == "org.freedesktop.DBus.Properties":
        if method == "GetAll":
            return inv.return_value(V("(a{sv})", (variant_props(path),)))
        if method == "Get":
            return inv.return_value(V("(v)", (variant_props(path)[args[1]],)))
        name, value = args[1], args[2]
        if path == HCI and name == "Powered" and value and state["blocked"]:
            return inv.return_dbus_error("org.bluez.Error.Blocked", "Blocked through rfkill")
        props[path][1][name] = value
        return inv.return_value(None)
    if iface == "org.bluez.AgentManager1":
        if method == "RegisterAgent":
            state["agent"] = (sender, args[0])
        return inv.return_value(None)
    if iface == "org.bluez.Adapter1":
        a = props[HCI][1]
        if not a["Powered"]:
            return inv.return_dbus_error("org.bluez.Error.NotReady", "Resource Not Ready")
        if method in ("StartDiscovery", "StopDiscovery"):
            a["Discovering"] = method == "StartDiscovery"
        elif method == "RemoveDevice":
            props.pop(args[0], None)
        return inv.return_value(None)
    if iface == "org.bluez.Device1":
        d = props[path][1]
        if method == "Pair":
            if state["agent"] is None:
                return inv.return_dbus_error("org.bluez.Error.AuthenticationFailed", "No agent")
            agent_bus, agent_path = state["agent"]

            def answered(c, result):
                try:
                    c.call_finish(result)
                except GLib.Error:
                    return inv.return_dbus_error("org.bluez.Error.AuthenticationFailed", "Authentication Failed")
                d["Paired"] = True
                inv.return_value(None)
            conn.call(agent_bus, agent_path, "org.bluez.Agent1", "RequestConfirmation",
                      V("(ou)", (path, 123456)), None, Gio.DBusCallFlags.NONE, 10000, None, answered)
            return
        if method == "Connect":
            if not d["Paired"]:
                return inv.return_dbus_error("org.bluez.Error.Failed", "br-connection-create-socket")
            d["Connected"] = True
        elif method == "Disconnect":
            d["Connected"] = False
        return inv.return_value(None)
    inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method)


def on_bus(conn, _name):
    conn.register_object("/", xml("org.freedesktop.DBus.ObjectManager", [
        '<method name="GetManagedObjects"><arg type="a{oa{sa{sv}}}" direction="out"/></method>']), handle, None, None)
    conn.register_object("/org/bluez", xml("org.bluez.AgentManager1", [
        '<method name="RegisterAgent"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>',
        '<method name="RequestDefaultAgent"><arg type="o" direction="in"/></method>']), handle, None, None)
    conn.register_object(HCI, xml("org.bluez.Adapter1", [
        '<method name="StartDiscovery"/>', '<method name="StopDiscovery"/>',
        '<method name="RemoveDevice"><arg type="o" direction="in"/></method>']), handle, None, None)
    conn.register_object(HCI, PROPS_XML, handle, None, None)
    for p in (HEADPHONES, MOUSE, BEACON):
        conn.register_object(p, xml("org.bluez.Device1", [
            '<method name="Pair"/>', '<method name="Connect"/>', '<method name="Disconnect"/>']), handle, None, None)
        conn.register_object(p, PROPS_XML, handle, None, None)
    # test hook: toggle the rfkill block
    conn.register_object("/test", xml("test.Control", ['<method name="Block"><arg type="b" direction="in"/></method>']),
                         lambda c, s, p, i, m, params, inv: (state.update(blocked=params.unpack()[0]), inv.return_value(None)),
                         None, None)


Gio.bus_own_name(Gio.BusType.SESSION, NAME, Gio.BusNameOwnerFlags.NONE, on_bus, None, None)
GLib.MainLoop().run()
