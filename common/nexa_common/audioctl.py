import re
import subprocess

# Thin wrapper around wpctl (shipped by wireplumber), the standard CLI for
# controlling PipeWire's default audio sink. Shared by the Volume Mixer app
# and nexa-ai's volume tools, same reuse pattern as apkops.py for apk.

_SINK = "@DEFAULT_AUDIO_SINK@"
_VOLUME_RE = re.compile(r"Volume:\s*([\d.]+)(\s*\[MUTED\])?")


class AudioError(Exception):
    pass


def _run(argv, timeout=10):
    try:
        return subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except FileNotFoundError:
        raise AudioError("wpctl is not installed")
    except subprocess.TimeoutExpired:
        raise AudioError("wpctl timed out")


def get_volume():
    result = _run(["wpctl", "get-volume", _SINK])
    if result.returncode != 0:
        raise AudioError(result.stderr.strip() or "wpctl get-volume failed")
    m = _VOLUME_RE.search(result.stdout)
    if not m:
        raise AudioError("unexpected wpctl output: " + result.stdout.strip())
    return {"volume": round(float(m.group(1)) * 100), "muted": bool(m.group(2))}


def set_volume(percent):
    percent = max(0, min(100, round(float(percent))))
    result = _run(["wpctl", "set-volume", _SINK, f"{percent}%"])
    if result.returncode != 0:
        raise AudioError(result.stderr.strip() or "wpctl set-volume failed")
    return get_volume()


def set_mute(muted):
    result = _run(["wpctl", "set-mute", _SINK, "1" if muted else "0"])
    if result.returncode != 0:
        raise AudioError(result.stderr.strip() or "wpctl set-mute failed")
    return get_volume()
