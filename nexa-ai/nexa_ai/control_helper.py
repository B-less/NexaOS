import json
import sys

from . import tools

# Runs as root via `doas /usr/local/bin/nexa-ai-control-helper` (see
# /etc/doas.conf — that rule permits the "user" account to run this exact
# binary, nothing else). It reads one {"tool": ..., "args": ...} request from
# stdin, refuses anything that is not an allowlisted control tool (so this
# helper can never be used as a generic root shell even by something that
# reuses the doas grant), runs it, and writes one JSON response to stdout.


def main():
    try:
        request = json.loads(sys.stdin.read())
        name = request["tool"]
        args = request.get("args") or {}
    except (json.JSONDecodeError, KeyError, TypeError):
        print(json.dumps({"ok": False, "error": "malformed request"}))
        sys.exit(1)

    spec = tools.TOOLS.get(name)
    if not spec or spec["category"] != tools.CONTROL:
        print(json.dumps({"ok": False, "error": f"'{name}' is not a control tool"}))
        sys.exit(1)

    try:
        result = spec["func"](args)
        print(json.dumps({"ok": True, "result": result}))
    except tools.ToolError as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
