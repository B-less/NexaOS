"""WiFi control through iwd's D-Bus API (net.connman.iwd).

Shared by the Network settings app and nexa-ai's WiFi tools. iwd's D-Bus
policy already lets the "wheel" group (which the desktop user is in) talk to
it, so nothing here needs root or a doas helper. Every function blocks, so
GUI callers should run them off the main thread.
"""
import itertools
import os
import re
import time

import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

BUS_NAME = "net.connman.iwd"
IFACE_DEVICE = "net.connman.iwd.Device"
IFACE_STATION = "net.connman.iwd.Station"
IFACE_NETWORK = "net.connman.iwd.Network"
IFACE_KNOWN = "net.connman.iwd.KnownNetwork"
IFACE_AGENT_MANAGER = "net.connman.iwd.AgentManager"
IFACE_AGENT = "net.connman.iwd.Agent"
IWD_ROOT_PATH = "/net/connman/iwd"

_AGENT_XML = """
<node>
  <interface name="net.connman.iwd.Agent">
    <method name="Release"/>
    <method name="RequestPassphrase">
      <arg type="o" name="network" direction="in"/>
      <arg type="s" name="passphrase" direction="out"/>
    </method>
    <method name="Cancel">
      <arg type="s" name="reason" direction="in"/>
    </method>
  </interface>
</node>
"""

_FRIENDLY_ERRORS = {
    "org.freedesktop.DBus.Error.ServiceUnknown": "The WiFi service (iwd) isn't running.",
    "org.freedesktop.DBus.Error.NameHasNoOwner": "The WiFi service (iwd) isn't running.",
    "org.freedesktop.DBus.Error.AccessDenied": "Not allowed to control WiFi from this account.",
    "net.connman.iwd.Failed": "Couldn't connect - the password may be wrong, or the network rejected the connection.",
    "net.connman.iwd.Busy": "The WiFi adapter is busy, try again in a moment.",
    "net.connman.iwd.Aborted": "The connection attempt was cancelled.",
    "net.connman.iwd.NoAgent": "A password is needed to connect to this network.",
    "net.connman.iwd.NotSupported": "This kind of network isn't supported.",
    "net.connman.iwd.Timeout": "The network took too long to respond.",
    "net.connman.iwd.InProgress": "Another connection attempt is already in progress.",
    "net.connman.iwd.NotConfigured": "This network isn't configured.",
    "net.connman.iwd.NotConnected": "Not connected to any network.",
    "net.connman.iwd.NotFound": "That network wasn't found.",
}
_REMOTE_ERROR_RE = re.compile(r"GDBus\.Error:([^:\s]+):\s*(.*)", re.S)
_agent_counter = itertools.count(1)


class WifiError(Exception):
    pass


class PassphraseRequired(WifiError):
    pass


_bus_factory = lambda: Gio.bus_get_sync(Gio.BusType.SYSTEM, None)


def _friendly(err):
    m = _REMOTE_ERROR_RE.search(err.message or "")
    if m:
        name, detail = m.group(1), m.group(2).strip()
        return _FRIENDLY_ERRORS.get(name) or detail or name
    return err.message or "Unknown WiFi error"


def _bus():
    try:
        return _bus_factory()
    except GLib.Error as e:
        raise WifiError(_friendly(e))


def _call(path, iface, method, params=None, reply_type=None, timeout_ms=10000, bus=None):
    bus = bus or _bus()
    try:
        result = bus.call_sync(
            BUS_NAME, path, iface, method, params,
            GLib.VariantType(reply_type) if reply_type else None,
            Gio.DBusCallFlags.NONE, timeout_ms, None,
        )
    except GLib.Error as e:
        raise WifiError(_friendly(e))
    return result.unpack() if result is not None else None


def _objects(bus=None):
    return _call(
        "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects",
        None, "(a{oa{sa{sv}}})", bus=bus,
    )[0]


def _signal_percent(raw):
    dbm = raw / 100.0
    return int(max(0, min(100, 2 * (dbm + 100))))


