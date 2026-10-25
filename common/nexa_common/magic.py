"""Magic Layer and Living Visual Engine settings (TODOnexa_magic.md).

One file, ~/.config/nexa/magic.json, read by nexa-living (C, the renderer),
Nexa MicroBrain (the features) and the apps. Saved atomically; nexa-living
notices the change itself (it watches the folder), and notify() also tells
it straight away.

    living:   the backgrounds: on/off, desktop style, quality, speed,
              brightness, density, pointer reaction, veil, fps cap,
              battery behaviour, reduced motion, window_alpha, and a
              style per app ("off" for none)
    features: one entry per Magic Layer feature, {"enabled": bool, ...}
"""
import copy
import json
import os
import socket

VERSION = 1

# The renderer's styles (tools-c/nexa-living/styles.c), in order.
STYLES = [
    ("circuit", "Cyber Circuit"), ("quantum", "Quantum Flow"), ("neural", "Neural Network"),
    ("holo-grid", "Holographic Grid"), ("rain", "Digital Rain"), ("nebula", "Cosmic Nebula"),
    ("pulse", "Energy Pulse"), ("orbital", "Orbital Field"), ("geometry", "Living Geometry"),
    ("signature", "Nexa Signature"),
]
STYLE_NAMES = [s for s, _ in STYLES]

# First-party apps (their Wayland app_id) and the style each starts with;
# the same list as nexa-living's own defaults.
APPS = [
    ("nexa-settings", "Settings", "holo-grid"), ("nexa-files", "Files", "circuit"), ("foot", "Terminal", "rain"),
    ("nexa-pulse", "Pulse (system monitor)", "pulse"), ("nexa-store", "Store", "orbital"),
    ("nexa-software", "Software", "orbital"), ("nexa-ai-gui", "Assistant", "neural"), ("nexa-music", "Music", "pulse"),
    ("nexa-notes", "Notes", "nebula"), ("nexa-calc", "Calculator", "geometry"), ("nexa-network", "Network", "quantum"),
    ("nexa-power", "Power", "signature"),
]

QUALITIES = [("auto", "Automatic"), ("high", "High (up to 60 fps)"), ("balanced", "Balanced (up to 30 fps)"),
             ("low", "Low resource (about 12 fps)"), ("static", "Static picture"), ("off", "Off")]

FEATURES = ["peek", "gravity", "rewind", "hold", "thread", "orbit", "filetrail", "passport", "postbox", "why",
            "ready", "lifeboat", "tomorrow", "appetite", "quiet", "clipboard", "surprise", "gestures"]
# Learning and anything that watches what you do starts off (opt-in).
OPT_IN = {"thread", "tomorrow", "appetite", "clipboard", "surprise", "gravity", "gestures"}

DEFAULTS = {
    "version": VERSION,
    "living": {
        "enabled": True, "desktop": "signature", "quality": "auto", "speed": 1.0, "brightness": 1.0,
        "density": 1.0, "reaction": 0.5, "veil": 0.3, "fps_cap": 0, "battery": "lower",
        "reduced_motion": False, "window_alpha": 0.72,
        "apps": {app: style for app, _t, style in APPS},
    },
    "features": {f: {"enabled": f not in OPT_IN} for f in FEATURES},
}


def path(home=None):
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(home or os.path.expanduser("~"), ".config")
    return os.path.join(base, "nexa", "magic.json")


def _merge(base, extra):
    """extra over base, keeping only known keys' types; unknown keys kept."""
    out = copy.deepcopy(base)
    for k, v in (extra or {}).items():
        if isinstance(out.get(k), dict) and isinstance(v, dict):
            out[k] = _merge(out[k], v)
        elif k in out and out[k] is not None and not isinstance(v, type(out[k])) \
                and not (isinstance(out[k], float) and isinstance(v, int)):
            continue  # a value of the wrong kind: keep the default
        else:
            out[k] = v
    return out


def load(home=None):
    """The settings, defaults filled in. A missing or broken file is defaults."""
    try:
        with open(path(home)) as f:
            data = json.load(f)
        if not isinstance(data, dict):
            data = {}
    except (OSError, ValueError):
        data = {}
    return _merge(DEFAULTS, data)


def save(data, home=None):
    p = path(home)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    tmp = p + ".tmp"
    with open(tmp, "w") as f:
        json.dump(data, f, indent=2, sort_keys=True)
    os.replace(tmp, p)
    notify()


def set_living(home=None, **changes):
    """Change some of the backgrounds' settings, checked."""
    data = load(home)
    lv = data["living"]
    for k, v in changes.items():
        if k not in DEFAULTS["living"] or k == "apps":
            raise ValueError(f"unknown setting {k}")
        if k == "desktop" and v not in STYLE_NAMES + ["none"]:
            raise ValueError(f"unknown style {v}")
        if k == "quality" and v not in [q for q, _ in QUALITIES]:
            raise ValueError(f"unknown quality {v}")
        lv[k] = v
    save(data, home)
    return lv


def set_app_style(apps, style, home=None):
    """One style (or "off") for one or several apps."""
    if style not in STYLE_NAMES + ["off"]:
        raise ValueError(f"unknown style {style}")
    data = load(home)
    for app in ([apps] if isinstance(apps, str) else apps):
        data["living"]["apps"][app] = style
    save(data, home)


def app_style(app, home=None):
    """The style this app's window gets, or None (off, not first-party, or
    living backgrounds turned off)."""
    lv = load(home)["living"]
    if not lv["enabled"] or lv["quality"] == "off":
        return None
    s = lv["apps"].get(app)
    return s if s in STYLE_NAMES else None


def window_alpha(app, home=None):
    """How solid this app's window should be so its background shows
    through (None: leave the theme's glass as it is)."""
    if not app_style(app, home):
        return None
    return max(0.55, min(0.95, float(load(home)["living"].get("window_alpha", 0.72))))


def feature(name, home=None):
    return load(home)["features"].get(name, {"enabled": False})


def set_feature(name, home=None, **changes):
    if name not in FEATURES:
        raise ValueError(f"unknown feature {name}")
    data = load(home)
    data["features"].setdefault(name, {}).update(changes)
    save(data, home)
    return data["features"][name]


def reset(section=None, home=None):
    """Defaults again: everything, "living", or one feature."""
    data = load(home)
    if section is None:
        data = copy.deepcopy(DEFAULTS)
    elif section == "living":
        data["living"] = copy.deepcopy(DEFAULTS["living"])
    elif section in FEATURES:
        data["features"][section] = copy.deepcopy(DEFAULTS["features"][section])
    save(data, home)


# ---- talking to the renderer -------------------------------------------------

def _sock():
    run = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    return os.path.join(run, "nexa-living.sock")


def send(message):
    """One line to nexa-living; nothing happens if it isn't running."""
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as s:
            s.sendto(message.encode()[:200], _sock())
        return True
    except OSError:
        return False


def notify():
    return send("reload")


def click(app, x, y):
    return send(f"click {app} {x:.0f} {y:.0f}")


def level(app, value):
    return send(f"level {app} {max(0.0, min(1.0, value)):.3f}")


def stats():
    """What nexa-living measured over its last 5 s, or None."""
    run = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    try:
        with open(os.path.join(run, "nexa-living.stats")) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None
