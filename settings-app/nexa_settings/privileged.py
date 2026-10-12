import json
import subprocess


class PrivilegedError(Exception):
    pass


def call(action, timeout=15, **fields):
    request = {"action": action, **fields}
    try:
        p = subprocess.run(
            ["doas", "/usr/local/bin/nexa-settings-helper"],
            input=json.dumps(request), capture_output=True, text=True, timeout=timeout,
        )
    except FileNotFoundError:
        raise PrivilegedError("doas is not installed")
    except subprocess.TimeoutExpired:
        raise PrivilegedError("timed out")
    try:
        resp = json.loads(p.stdout)
    except json.JSONDecodeError:
        raise PrivilegedError(p.stderr.strip() or f"helper produced no output (exit {p.returncode})")
    if not resp.get("ok"):
        raise PrivilegedError(resp.get("error", "unknown error"))
    return resp
