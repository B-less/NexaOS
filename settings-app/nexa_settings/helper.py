import json
import os
import re
import sys

from nexa_common import persist

# Runs as root via `doas nexa-settings-helper`. Since the system broker
# (nexa-brokerd, Rust) took over hostname, time zone, assistant settings,
# VPN and Bluetooth, only one action is left here: creating the live USB's
# persistence partition (disk partitioning, which moves last, with the
# OS installer).

def persist_create(req):
    """Live USB only: add the NEXA_PERSIST partition (see persist.py)."""
    try:
        return {"ok": True, **persist.create()}
    except persist.PersistError as e:
        return {"ok": False, "error": str(e)}


ACTIONS = {
    "persist_create": persist_create,
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