def _station_devices(objects):
    devices = []
    for path, ifaces in objects.items():
        dev = ifaces.get(IFACE_DEVICE)
        if dev is None:
            continue
        station = ifaces.get(IFACE_STATION)
        connected_ssid = None
        if station and station.get("ConnectedNetwork"):
            net = objects.get(station["ConnectedNetwork"], {}).get(IFACE_NETWORK, {})
            connected_ssid = net.get("Name")
        devices.append({
            "path": path,
            "name": dev.get("Name"),
            "address": dev.get("Address"),
            "powered": bool(dev.get("Powered")),
            "station": station is not None,
            "state": station.get("State") if station else "not-in-station-mode",
            "scanning": bool(station.get("Scanning")) if station else False,
            "connected_ssid": connected_ssid,
        })
    # Real adapters first: mac80211_hwsim (built into the kernel for VM
    # testing) creates fake radios with 02:00:00:00:xx:00 addresses, and a
    # name sort put its wlan0 ahead of the real card.
    return sorted(devices, key=lambda d: (_is_simulated(d["address"]), d["name"] or ""))


def _is_simulated(address):
    return (address or "").lower().startswith("02:00:00:00:")


def _pick_device(objects, name=None):
    for d in _station_devices(objects):
        if d["station"] and (name is None or d["name"] == name):
            return d
    raise WifiError("No WiFi adapter found." if name is None else f"No WiFi adapter named {name}.")


def status():
    """Adapters and which network (if any) each is connected to."""
    devices = _station_devices(_objects())
    connected = next((d for d in devices if d["connected_ssid"]), None)
    return {
        "adapters": [
            {k: d[k] for k in ("name", "address", "powered", "state", "scanning", "connected_ssid")}
            for d in devices
        ],
        "connected_to": connected["connected_ssid"] if connected else None,
    }


def scan(device=None, wait=True, timeout=10):
    dev = _pick_device(_objects(), device)
    try:
        _call(dev["path"], IFACE_STATION, "Scan")
    except WifiError as e:
        if "busy" not in str(e).lower():
            raise
    if not wait:
        return
    deadline = time.monotonic() + timeout
    time.sleep(0.3)
    while time.monotonic() < deadline:
        current = _pick_device(_objects(), dev["name"])
        if not current["scanning"]:
            return
        time.sleep(0.5)


def list_networks(device=None, do_scan=False):
    if do_scan:
        scan(device)
    objects = _objects()
    dev = _pick_device(objects, device)
    ordered = _call(
        dev["path"], IFACE_STATION, "GetOrderedNetworks", None, "(a(on))",
    )[0]
    objects = _objects()
    networks = []
    for path, raw_signal in ordered:
        props = objects.get(path, {}).get(IFACE_NETWORK)
        if props is None:
            continue
        networks.append({
            "ssid": props.get("Name"),
            "security": props.get("Type"),
            "signal": _signal_percent(raw_signal),
            "connected": bool(props.get("Connected")),
            "known": "KnownNetwork" in props,
            "path": path,
        })
    return networks


def list_known():
    known = []
    for path, ifaces in _objects().items():
        props = ifaces.get(IFACE_KNOWN)
        if props is not None:
            known.append({
                "ssid": props.get("Name"),
                "security": props.get("Type"),
                "auto_connect": bool(props.get("AutoConnect", True)),
                "path": path,
            })
    return sorted(known, key=lambda k: k["ssid"] or "")


def _valid_passphrase(passphrase):
    if re.fullmatch(r"[0-9a-fA-F]{64}", passphrase):
        return True
    return 8 <= len(passphrase) <= 63 and all(32 <= ord(c) < 127 for c in passphrase)


