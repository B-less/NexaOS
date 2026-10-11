"""Client for nexa-mount-helper (removable drives). Runs unprivileged and asks
the narrow root helper through doas, the same pattern as the other helpers."""
import json
import subprocess


class RemovableError(Exception):
    pass


def _call(action, name=None):
    request = {"action": action}
    if name is not None:
        request["name"] = name
    try:
        p = subprocess.run(
            ["doas", "/usr/local/bin/nexa-mount-helper"],
            input=json.dumps(request), capture_output=True, text=True, timeout=60,
        )
    except FileNotFoundError:
        raise RemovableError("doas is not installed")
    except subprocess.TimeoutExpired:
        raise RemovableError("The drive took too long to respond.")
    try:
        response = json.loads(p.stdout)
    except json.JSONDecodeError:
        raise RemovableError(p.stderr.strip() or "The drive helper produced no output.")
    if not response.get("ok"):
        raise RemovableError(response.get("error", "That didn't work."))
    return response["result"]


def list_volumes():
    return _call("list")


def mount(name):
    return _call("mount", name)


def unmount(name):
    return _call("unmount", name)


def eject(name):
    return _call("eject", name)


def human_size(n):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024


def display_name(volume):
    return volume["label"] or f"Drive ({human_size(volume['size_bytes'])})"
