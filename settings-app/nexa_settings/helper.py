import json
import os
import re
import sys

from nexa_common import persist, sysconf, wgconf

# Runs as root via `doas nexa-settings-helper` (see /etc/doas.conf — that rule
# permits "user" to run this exact binary, nothing else). Reads one
# {"action": ..., ...} request from stdin, only ever touches the few files
# listed below, and validates every field before writing — same
# allowlist-and-nothing-else shape as nexa-ai's control_helper.py.

HOSTNAME_RE = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9-]{0,62}$")
AI_CONFIG_PATH = "/etc/nexa-ai/config.json"


def set_hostname(req):
    name = req.get("name", "")
    if not HOSTNAME_RE.match(name):
        return {"ok": False, "error": "invalid hostname"}
    with open("/etc/hostname", "w") as f:
        f.write(name + "\n")
    import subprocess
    subprocess.run(["hostname", name], check=False)
    return {"ok": True}


def write_ai_config(req):
    data = req.get("data", {})
    allowed = {"api_key", "model", "allow_control", "require_confirmation"}
    if not set(data.keys()) <= allowed:
        return {"ok": False, "error": "unexpected field in config update"}
    if "api_key" in data and not isinstance(data["api_key"], str):
        return {"ok": False, "error": "api_key must be a string"}
    if "model" in data and not isinstance(data["model"], str):
        return {"ok": False, "error": "model must be a string"}
    for flag in ("allow_control", "require_confirmation"):
        if flag in data and not isinstance(data[flag], bool):
            return {"ok": False, "error": f"{flag} must be a boolean"}

    try:
        with open(AI_CONFIG_PATH) as f:
            current = json.load(f)
    except (OSError, json.JSONDecodeError):
        current = {}
    current.update(data)

    with open(AI_CONFIG_PATH, "w") as f:
        json.dump(current, f, indent=2)
    import os
    os.chown(AI_CONFIG_PATH, 0, 1000)
    os.chmod(AI_CONFIG_PATH, 0o640)
    return {"ok": True}


def set_timezone(req):
    zone = req.get("zone", "")
    if not sysconf.valid_zone(zone):
        return {"ok": False, "error": "unknown time zone"}
    # Replace /etc/localtime atomically so a failure can never leave the
    # system with no timezone link at all.
    tmp = "/etc/localtime.nexa-new"
    try:
        if os.path.lexists(tmp):
            os.remove(tmp)
        os.symlink(os.path.join(sysconf.ZONEINFO, zone), tmp)
        os.replace(tmp, "/etc/localtime")
        with open("/etc/timezone", "w") as f:
            f.write(zone + "\n")
    except OSError as e:
        return {"ok": False, "error": str(e)}
    return {"ok": True}


# ---- VPN (WireGuard) -------------------------------------------------------
# Tunnel files live in /etc/wireguard (root only). Imports are checked by
# wgconf.validate, which refuses PostUp-style lines: wg-quick would run them
# as root. DNS servers are kept beside the file and applied here, with the
# previous /etc/resolv.conf restored when the tunnel goes down.

WG_DIR = "/etc/wireguard"
RESOLV = "/etc/resolv.conf"
RESOLV_BACKUP = "/run/nexa-vpn/resolv.conf.before-vpn"
_run = __import__("subprocess").run


def _vpn_name(req):
    name = req.get("name", "")
    if not isinstance(name, str) or not wgconf.NAME_RE.match(name):
        return None
    return name


def _active_tunnels():
    out = _run(["ip", "-o", "link", "show", "type", "wireguard"], capture_output=True, text=True).stdout
    return {line.split(":")[1].strip().split("@")[0] for line in out.splitlines() if ":" in line}


def _tunnels():
    try:
        return sorted(f[:-5] for f in os.listdir(WG_DIR) if f.endswith(".conf"))
    except OSError:
        return []


def vpn_list(req):
    active = _active_tunnels()
    return {"ok": True, "tunnels": [{"name": n, "active": n in active} for n in _tunnels()]}


