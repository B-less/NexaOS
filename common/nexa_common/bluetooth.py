"""Bluetooth through BlueZ's D-Bus API (org.bluez).

Used by Settings. BlueZ's D-Bus policy lets local desktop users talk to it,
so nothing here needs root, except lifting an rfkill block, which goes
through the Settings helper. Calls block, so GUI callers should run them off
the main thread; register_agent() must be called on the main thread.
"""
import re

import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

BUS_NAME = "org.bluez"
IFACE_ADAPTER = "org.bluez.Adapter1"
IFACE_DEVICE = "org.bluez.Device1"
IFACE_AGENT_MANAGER = "org.bluez.AgentManager1"
AGENT_PATH = "/org/nexaos/bluetooth_agent"

_AGENT_XML = """
<node>
  <interface name="org.bluez.Agent1">
    <method name="Release"/>
    <method name="RequestPinCode"><arg type="o" direction="in"/><arg type="s" direction="out"/></method>
    <method name="DisplayPinCode"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
    <method name="RequestPasskey"><arg type="o" direction="in"/><arg type="u" direction="out"/></method>
    <method name="DisplayPasskey"><arg type="o" direction="in"/><arg type="u" direction="in"/><arg type="q" direction="in"/></method>
    <method name="RequestConfirmation"><arg type="o" direction="in"/><arg type="u" direction="in"/></method>
    <method name="RequestAuthorization"><arg type="o" direction="in"/></method>
    <method name="AuthorizeService"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
    <method name="Cancel"/>
  </interface>
</node>
"""

_FRIENDLY_ERRORS = {
    "org.freedesktop.DBus.Error.ServiceUnknown": "The Bluetooth service isn't running.",
    "org.freedesktop.DBus.Error.NameHasNoOwner": "The Bluetooth service isn't running.",
    "org.freedesktop.DBus.Error.AccessDenied": "Not allowed to control Bluetooth from this account.",
    "org.bluez.Error.Blocked": "Bluetooth is switched off by the airplane-mode (rfkill) switch.",
    "org.bluez.Error.AuthenticationFailed": "Pairing failed: the code didn't match or was rejected.",
    "org.bluez.Error.AuthenticationCanceled": "Pairing was cancelled.",
    "org.bluez.Error.AuthenticationRejected": "Pairing was rejected.",
    "org.bluez.Error.AuthenticationTimeout": "The device took too long to answer.",
    "org.bluez.Error.ConnectionAttemptFailed": "Couldn't connect. Is the device on and nearby?",
    "org.bluez.Error.AlreadyExists": "This device is already paired.",
    "org.bluez.Error.InProgress": "Already working on it, give it a moment.",
    "org.bluez.Error.NotReady": "Bluetooth is off.",
    "org.bluez.Error.Failed": "The device didn't respond. Is it on and in pairing mode?",
}
_REMOTE_ERROR_RE = re.compile(r"GDBus\.Error:([^:\s]+):\s*(.*)", re.S)


class BluetoothError(Exception):
    pass


class Blocked(BluetoothError):
    """The adapter is rfkill-blocked; unblocking needs the Settings helper."""


_bus_factory = lambda: Gio.bus_get_sync(Gio.BusType.SYSTEM, None)


def _friendly(err):
    m = _REMOTE_ERROR_RE.search(err.message or "")
    if m:
        name, detail = m.group(1), m.group(2).strip()
        return _FRIENDLY_ERRORS.get(name) or detail or name
    return err.message or "Unknown Bluetooth error"


def _bus():
    try:
        return _bus_factory()
    except GLib.Error as e:
        raise BluetoothError(f"Can't reach the system bus: {e.message}")


def _call(path, iface, method, params=None, timeout_ms=10000, bus=None):
    try:
        return (bus or _bus()).call_sync(BUS_NAME, path, iface, method, params, None,
                                          Gio.DBusCallFlags.NONE, timeout_ms, None)
    except GLib.Error as e:
        if "org.bluez.Error.Blocked" in (e.message or ""):
            raise Blocked(_friendly(e))
        raise BluetoothError(_friendly(e))


def _objects(bus=None):
    reply = _call("/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", bus=bus)
    return reply.unpack()[0]


def _set_prop(path, iface, name, value, bus=None):
    _call(path, "org.freedesktop.DBus.Properties", "Set",
          GLib.Variant("(ssv)", (iface, name, value)), bus=bus)


def adapter(bus=None):
    """{path, name, address, powered, discovering} for the first adapter, or None."""
    for path, ifaces in sorted(_objects(bus).items()):
        a = ifaces.get(IFACE_ADAPTER)
        if a is not None:
            return {"path": path, "name": a.get("Alias") or a.get("Name") or "Bluetooth",
                    "address": a.get("Address", ""), "powered": bool(a.get("Powered")),
                    "discovering": bool(a.get("Discovering"))}
    return None


def _require_adapter(bus=None):
    a = adapter(bus)
    if a is None:
        raise BluetoothError("No Bluetooth adapter found.")
    return a


