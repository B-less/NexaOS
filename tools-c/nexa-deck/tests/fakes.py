"""Stand-ins for Deck's test (run by scripts/test-deck.sh):
  broker  /tmp/broker.sock: vpn_list / vpn_up / vpn_down / set_brightness, logged
  player  an MPRIS player on the session bus, calls logged
Both log one line per request to /tmp/fakes.log."""
import json
import os
import socket
import sys
import threading

LOG = "/tmp/fakes.log"


def log(line):
    with open(LOG, "a") as f:
        f.write(line + "\n")


def broker():
    path = "/tmp/broker.sock"
    if os.path.exists(path):
        os.unlink(path)
    s = socket.socket(socket.AF_UNIX)
    s.bind(path)
    s.listen(8)
    tunnels = {"Sweden": False, "Home": False}
    while True:
        c, _ = s.accept()
        data = b""
        while not data.endswith(b"\n"):
            chunk = c.recv(4096)
            if not chunk:
                break
            data += chunk
        req = json.loads(data)
        a, args = req["action"], req.get("args", {})
        log("broker " + json.dumps(req, sort_keys=True))
        if a == "vpn_list":
            out = {"ok": True, "result": {"tunnels": [{"name": n, "active": v} for n, v in tunnels.items()]}}
        elif a in ("vpn_up", "vpn_down") and args.get("name") in tunnels:
            tunnels[args["name"]] = a == "vpn_up"
            out = {"ok": True, "result": {"done": "ok"}}
        elif a == "set_brightness":
            with open("/tmp/bl/intel_backlight/brightness", "w") as f:
                f.write(str(int(args["percent"] * 10)))  # max 1000
            out = {"ok": True, "result": {}}
        else:
            out = {"ok": False, "error": "No such VPN."}
        c.sendall((json.dumps(out) + "\n").encode())
        c.close()


def player():
    import gi
    gi.require_version("Gio", "2.0")
    from gi.repository import Gio, GLib
    V = GLib.Variant
    st = {"status": "Paused"}
    xml = """<node><interface name="org.mpris.MediaPlayer2.Player">
      <method name="PlayPause"/><method name="Next"/><method name="Previous"/>
      <property name="PlaybackStatus" type="s" access="read"/><property name="Metadata" type="a{sv}" access="read"/>
    </interface></node>"""
    info = Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0]

    def props():
        return {"PlaybackStatus": V("s", st["status"]),
                "Metadata": V("a{sv}", {"xesam:title": V("s", "Café Nights"), "xesam:artist": V("as", ["Nexa Ensemble"])})}

    def method(conn, sender, path, iface, name, params, inv):
        log("player " + name)
        if name == "PlayPause":
            st["status"] = "Playing" if st["status"] != "Playing" else "Paused"
        inv.return_value(None)

    def get_prop(conn, sender, path, iface, name):
        return props()[name]

    def on_bus(conn, _name):
        conn.register_object("/org/mpris/MediaPlayer2", info, method, get_prop, None)

    Gio.bus_own_name(Gio.BusType.SESSION, "org.mpris.MediaPlayer2.nexatest", Gio.BusNameOwnerFlags.NONE, on_bus, None, None)
    GLib.MainLoop().run()


if __name__ == "__main__":
    threading.Thread(target=broker, daemon=True).start() if sys.argv[1:] == ["all"] else None
    player()