def vpn_import(req):
    name = _vpn_name(req)
    if name is None:
        return {"ok": False, "error": "Names can use letters, digits and - _ . (up to 15)."}
    try:
        clean, dns = wgconf.validate(str(req.get("config", "")))
    except wgconf.WgConfError as e:
        return {"ok": False, "error": str(e)}
    os.makedirs(WG_DIR, mode=0o700, exist_ok=True)
    for path, body in ((f"{WG_DIR}/{name}.conf", clean), (f"{WG_DIR}/{name}.dns", json.dumps(dns))):
        fd = os.open(path + ".tmp", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(body)
        os.replace(path + ".tmp", path)
    return {"ok": True, "name": name}


# Copied, not renamed: /run is a different filesystem from /etc (and
# resolv.conf can be a bind mount), so rename() would fail.
def _restore_dns():
    if os.path.exists(RESOLV_BACKUP):
        with open(RESOLV_BACKUP) as src, open(RESOLV, "w") as dst:
            dst.write(src.read())
        os.remove(RESOLV_BACKUP)


def _wg_quick(direction, name):
    r = _run(["wg-quick", direction, name], capture_output=True, text=True, timeout=30)
    if r.returncode != 0:
        lines = [l for l in (r.stderr or r.stdout).splitlines() if l.strip() and not l.startswith("[#]")]
        return lines[-1] if lines else f"wg-quick {direction} failed"
    return None


def vpn_down(req):
    name = _vpn_name(req)
    if name is None:
        return {"ok": False, "error": "bad tunnel name"}
    if name in _active_tunnels():
        err = _wg_quick("down", name)
        if err:
            return {"ok": False, "error": err}
    _restore_dns()
    return {"ok": True}


def vpn_up(req):
    name = _vpn_name(req)
    if name is None or name not in _tunnels():
        return {"ok": False, "error": "No such VPN."}
    for other in _active_tunnels() - {name}:  # one VPN at a time
        if other in _tunnels():
            _wg_quick("down", other)
    if name not in _active_tunnels():
        err = _wg_quick("up", name)
        if err:
            return {"ok": False, "error": err}
    try:
        with open(f"{WG_DIR}/{name}.dns") as f:
            dns = [d for d in json.load(f) if isinstance(d, str)]
    except (OSError, ValueError):
        dns = []
    if dns:
        os.makedirs(os.path.dirname(RESOLV_BACKUP), exist_ok=True)
        if not os.path.exists(RESOLV_BACKUP) and os.path.exists(RESOLV):
            with open(RESOLV) as src, open(RESOLV_BACKUP, "w") as dst:
                dst.write(src.read())
        with open(RESOLV, "w") as f:
            f.write(f"# set by NexaOS VPN '{name}'\n" + "".join(f"nameserver {d}\n" for d in dns))
    return {"ok": True}


def vpn_remove(req):
    name = _vpn_name(req)
    if name is None:
        return {"ok": False, "error": "bad tunnel name"}
    result = vpn_down(req)
    if not result["ok"]:
        return result
    for ext in (".conf", ".dns"):
        try:
            os.remove(f"{WG_DIR}/{name}{ext}")
        except FileNotFoundError:
            pass
    return {"ok": True}


RFKILL_DIR = "/sys/class/rfkill"


def bluetooth_unblock(req):
    """Lift the soft (airplane-mode) block on Bluetooth radios. A hardware
    switch can't be overridden from software."""
    hard = False
    for dev in sorted(os.listdir(RFKILL_DIR)) if os.path.isdir(RFKILL_DIR) else []:
        base = os.path.join(RFKILL_DIR, dev)
        try:
            with open(os.path.join(base, "type")) as f:
                if f.read().strip() != "bluetooth":
                    continue
            with open(os.path.join(base, "soft"), "w") as f:
                f.write("0")
            with open(os.path.join(base, "hard")) as f:
                hard = hard or f.read().strip() == "1"
        except OSError:
            continue
    if hard:
        return {"ok": False, "error": "Bluetooth is turned off by a hardware switch or key."}
    return {"ok": True}


def persist_create(req):
    """Live USB only: add the NEXA_PERSIST partition (see persist.py)."""
    try:
        return {"ok": True, **persist.create()}
    except persist.PersistError as e:
        return {"ok": False, "error": str(e)}


ACTIONS = {
    "persist_create": persist_create,
    "set_hostname": set_hostname,
    "write_ai_config": write_ai_config,
    "set_timezone": set_timezone,
    "vpn_list": vpn_list,
    "vpn_import": vpn_import,
    "vpn_up": vpn_up,
    "vpn_down": vpn_down,
    "vpn_remove": vpn_remove,
    "bluetooth_unblock": bluetooth_unblock,
}


def main():
    try:
        req = json.loads(sys.stdin.read())
        action = req["action"]
    except (json.JSONDecodeError, KeyError, TypeError):
        print(json.dumps({"ok": False, "error": "malformed request"}))
        sys.exit(1)

    handler = ACTIONS.get(action)
    if not handler:
        print(json.dumps({"ok": False, "error": f"unknown action: {action}"}))
        sys.exit(1)

    result = handler(req)
    print(json.dumps(result))
    sys.exit(0 if result.get("ok") else 1)


if __name__ == "__main__":
    main()
