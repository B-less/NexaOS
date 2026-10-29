"""Mouse and touchpad settings for Settings > Mouse & Touchpad.

Saved in ~/.config/nexa/input.json and written as compositor config:
Sway reads ~/.config/sway/config.d/30-nexa-input.conf (after
/etc/sway/config's defaults), Hyprland ~/.config/hypr/nexa-input.conf
(sourced by /etc/hypr/hyprland.conf). Changes also apply at once."""
import json
import os

from . import compositor

DEFAULTS = {
    "mouse_speed": 0.0,          # -1 slow .. 1 fast
    "mouse_natural_scroll": False,
    "left_handed": False,
    "touchpad_speed": 0.0,
    "touchpad_natural_scroll": True,
    "tap_to_click": True,
    "disable_while_typing": True,
}


def _config_dir():
    return os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")


def prefs_path():
    return os.path.join(_config_dir(), "nexa", "input.json")


def sway_path():
    return os.path.join(_config_dir(), "sway", "config.d", "30-nexa-input.conf")


def hypr_path():
    return os.path.join(_config_dir(), "hypr", "nexa-input.conf")


def clean(prefs):
    out = dict(DEFAULTS)
    for k, v in (prefs or {}).items():
        if k in ("mouse_speed", "touchpad_speed") and isinstance(v, (int, float)) and not isinstance(v, bool):
            out[k] = round(max(-1.0, min(1.0, float(v))), 2)
        elif k in DEFAULTS and isinstance(DEFAULTS[k], bool) and isinstance(v, bool):
            out[k] = v
    return out


def load():
    try:
        with open(prefs_path()) as f:
            return clean(json.load(f))
    except (OSError, ValueError, AttributeError):
        return dict(DEFAULTS)


def _on(b):
    return "enabled" if b else "disabled"


def sway_lines(p):
    """`input` commands, one per line, for the config file and swaymsg."""
    return [
        f"input type:pointer pointer_accel {p['mouse_speed']}",
        f"input type:pointer natural_scroll {_on(p['mouse_natural_scroll'])}",
        f"input type:pointer left_handed {_on(p['left_handed'])}",
        f"input type:touchpad pointer_accel {p['touchpad_speed']}",
        f"input type:touchpad natural_scroll {_on(p['touchpad_natural_scroll'])}",
        f"input type:touchpad tap {_on(p['tap_to_click'])}",
        f"input type:touchpad dwt {_on(p['disable_while_typing'])}",
        f"input type:touchpad left_handed {_on(p['left_handed'])}",
    ]


def _tf(b):
    return "true" if b else "false"


def hypr_keywords(p):
    """(keyword, value) for hyprctl and the config file. Hyprland has one
    speed for every pointer; the mouse's is used."""
    return [
        ("input:sensitivity", str(p["mouse_speed"])),
        ("input:natural_scroll", _tf(p["mouse_natural_scroll"])),
        ("input:left_handed", _tf(p["left_handed"])),
        ("input:touchpad:natural_scroll", _tf(p["touchpad_natural_scroll"])),
        ("input:touchpad:tap-to-click", _tf(p["tap_to_click"])),
        ("input:touchpad:disable_while_typing", _tf(p["disable_while_typing"])),
    ]


def hypr_conf(p):
    """A block Hyprland merges over the input settings in hyprland.conf."""
    return f"""# Written by Settings > Mouse & Touchpad; changes there replace this file.
input {{
    sensitivity = {p['mouse_speed']}
    natural_scroll = {_tf(p['mouse_natural_scroll'])}
    left_handed = {_tf(p['left_handed'])}
    touchpad {{
        natural_scroll = {_tf(p['touchpad_natural_scroll'])}
        tap-to-click = {_tf(p['tap_to_click'])}
        disable_while_typing = {_tf(p['disable_while_typing'])}
    }}
}}
"""


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        f.write(text)
    os.replace(tmp, path)


def save(prefs, apply_now=True):
    p = clean(prefs)
    _write(prefs_path(), json.dumps(p, indent=1) + "\n")
    _write(sway_path(), "# Written by Settings > Mouse & Touchpad; changes there replace this file.\n"
           + "\n".join(sway_lines(p)) + "\n")
    _write(hypr_path(), hypr_conf(p))
    if apply_now:
        apply(p)
    return p


def apply(p):
    """Tell the running compositor (nothing happens outside a session)."""
    which = compositor.which()
    if which == "sway":
        for line in sway_lines(p):
            compositor._sway(line)
    elif which == "hyprland":
        for k, v in hypr_keywords(p):
            compositor._call(["hyprctl", "keyword", k, v])


def ensure_files():
    """Hyprland refuses a config that sources a missing file: make sure
    nexa-input.conf exists before the session starts."""
    if not os.path.exists(hypr_path()):
        _write(hypr_path(), hypr_conf(load()))
