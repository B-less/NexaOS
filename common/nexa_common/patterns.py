"""Client for nexa-patternd (Rust, tools/nexa-patternd): Nexa Patterns,
which learns on the device which apps you use when and after what, and
suggests what you'll open next. Also reads and writes your Patterns
settings (~/.config/nexa/patterns.json), which the daemon picks up.

Used by the desktop "For you" card, Nexus' Suggested row and Settings >
Privacy. See docs/NEXA_PATTERNS.md."""
import json
import os
import socket
import time

from nexa_common import desktopapps


class PatternsError(Exception):
    pass


def socket_path():
    if os.environ.get("NEXA_PATTERNS_SOCKET"):
        return os.environ["NEXA_PATTERNS_SOCKET"]
    run = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    return os.path.join(run, "nexa-patterns.sock")


def config_path():
    base = os.environ.get("XDG_CONFIG_HOME")
    base = base if base and os.path.isabs(base) else os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "nexa", "patterns.json")


DEFAULTS = {"enabled": False, "desktop_card": True, "card_position": "top-right",
            "excluded_apps": [], "never_suggest": [], "hidden_until": 0, "preload": True}
POSITIONS = {"top-left": "Top left", "top-right": "Top right", "bottom-right": "Bottom right", "center": "Centre"}


def request(op, timeout=2, **args):
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(timeout)
            s.connect(socket_path())
            s.sendall((json.dumps(dict(op=op, **args)) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = s.recv(65536)
                if not chunk:
                    break
                data += chunk
    except OSError as e:
        raise PatternsError(f"Nexa Patterns isn't running: {e}")
    try:
        reply = json.loads(data)
    except ValueError:
        raise PatternsError("Nexa Patterns gave a broken answer")
    if not reply.get("ok"):
        raise PatternsError(reply.get("error") or "Nexa Patterns refused")
    return reply["result"]


def load_config():
    cfg = dict(DEFAULTS)
    try:
        with open(config_path()) as f:
            data = json.load(f)
        if isinstance(data, dict):
            cfg.update({k: v for k, v in data.items() if k in DEFAULTS})
    except (OSError, ValueError):
        pass
    return cfg


def save_config(cfg):
    path = config_path()
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump({k: cfg.get(k, v) for k, v in DEFAULTS.items()}, f, indent=1)
    os.replace(tmp, path)
    try:
        request("reload", timeout=1)
    except PatternsError:
        pass  # it reads the file at start and checks it every few seconds


def update_config(**changes):
    cfg = load_config()
    cfg.update(changes)
    save_config(cfg)
    return cfg


def _toggle_list(key, app, on):
    cfg = load_config()
    items = [a for a in cfg[key] if a != app] + ([app] if on else [])
    return update_config(**{key: items})


def never_suggest(app, on=True):
    """'Don't suggest this'. Also tells the model it was dismissed."""
    if on:
        try:
            request("feedback", app=app, action="dismissed")
        except PatternsError:
            pass
    return _toggle_list("never_suggest", app, on)


def exclude(app, on=True):
    """'Don't learn from this app'."""
    return _toggle_list("excluded_apps", app, on)


def hide_for_today(now=None):
    # NEXA_PATTERNS_NOW: the daemon's test clock, for tests.
    lt = time.localtime(now or float(os.environ.get("NEXA_PATTERNS_NOW") or time.time()))
    midnight = time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday + 1, 0, 0, 0, 0, 0, -1))
    return update_config(hidden_until=int(midnight))


def resolve(app_id, apps=None):
    """The installed app behind a Wayland app id: name, icon, how to start it."""
    apps = desktopapps.load_apps() if apps is None else apps
    low = app_id.lower()
    for test in (lambda a: a["id"] == app_id, lambda a: a["id"].lower() == low,
                 lambda a: a["id"].lower().endswith("." + low), lambda a: a["name"].lower() == low):
        for a in apps:
            if test(a):
                return a
    return None


def why_text(suggestion, name, apps=None):
    """The reason for a suggestion, in words, with display names."""
    parts = suggestion.get("why_parts") or {}
    kind = parts.get("kind")
    if kind == "time":
        days = "weekend days" if parts.get("weekend") else "weekdays"
        return f"You opened {name} around {parts['around']} on {parts['days']} of the last {parts['of']} {days}."
    if kind == "after":
        before = resolve(parts["after"], apps)
        return f"You often open {name} after {before['name'] if before else parts['after']} ({parts['times']} times in the last two weeks)."
    if kind == "often":
        return f"You've used {name} a lot lately ({parts.get('times', 0)} times in the last two weeks)."
    return suggestion.get("why", "")


def suggestions(op="card", apps=None, **args):
    """The daemon's answer with each app resolved to an installed app (apps
    it can't find, e.g. uninstalled since, are left out)."""
    result = request(op, **args)
    apps = desktopapps.load_apps() if apps is None else apps
    shown = []
    for s in result.get("apps", []):
        app = resolve(s["app"], apps)
        if app:
            shown.append(dict(s, entry=app, text=why_text(s, app["name"], apps)))
    result["apps"] = shown
    return result


def launch(suggestion):
    try:
        request("feedback", app=suggestion["app"], action="used", timeout=1)
    except PatternsError:
        pass
    desktopapps.launch(suggestion["entry"]["exec"])
