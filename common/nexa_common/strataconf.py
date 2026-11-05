"""Strata's settings (~/.config/nexa/strata.conf), for Settings > Layers.
The same file is read by Orbit (the stack, power), Living (tint, style,
dim) and Nexa Signals (notifications): tools-c/nexa-strata/layers.c is the
reference parser. Layer moods: TODOnexa_identity.md, phase 5.

    show=yes|no            tint=0..0.3
    layer.N.name=Work      layer.N.signals=all|quiet   layer.N.mute=App,App
    layer.N.style=         layer.N.dim=0..0.6          layer.N.power=|powersave|performance
"""
import os

LAYERS = 5
TINT_MAX, DIM_MAX = 0.3, 0.6
POWER = [("", "Unchanged"), ("powersave", "Saving"), ("performance", "Fast")]
DEFAULT = {"show": True, "tint": 0.14,
           "layers": [{"name": "", "quiet": False, "mute": [], "style": "", "dim": 0.0, "power": ""} for _ in range(LAYERS)]}
# A starting point people can pick from (each a full mood).
PRESETS = {
    "focus": {"name": "Work", "quiet": True, "mute": [], "style": "", "dim": 0.0, "power": "performance"},
    "night": {"name": "Night", "quiet": True, "mute": [], "style": "none", "dim": 0.45, "power": "powersave"},
    "play": {"name": "Play", "quiet": False, "mute": [], "style": "", "dim": 0.0, "power": "performance"},
}


def path(home=None):
    cfg = os.environ.get("XDG_CONFIG_HOME") if home is None else None
    base = cfg if cfg and os.path.isabs(cfg) else os.path.join(home or os.path.expanduser("~"), ".config")
    return os.path.join(base, "nexa", "strata.conf")


def _clean(text, limit):
    return "".join(c for c in text if c.isprintable()).strip()[:limit]


def _number(v, lo, hi, default):
    try:
        x = float(v)
    except ValueError:
        return default
    return default if x != x else min(hi, max(lo, x))


def parse(text):
    conf = {"show": True, "tint": 0.14, "layers": [dict(m, mute=[]) for m in DEFAULT["layers"]]}
    for line in (text or "").splitlines():
        if line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        if k == "show":
            conf["show"] = v != "no"
        elif k == "tint":
            conf["tint"] = _number(v, 0, TINT_MAX, conf["tint"])
        elif k.startswith("layer.") and len(k) > 8 and k[6] in "12345" and k[7] == ".":
            m, key = conf["layers"][int(k[6]) - 1], k[8:]
            if key == "name":
                m["name"] = _clean(v, 30)
            elif key == "signals":
                m["quiet"] = v.strip() == "quiet"
            elif key == "mute":
                m["mute"] = [a for a in (_clean(x, 60) for x in v.split(",")) if a][:50]
            elif key == "style" and len(v) < 24 and all(c.islower() or c == "-" for c in v):
                m["style"] = v
            elif key == "dim":
                m["dim"] = _number(v, 0, DIM_MAX, 0.0)
            elif key == "power" and v in ("", "powersave", "performance"):
                m["power"] = v
    return conf


def load(home=None):
    try:
        with open(path(home)) as f:
            return parse(f.read())
    except OSError:
        return parse("")


def render(conf):
    lines = ["# Strata: Settings > Layers (read by Orbit, Living and Nexa Signals)",
             f"show={'yes' if conf['show'] else 'no'}", f"tint={conf['tint']:g}"]
    for i, m in enumerate(conf["layers"][:LAYERS], 1):
        name = _clean(m.get("name", ""), 30)
        if name:
            lines.append(f"layer.{i}.name={name}")
        if m.get("quiet"):
            lines.append(f"layer.{i}.signals=quiet")
        mute = [a for a in (_clean(x.replace(",", " "), 60) for x in m.get("mute", [])) if a]
        if mute:
            lines.append(f"layer.{i}.mute={','.join(mute)}")
        if m.get("style"):
            lines.append(f"layer.{i}.style={m['style']}")
        if m.get("dim"):
            lines.append(f"layer.{i}.dim={min(DIM_MAX, max(0.0, float(m['dim']))):.2f}")
        if m.get("power") in ("powersave", "performance"):
            lines.append(f"layer.{i}.power={m['power']}")
    return "\n".join(lines) + "\n"


def save(conf, home=None):
    """Written whole and swapped in, so a reader never sees half a file."""
    p = path(home)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    tmp = p + ".tmp"
    with open(tmp, "w") as f:
        f.write(render(conf))
    os.replace(tmp, p)
    return parse(render(conf))
