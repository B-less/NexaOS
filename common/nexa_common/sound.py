"""Sound devices and app volumes for Settings > Sound, through PipeWire's
PulseAudio interface (pactl, JSON output). Outputs are "sinks", inputs
"sources" (the ".monitor" copies of outputs left out), app streams
"sink-inputs". audioctl.py keeps the quick default-volume calls."""
import json
import subprocess


class SoundError(Exception):
    pass


def _pactl(*args, json_out=False):
    argv = ["pactl", *(["-f", "json"] if json_out else []), *args]
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=8)
    except FileNotFoundError:
        raise SoundError("pactl is not installed")
    except subprocess.TimeoutExpired:
        raise SoundError("the sound server didn't answer")
    if r.returncode != 0:
        raise SoundError(r.stderr.strip() or "the sound server refused that")
    return r.stdout


def _percent(volume):
    """Average of the channels' "value_percent" ("65%") as an int."""
    vals = []
    for ch in (volume or {}).values():
        try:
            vals.append(int(str(ch.get("value_percent", "0")).rstrip("%")))
        except (ValueError, AttributeError):
            pass
    return round(sum(vals) / len(vals)) if vals else 0


def parse_devices(text, default_name, inputs=False):
    out = []
    for d in json.loads(text or "[]"):
        name = d.get("name", "")
        if inputs and name.endswith(".monitor"):
            continue
        out.append({"name": name, "description": d.get("description") or name, "volume": _percent(d.get("volume")),
                    "mute": bool(d.get("mute")), "default": name == default_name})
    return out


def parse_apps(text):
    out = []
    for s in json.loads(text or "[]"):
        props = s.get("properties") or {}
        out.append({"index": s.get("index"), "app": props.get("application.name") or props.get("media.name") or "An app",
                    "icon": props.get("application.icon_name") or "audio-x-generic",
                    "volume": _percent(s.get("volume")), "mute": bool(s.get("mute"))})
    return out


def outputs():
    return parse_devices(_pactl("list", "sinks", json_out=True), _pactl("get-default-sink").strip())


def inputs():
    return parse_devices(_pactl("list", "sources", json_out=True), _pactl("get-default-source").strip(), inputs=True)


def apps():
    return parse_apps(_pactl("list", "sink-inputs", json_out=True))


def _pct(p):
    return f"{max(0, min(150, int(round(p))))}%"


def set_default(kind, name):
    _pactl("set-default-sink" if kind == "output" else "set-default-source", name)


def set_volume(kind, name, percent):
    _pactl("set-sink-volume" if kind == "output" else "set-source-volume", name, _pct(percent))


def set_mute(kind, name, mute):
    _pactl("set-sink-mute" if kind == "output" else "set-source-mute", name, "1" if mute else "0")


def set_app_volume(index, percent):
    _pactl("set-sink-input-volume", str(index), _pct(percent))


def set_app_mute(index, mute):
    _pactl("set-sink-input-mute", str(index), "1" if mute else "0")
