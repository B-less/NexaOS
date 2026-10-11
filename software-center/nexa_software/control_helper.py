import json
import sys

from nexa_common import apkops

# Runs as root via `doas /usr/local/bin/nexa-software-helper` (see
# /etc/doas.conf). Reads one {"action", "args"} request from stdin, refuses
# anything not in ACTIONS, runs it, and writes one JSON response to stdout --
# same shape as nexa-ai's control_helper.py. install/remove re-validate the
# package name themselves (apkops.is_valid_package_name) rather than trusting
# the caller, so this can't be turned into a generic root shell even by
# something that reuses the doas grant.

ACTIONS = {
    "update_index": lambda args: apkops.update_index(),
    "install": lambda args: apkops.install_package(args["package"]),
    "remove": lambda args: apkops.remove_package(args["package"]),
}


def main():
    try:
        request = json.loads(sys.stdin.read())
        action = request["action"]
        args = request.get("args") or {}
    except (json.JSONDecodeError, KeyError, TypeError):
        print(json.dumps({"ok": False, "error": "malformed request"}))
        sys.exit(1)

    handler = ACTIONS.get(action)
    if handler is None:
        print(json.dumps({"ok": False, "error": f"'{action}' is not a known action"}))
        sys.exit(1)

    try:
        output = handler(args)
        print(json.dumps({"ok": True, "output": output}))
    except (apkops.ApkError, KeyError) as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
