"""Timezone, keyboard-layout and idle/lock settings shared by the Settings app
and its privileged helper. Pure file handling, no GTK, so it is testable on
any host (every path can be overridden)."""
import os
import re

ZONEINFO = "/usr/share/zoneinfo"
XKB_LIST = "/usr/share/X11/xkb/rules/base.lst"
KEYBOARD_CONF = os.path.expanduser("~/.config/sway/config.d/10-nexa-keyboard.conf")
IDLE_CONF = os.path.expanduser("~/.config/nexa/idle.conf")

DEFAULT_IDLE = {"lock_minutes": 5, "blank_minutes": 10}
MAX_IDLE_MINUTES = 240

_ZONE_RE = re.compile(r"^[A-Za-z0-9_+\-]+(/[A-Za-z0-9_+\-]+){0,2}$")
_LAYOUT_RE = re.compile(r"^[a-z0-9_]{2,12}$")
_SKIP_DIRS = {"posix", "right"}


def valid_zone(zone, root=ZONEINFO):
    """True only for a real tz database file under `root` (checked by its
    TZif magic), never a path that escapes it or a non-zone file."""
    if not isinstance(zone, str) or not _ZONE_RE.match(zone):
        return False
    if zone.split("/")[0] in _SKIP_DIRS:
        return False
    real_root = os.path.realpath(root)
    path = os.path.realpath(os.path.join(root, zone))
    if not path.startswith(real_root + os.sep):
        return False
    try:
        with open(path, "rb") as f:
            return f.read(4) == b"TZif"
    except OSError:
        return False


def list_zones(root=ZONEINFO):
    zones = []
    for dirpath, dirnames, filenames in os.walk(root):
        rel_dir = os.path.relpath(dirpath, root)
        if rel_dir == ".":
            dirnames[:] = [d for d in dirnames if d not in _SKIP_DIRS]
        for name in filenames:
            zone = name if rel_dir == "." else f"{rel_dir}/{name}"
            if valid_zone(zone, root):
                zones.append(zone)
    return sorted(zones)


def current_zone(localtime="/etc/localtime", timezone_file="/etc/timezone", root=ZONEINFO):
    try:
        target = os.readlink(localtime)
        prefix = root.rstrip("/") + "/"
        if target.startswith(prefix) and valid_zone(target[len(prefix):], root):
            return target[len(prefix):]
    except OSError:
        pass
    try:
        with open(timezone_file) as f:
            zone = f.readline().strip()
        if valid_zone(zone, root):
            return zone
    except OSError:
        pass
    return "UTC"


def list_layouts(path=XKB_LIST):
    """(code, description) pairs from xkeyboard-config's rules list."""
    layouts = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            in_section = False
            for line in f:
                if line.startswith("!"):
                    in_section = line.strip() == "! layout"
                    continue
                if in_section and line.strip():
                    code, _, desc = line.strip().partition(" ")
                    if _LAYOUT_RE.match(code):
                        layouts.append((code, desc.strip() or code))
    except OSError:
        pass
    return layouts or [("us", "English (US)")]


def valid_layout(code, layouts=None):
    if not isinstance(code, str) or not _LAYOUT_RE.match(code):
        return False
    return code in {c for c, _ in (layouts if layouts is not None else list_layouts())}


def read_keyboard_layout(conf=KEYBOARD_CONF):
    try:
        with open(conf) as f:
            m = re.search(r'xkb_layout\s+"([a-z0-9_]+)"', f.read())
        if m:
            return m.group(1)
    except OSError:
        pass
    return "us"


def write_keyboard_layout(code, conf=KEYBOARD_CONF):
    if not _LAYOUT_RE.match(code):
        raise ValueError("invalid keyboard layout")
    os.makedirs(os.path.dirname(conf), exist_ok=True)
    with open(conf, "w") as f:
        f.write(f'input type:keyboard {{\n    xkb_layout "{code}"\n}}\n')


def read_idle(conf=IDLE_CONF):
    values = dict(DEFAULT_IDLE)
    try:
        with open(conf) as f:
            for line in f:
                m = re.fullmatch(r"(lock_minutes|blank_minutes)=(\d{1,4})\s*", line)
                if m:
                    values[m.group(1)] = min(int(m.group(2)), MAX_IDLE_MINUTES)
    except OSError:
        pass
    return values


def write_idle(lock_minutes, blank_minutes, conf=IDLE_CONF):
    lock = max(0, min(int(lock_minutes), MAX_IDLE_MINUTES))
    blank = max(0, min(int(blank_minutes), MAX_IDLE_MINUTES))
    os.makedirs(os.path.dirname(conf), exist_ok=True)
    with open(conf, "w") as f:
        f.write(f"lock_minutes={lock}\nblank_minutes={blank}\n")
    return {"lock_minutes": lock, "blank_minutes": blank}
