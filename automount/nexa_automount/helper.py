import json
import sys

from . import mounting

# Two entry points, one validator (mounting.py):
#   nexa-mount-helper auto|cleanup <kname>     -- run by udev as root on hot-plug
#   echo '{"action": ..., "name": ...}' | doas nexa-mount-helper
#                                             -- run by the desktop (list/mount/unmount/eject)

ACTIONS = {
    "list": lambda req: mounting.list_volumes(),
    "mount": lambda req: mounting.mount(req.get("name")),
    "unmount": lambda req: mounting.unmount(req.get("name")),
    "eject": lambda req: mounting.eject(req.get("name")),
}


def main(argv):
    if len(argv) == 3 and argv[1] in ("auto", "cleanup"):
        (mounting.auto if argv[1] == "auto" else mounting.cleanup)(argv[2])
        return 0
    try:
        req = json.loads(sys.stdin.read())
        handler = ACTIONS[req["action"]]
    except (json.JSONDecodeError, KeyError, TypeError):
        print(json.dumps({"ok": False, "error": "malformed or unknown request"}))
        return 1
    try:
        print(json.dumps({"ok": True, "result": handler(req)}))
        return 0
    except mounting.MountError as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        return 1
    except (OSError, ValueError) as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
