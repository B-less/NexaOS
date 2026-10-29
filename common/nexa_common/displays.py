"""Screens and night light, for Settings > Display.

The screens are changed by nexa-display (C, tools-c/nexa-display) through
the compositor; this module runs it, remembers what was chosen in
~/.config/nexa/displays.json (applied again at every login by
`nexa-display apply-saved`), and keeps the night light's settings for
power/nexa-nightlight (wlsunset)."""
import json
import os
import shutil
import subprocess

REPO_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
                          "build", "native", "nexa-display")
TRANSFORMS = ("normal", "90", "180", "270")
ROTATION_NAMES = {"normal": "Normal", "90": "90° right", "180": "Upside down", "270": "90° left"}
SCALES = (1.0, 1.25, 1.5, 1.75, 2.0)
NIGHT_DEFAULT = {"enabled": False, "temp": 4000, "from": "20:00", "to": "07:00"}


class DisplayError(Exception):
    pass


def _config_dir():
    return os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")


def saved_path():
    return os.path.join(_config_dir(), "nexa", "displays.json")


def night_path():
    return os.path.join(_config_dir(), "nexa", "nightlight.conf")


def binary():
    for candidate in (os.environ.get("NEXA_DISPLAY"), "/usr/local/bin/nexa-display", REPO_BUILD):
        if candidate and os.access(candidate, os.X_OK):
            return candidate
    return shutil.which("nexa-display") or "nexa-display"


def _run(args, stdin=None):
    try:
        out = subprocess.run([binary(), *args], input=stdin, capture_output=True, text=True, timeout=15).stdout
    except (OSError, subprocess.TimeoutExpired) as e:
        raise DisplayError(f"nexa-display didn't run: {e}")
    try:
        reply = json.loads(out.strip().splitlines()[-1])
    except (ValueError, IndexError):
        raise DisplayError("nexa-display gave no answer")
    if isinstance(reply, dict) and reply.get("ok") is False:
        raise DisplayError(reply.get("error") or "that didn't work")
    return reply


def screens():
    """Every screen: name, id, make, model, enabled, mode, modes, scale,
    transform, x, y."""
    return _run(["list"])


def apply(heads, test=False):
    """heads: [{"name", and any of "enabled", "width", "height", "refresh",
    "scale", "transform", "x", "y"}]. Raises DisplayError if refused."""
    return _run(["apply", "--test"] if test else ["apply"], json.dumps({"heads": heads}))


def resolutions(screen):
    """[(width, height, [refresh mHz, best first])], largest first."""
    by_size = {}
    for m in screen.get("modes", []):
        by_size.setdefault((m["width"], m["height"]), set()).add(m["refresh"])
    return sorted(((w, h, sorted(r, reverse=True)) for (w, h), r in by_size.items()), key=lambda t: (-t[0] * t[1], -t[0]))


def describe(screen):
    name = " ".join(x for x in (screen.get("make"), screen.get("model")) if x).strip()
    return name or screen.get("description") or screen["name"]


def snapshot(screen):
    """What apply() needs to put a screen back exactly as it is now."""
    head = {"name": screen["name"], "enabled": screen["enabled"], "scale": screen["scale"],
            "transform": screen["transform"], "x": screen["x"], "y": screen["y"]}
    if screen.get("mode"):
        head.update(screen["mode"])
    return head


def load_saved():
    try:
        with open(saved_path()) as f:
            data = json.load(f)
        return {h["id"]: h for h in data.get("heads", []) if isinstance(h, dict) and isinstance(h.get("id"), str)}
    except (OSError, ValueError, AttributeError):
        return {}


def save(screen_list):
    """Remember these screens as they are now (by id), keeping others."""
    saved = load_saved()
    for s in screen_list:
        entry = snapshot(s)
        entry.pop("name", None)
        entry["id"] = s["id"]
        saved[s["id"]] = entry
    os.makedirs(os.path.dirname(saved_path()), exist_ok=True)
    tmp = saved_path() + ".tmp"
    with open(tmp, "w") as f:
        json.dump({"version": 1, "heads": list(saved.values())}, f, indent=1)
    os.replace(tmp, saved_path())


def load_night():
    conf = dict(NIGHT_DEFAULT)
    try:
        with open(night_path()) as f:
            for line in f:
                k, _, v = line.strip().partition("=")
                if k == "enabled":
                    conf["enabled"] = v == "1"
                elif k == "temp" and v.isdigit() and 2500 <= int(v) <= 6000:
                    conf["temp"] = int(v)
                elif k in ("from", "to") and valid_time(v):
                    conf[k] = v
    except OSError:
        pass
    return conf


def valid_time(t):
    parts = t.split(":")
    return (len(parts) == 2 and all(p.isdigit() and len(p) == 2 for p in parts)
            and int(parts[0]) < 24 and int(parts[1]) < 60)


def save_night(enabled, temp, start, end, restart=True):
    if not (valid_time(start) and valid_time(end)):
        raise ValueError("times look like 20:00")
    temp = max(2500, min(6000, int(temp)))
    os.makedirs(os.path.dirname(night_path()), exist_ok=True)
    with open(night_path(), "w") as f:
        f.write(f"enabled={1 if enabled else 0}\ntemp={temp}\nfrom={start}\nto={end}\n")
    if restart:
        try:
            subprocess.Popen(["nexa-nightlight"], start_new_session=True,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except OSError:
            pass
