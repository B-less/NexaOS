"""Parsing of the system state shown in the Control Centre. No GTK here, so
it can be unit-tested anywhere."""
import glob
import json
import os
import re


def parse_volume(wpctl_output):
    """`wpctl get-volume @DEFAULT_AUDIO_SINK@` -> (percent, muted), or None."""
    m = re.search(r"Volume:\s*([0-9.]+)(\s*\[MUTED\])?", wpctl_output or "")
    if not m:
        return None
    return int(round(float(m.group(1)) * 100)), bool(m.group(2))


def parse_brightness(brightnessctl_m_output):
    """`brightnessctl -m` (device,class,current,percent%,max) -> percent, or None."""
    for line in (brightnessctl_m_output or "").splitlines():
        parts = line.split(",")
        if len(parts) >= 4 and parts[1] == "backlight" and parts[3].endswith("%"):
            try:
                return int(parts[3][:-1])
            except ValueError:
                return None
    return None


def battery(sys_root="/sys/class/power_supply"):
    """(percent, status) of the first battery, or None on a desktop."""
    for bat in sorted(glob.glob(os.path.join(sys_root, "BAT*"))):
        try:
            with open(os.path.join(bat, "capacity")) as f:
                cap = int(f.read().strip())
        except (OSError, ValueError):
            continue
        try:
            with open(os.path.join(bat, "status")) as f:
                status = f.read().strip()
        except OSError:
            status = "Unknown"
        return cap, status
    return None


def _field(entry, key):
    v = entry.get(key, {})
    return v.get("data") if isinstance(v, dict) else v


def parse_mako_history(makoctl_json, cleared_up_to=0):
    """`makoctl history` -> newest-first [{id, app, summary, body}], skipping
    ids at or below cleared_up_to (what "Clear all" already hid)."""
    try:
        data = json.loads(makoctl_json or "{}").get("data", [])
    except (ValueError, AttributeError):
        return []
    entries = data[0] if data and isinstance(data[0], list) else data
    out = []
    for e in entries:
        if not isinstance(e, dict):
            continue
        try:
            nid = int(_field(e, "id") or 0)
        except (TypeError, ValueError):
            nid = 0
        if nid and nid <= cleared_up_to:
            continue
        out.append({"id": nid, "app": _field(e, "app-name") or "",
                    "summary": _field(e, "summary") or "", "body": _field(e, "body") or ""})
    return sorted(out, key=lambda n: -n["id"])


def notifications(list_json, history_json, cleared_up_to=0):
    """Showing (`makoctl list`) plus past (`makoctl history`), newest first."""
    seen, out = set(), []
    for n in parse_mako_history(list_json, cleared_up_to) + parse_mako_history(history_json, cleared_up_to):
        key = n["id"] or (n["app"], n["summary"], n["body"])
        if key not in seen:
            seen.add(key)
            out.append(n)
    return sorted(out, key=lambda n: -n["id"])


def dnd_on(makoctl_mode_output):
    return "do-not-disturb" in (makoctl_mode_output or "").split()
