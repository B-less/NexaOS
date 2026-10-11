import json
import os
import re
import sys

from nexa_common import sysconf

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


ACTIONS = {
    "set_hostname": set_hostname,
    "write_ai_config": write_ai_config,
    "set_timezone": set_timezone,
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
