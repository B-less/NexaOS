"""Notification settings for Settings > Notifications: how long pop-ups
stay, where they appear, and apps whose pop-ups are hidden. Kept in
~/.config/nexa/notifications.json; themes.mako_config() adds them to the
mako config it writes, and write_mako() rewrites it at once. Do not
disturb is mako's "do-not-disturb" mode (also in Deck: the Quiet card)."""
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


def mako_extra(p):
    """Lines for the top of mako's config, and sections for muted apps."""
    top = f"anchor={p['position']}\ndefault-timeout={p['timeout'] * 1000}\n"
    sections = "".join(f'\n[app-name="{a}"]\ninvisible=1\n' for a in p["muted_apps"])
    return top, sections


def save(prefs):
    p = clean(prefs)
    os.makedirs(os.path.dirname(prefs_path()), exist_ok=True)
    with open(prefs_path(), "w") as f:
        json.dump(p, f, indent=1)
    write_mako()
    return p


def write_mako():
    from . import themes
    s = themes.load_settings()
    t = themes.THEMES.get(s["theme"]) or next(iter(themes.THEMES.values()))
    themes._write(os.path.join(_config_dir(), "mako", "config"), themes.mako_config(t, s["opacity"]))
    themes._quiet(["makoctl", "reload"])


def dnd_on():
    try:
        out = subprocess.run(["makoctl", "mode"], capture_output=True, text=True, timeout=3).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    return "do-not-disturb" in out.split()


def set_dnd(on):
    subprocess.run(["makoctl", "mode", "-a" if on else "-r", "do-not-disturb"], capture_output=True, timeout=3)


def recent_apps():
    """Apps that sent notifications lately (mako's history), for the list."""
    try:
        out = subprocess.run(["makoctl", "history"], capture_output=True, text=True, timeout=3).stdout
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