def _connect_blocking(network_path, passphrase, timeout):
    bus = _bus()
    # The agent iwd calls back into is served from a private main context so
    # this works from any thread (a GUI worker thread has no running loop),
    # and so the blocking wait below can't deadlock against the agent
    # callback that needs to run while Connect() is still pending.
    ctx = GLib.MainContext.new()
    ctx.push_thread_default()
    agent_path = f"/org/nexaos/wifi/agent{os.getpid()}_{next(_agent_counter)}"
    agent_id = None
    registered = False
    outcome = {"error": None, "timed_out": False}
    try:
        loop = GLib.MainLoop.new(ctx, False)
        if passphrase is not None:
            def on_agent_call(_conn, _sender, _path, _iface, method, params, invocation):
                if method == "RequestPassphrase":
                    invocation.return_value(GLib.Variant("(s)", (passphrase,)))
                elif method in ("Cancel", "Release"):
                    invocation.return_value(None)
                else:
                    invocation.return_dbus_error(
                        "net.connman.iwd.Agent.Error.Canceled", "unsupported request",
                    )
            info = Gio.DBusNodeInfo.new_for_xml(_AGENT_XML).interfaces[0]
            agent_id = bus.register_object(agent_path, info, on_agent_call, None, None)
            _call(
                IWD_ROOT_PATH, IFACE_AGENT_MANAGER, "RegisterAgent",
                GLib.Variant("(o)", (agent_path,)), bus=bus,
            )
            registered = True

        def on_connect_done(source, result):
            try:
                source.call_finish(result)
            except GLib.Error as e:
                outcome["error"] = e
            loop.quit()

        bus.call(
            BUS_NAME, network_path, IFACE_NETWORK, "Connect", None, None,
            Gio.DBusCallFlags.NONE, (timeout + 5) * 1000, None, on_connect_done,
        )

        def on_timeout(*_a):
            outcome["timed_out"] = True
            loop.quit()
            return False

        timer = GLib.timeout_source_new_seconds(timeout + 10)
        timer.set_callback(on_timeout)
        timer.attach(ctx)
        loop.run()
        timer.destroy()
    finally:
        if registered:
            try:
                _call(
                    IWD_ROOT_PATH, IFACE_AGENT_MANAGER, "UnregisterAgent",
                    GLib.Variant("(o)", (agent_path,)), bus=bus,
                )
            except WifiError:
                pass
        if agent_id is not None:
            bus.unregister_object(agent_id)
        ctx.pop_thread_default()
    if outcome["timed_out"]:
        raise WifiError("The network took too long to respond.")
    if outcome["error"] is not None:
        raise WifiError(_friendly(outcome["error"]))


def _wait_until_connected(device, ssid, timeout=25):
    # Network.Connect() returns once the link is associated; with iwd's
    # built-in DHCP the address arrives a second or two later, so report
    # success only when the station has actually reached "connected".
    deadline = time.monotonic() + timeout
    while True:
        dev = _pick_device(_objects(), device)
        if dev["state"] == "connected" and dev["connected_ssid"] == ssid:
            return
        if dev["state"] == "disconnected":
            raise WifiError("The connection dropped before it finished setting up.")
        if time.monotonic() >= deadline:
            raise WifiError("Connected to the network, but it didn't hand out an IP address in time.")
        time.sleep(0.5)


def connect(ssid, passphrase=None, device=None, timeout=60):
    """Connect to `ssid`. Saved and open networks need no passphrase; a new
    secured network raises PassphraseRequired until one is supplied."""
    if not ssid:
        raise WifiError("A network name is required.")
    networks = [n for n in list_networks(device) if n["ssid"] == ssid]
    if not networks:
        raise WifiError(f"Network '{ssid}' wasn't found. Try scanning again.")
    net = networks[0]
    if net["security"] == "8021x":
        raise WifiError("Enterprise (802.1X) networks aren't supported yet.")
    if net["security"] == "wep":
        raise WifiError("WEP networks aren't supported.")
    if net["security"] == "psk" and not net["known"]:
        if not passphrase:
            raise PassphraseRequired(f"'{ssid}' needs a password the first time you connect.")
        if not _valid_passphrase(passphrase):
            raise WifiError("A WiFi password is 8-63 characters (or a 64-digit hex key).")
    adapter = _pick_device(_objects(), device)["name"]
    _connect_blocking(net["path"], passphrase if net["security"] == "psk" else None, timeout)
    _wait_until_connected(adapter, ssid)
    return {"connected_to": ssid}


def disconnect(device=None):
    dev = _pick_device(_objects(), device)
    _call(dev["path"], IFACE_STATION, "Disconnect")
    return {"disconnected": dev["name"]}


def forget(ssid):
    for k in list_known():
        if k["ssid"] == ssid:
            _call(k["path"], IFACE_KNOWN, "Forget")
            return {"forgot": ssid}
    raise WifiError(f"'{ssid}' isn't a saved network.")
