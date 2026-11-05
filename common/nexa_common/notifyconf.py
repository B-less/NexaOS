"""Notification settings for Settings > Notifications: how long Signals
stay, where they appear, and apps whose Signals don't pop up. Kept in
~/.config/nexa/notifications.json, read by nexa-signald (how long, muted
apps) and nexa-signals (where); save() tells them at once. Do not disturb
is Signals' Quiet (also in Deck: the Quiet card)."""
import json
import os
import re
import subprocess

POSITIONS = {"top-right": "Top right", "top-center": "Top middle", "top-left": "Top left",
             "bottom-right": "Bottom right", "bottom-center": "Bottom middle", "bottom-left": "Bottom left"}
DEFAULTS = {"timeout": 6, "position": "top-right", "muted_apps": []}
_APP_RE = re.compile(r"^[^\[\]\"\n\\]{1,80}$")


def _config_dir(home=None):
    if home:
        return os.path.join(home, ".config")
    return os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")


def prefs_path(home=None):
    return os.path.join(_config_dir(home), "nexa", "notifications.json")


def clean(p):
    out = {"timeout": DEFAULTS["timeout"], "position": DEFAULTS["position"], "muted_apps": []}
    p = p if isinstance(p, dict) else {}
    if isinstance(p.get("timeout"), int) and not isinstance(p["timeout"], bool) and 0 <= p["timeout"] <= 60:
        out["timeout"] = p["timeout"]
    if p.get("position") in POSITIONS:
        out["position"] = p["position"]
    if isinstance(p.get("muted_apps"), list):
        out["muted_apps"] = sorted({a for a in p["muted_apps"] if isinstance(a, str) and _APP_RE.match(a)})[:100]
    return out


def load(home=None):
    try:
        with open(prefs_path(home)) as f:
            return clean(json.load(f))
    except (OSError, ValueError):
        return clean({})


def save(prefs):
    p = clean(prefs)
    os.makedirs(os.path.dirname(prefs_path()), exist_ok=True)
    with open(prefs_path(), "w") as f:
        json.dump(p, f, indent=1)
    tell_signals()
    return p


def tell_signals():
    """Nexa Signals reads the file again (nexa-signalctl reload)."""
    try:
        subprocess.run(["nexa-signalctl", "reload"], capture_output=True, timeout=3)
    except (OSError, subprocess.SubprocessError):
        pass


def dnd_on():
    try:
        out = subprocess.run(["nexa-signalctl", "mode"], capture_output=True, text=True, timeout=3).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    return "do-not-disturb" in out.split()


def set_dnd(on):
    try:
        subprocess.run(["nexa-signalctl", "mode", "-a" if on else "-r", "do-not-disturb"], capture_output=True, timeout=3)
    except (OSError, subprocess.SubprocessError):
        pass


def recent_apps():
    """Apps that sent notifications lately (Signals' history), for the list."""
    try:
        out = subprocess.run(["nexa-signalctl", "history"], capture_output=True, text=True, timeout=3).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    apps = set()
    for line in out.splitlines():
        m = re.search(r"App name:\s*(.+)$", line)
        if m:
            apps.add(m.group(1).strip())
    try:
        data = json.loads(out)
        for group in data.get("data", []):
            for n in group:
                name = (n.get("app-name") or {}).get("data")
                if name:
                    apps.add(name)
    except (ValueError, AttributeError, TypeError):
        pass
    return sorted(a for a in apps if _APP_RE.match(a))