def set_powered(on, bus=None):
    a = _require_adapter(bus)
    _set_prop(a["path"], IFACE_ADAPTER, "Powered", GLib.Variant("b", bool(on)), bus=bus)


def devices(bus=None):
    """Known and nearby devices: paired/connected first, then by signal."""
    out = []
    for path, ifaces in _objects(bus).items():
        d = ifaces.get(IFACE_DEVICE)
        if d is None:
            continue
        name = d.get("Alias") or d.get("Name") or d.get("Address", "")
        if not d.get("Name") and not d.get("Paired") and re.fullmatch(r"([0-9A-F]{2}-){5}[0-9A-F]{2}", name):
            continue  # anonymous beacons: an address with no name is just noise in the list
        out.append({"path": path, "name": name, "address": d.get("Address", ""),
                    "icon": d.get("Icon", "bluetooth"), "paired": bool(d.get("Paired")),
                    "connected": bool(d.get("Connected")), "trusted": bool(d.get("Trusted")),
                    "rssi": d.get("RSSI")})
    out.sort(key=lambda d: (not d["connected"], not d["paired"], -(d["rssi"] or -999), d["name"].lower()))
    return out


def start_discovery(bus=None):
    a = _require_adapter(bus)
    if not a["discovering"]:
        _call(a["path"], IFACE_ADAPTER, "StartDiscovery", bus=bus)


def stop_discovery(bus=None):
    a = adapter(bus)
    if a and a["discovering"]:
        try:
            _call(a["path"], IFACE_ADAPTER, "StopDiscovery", bus=bus)
        except BluetoothError:
            pass  # someone else's discovery session; harmless


def pair(path, bus=None):
    """Pair, trust (so it reconnects by itself) and connect."""
    _call(path, IFACE_DEVICE, "Pair", timeout_ms=90000, bus=bus)
    _set_prop(path, IFACE_DEVICE, "Trusted", GLib.Variant("b", True), bus=bus)
    connect(path, bus=bus)


def connect(path, bus=None):
    _call(path, IFACE_DEVICE, "Connect", timeout_ms=30000, bus=bus)


def disconnect(path, bus=None):
    _call(path, IFACE_DEVICE, "Disconnect", bus=bus)


def forget(path, bus=None):
    a = _require_adapter(bus)
    _call(a["path"], IFACE_ADAPTER, "RemoveDevice", GLib.Variant("(o)", (path,)), bus=bus)


def register_agent(ask, bus=None):
    """Answer BlueZ's pairing questions. ask(kind, device_path, value) is
    called on the main thread:
      kind "confirm": value is the 6-digit code; return True to accept.
      kind "display": value is the code to type on the device; return ignored.
      kind "pin":     return the PIN (str) or None to cancel.
    Returns the registration id (keep the bus alive while it is in use)."""
    bus = bus or _bus()
    node = Gio.DBusNodeInfo.new_for_xml(_AGENT_XML)

    def name_of(device):
        try:
            props = _call(device, "org.freedesktop.DBus.Properties", "GetAll",
                          GLib.Variant("(s)", (IFACE_DEVICE,)), bus=bus).unpack()[0]
            return props.get("Alias") or props.get("Address") or device
        except BluetoothError:
            return device

    def reject(inv, why="Rejected"):
        inv.return_dbus_error("org.bluez.Error.Rejected", why)

    def on_call(_conn, _sender, _path, _iface, method, params, inv):
        args = params.unpack()
        if method == "RequestConfirmation":
            ok = ask("confirm", name_of(args[0]), f"{args[1]:06d}")
            return inv.return_value(None) if ok else reject(inv)
        if method in ("DisplayPasskey", "DisplayPinCode"):
            code = f"{args[1]:06d}" if method == "DisplayPasskey" else args[1]
            ask("display", name_of(args[0]), code)
            return inv.return_value(None)
        if method in ("RequestPinCode", "RequestPasskey"):
            pin = ask("pin", name_of(args[0]), None)
            if not pin:
                return reject(inv, "Cancelled")
            if method == "RequestPinCode":
                return inv.return_value(GLib.Variant("(s)", (str(pin),)))
            if not str(pin).isdigit():
                return reject(inv, "Passkey must be digits")
            return inv.return_value(GLib.Variant("(u)", (int(pin),)))
        if method == "RequestAuthorization":
            ok = ask("confirm", name_of(args[0]), None)
            return inv.return_value(None) if ok else reject(inv)
        # AuthorizeService (a paired device opening a profile), Release, Cancel
        return inv.return_value(None)

    reg_id = bus.register_object(AGENT_PATH, node.interfaces[0], on_call, None, None)
    _call("/org/bluez", IFACE_AGENT_MANAGER, "RegisterAgent",
          GLib.Variant("(os)", (AGENT_PATH, "KeyboardDisplay")), bus=bus)
    try:
        _call("/org/bluez", IFACE_AGENT_MANAGER, "RequestDefaultAgent",
              GLib.Variant("(o)", (AGENT_PATH,)), bus=bus)
    except BluetoothError:
        pass  # another agent is default; ours still answers for pairings we start
    return reg_id
