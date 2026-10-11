import json
import os
import subprocess
import sys

# Runs as root via `doas /usr/local/bin/nexa-power-helper` (see /etc/doas.conf).
# Reads one {"action": ...} request from stdin and only ever restarts or powers
# off the machine -- same allowlist shape as the other helpers.


def _reboot():
    os.sync()
    subprocess.Popen(["reboot"])
    return "restarting"


def _poweroff():
    os.sync()
    subprocess.Popen(["poweroff"])
    return "powering off"


ACTIONS = {"reboot": _reboot, "poweroff": _poweroff}


def main():
    try:
        action = json.loads(sys.stdin.read())["action"]
    except (json.JSONDecodeError, KeyError, TypeError):
        print(json.dumps({"ok": False, "error": "malformed request"}))
        sys.exit(1)
    handler = ACTIONS.get(action)
    if handler is None:
        print(json.dumps({"ok": False, "error": f"'{action}' is not a known action"}))
        sys.exit(1)
    print(json.dumps({"ok": True, "output": handler()}))


if __name__ == "__main__":
    main()
