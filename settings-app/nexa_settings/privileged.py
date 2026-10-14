"""System changes from Settings. Everything goes through nexa-brokerd (the
Rust system broker: checked, password where needed, sandboxed, audited),
except creating the live USB's persistence partition, which is disk
partitioning and stays with the live-only setup helper for now."""
import json
import subprocess

from nexa_common import askpass, broker


class PrivilegedError(Exception):
    pass


LEGACY = {"persist_create"}


def _legacy(action, timeout, fields):
    request = {"action": action, **fields}
    try:
        p = subprocess.run(["doas", "/usr/local/bin/nexa-settings-helper"],
                           input=json.dumps(request), capture_output=True, text=True, timeout=timeout)
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


def call(action, timeout=15, **fields):
    """Returns {"ok": True, ...result fields} or raises PrivilegedError."""
    if action in LEGACY:
        return _legacy(action, timeout, fields)

    def ask_password(what, error):
        return askpass.ask(f"{what or 'This change'} needs your password.")(error is not None)

    try:
        result = broker.run(action, fields, ask_password)
    except broker.BrokerError as e:
        raise PrivilegedError(str(e))
    return {"ok": True, **(result if isinstance(result, dict) else {"result": result})}
