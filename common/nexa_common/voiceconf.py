"""Nexa's voice settings: ~/.config/nexa/voice.conf, read by nexa-voice
(tools-c/nexa-voice), which notices changes at once.

    voice=af_heart     one of VOICES (Kokoro's voices in nexa-mindd)
    speed=1.0          0.5 .. 2
    colours=voice      the helix: voice, cycle, calm or ember
"""
import os

VOICES = (
    ("af_heart", "Heart: warm, American"),
    ("af_bella", "Bella: bright, American"),
    ("am_michael", "Michael: calm, American"),
    ("bf_emma", "Emma: British"),
)
COLOURS = (
    ("voice", "Follow the voice"),
    ("cycle", "Rainbow"),
    ("calm", "Calm, in the theme's colour"),
    ("ember", "Ember"),
)
DEFAULTS = {"voice": "af_heart", "speed": 1.0, "colours": "voice"}


def path():
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return os.path.join(base, "nexa", "voice.conf")


def _clean(key, value):
    if key == "voice":
        return value if value in dict(VOICES) else None
    if key == "colours":
        return value if value in dict(COLOURS) else None
    if key == "speed":
        try:
            v = round(float(value), 2)
        except (TypeError, ValueError):
            return None
        return v if 0.5 <= v <= 2.0 else None
    return None


def load():
    conf = dict(DEFAULTS)
    try:
        with open(path()) as f:
            for line in f:
                key, sep, value = line.partition("=")
                if sep and not key.startswith("#"):
                    v = _clean(key.strip(), value.strip())
                    if v is not None:
                        conf[key.strip()] = v
    except OSError:
        pass
    return conf


def save(**changes):
    """Merge valid changes into the file (others are ignored); returns the
    settings now in force."""
    conf = load()
    for key, value in changes.items():
        v = _clean(key, value)
        if v is not None:
            conf[key] = v
    p = path()
    os.makedirs(os.path.dirname(p), exist_ok=True)
    tmp = p + ".tmp"
    with open(tmp, "w") as f:
        f.write("# Nexa's voice (Settings > Assistant)\n")
        f.write(f"voice={conf['voice']}\nspeed={conf['speed']}\ncolours={conf['colours']}\n")
    os.replace(tmp, p)
    return conf
