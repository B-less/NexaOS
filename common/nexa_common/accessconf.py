"""Settings > Accessibility: text size, pointer size, reduce motion, high
contrast. Kept in ~/.config/nexa/accessibility.json.

themes.py reads it when it writes GTK's settings (font size, pointer size,
animations) and GSettings (text-scaling-factor, cursor-size); this module
writes the compositor side: Sway ~/.config/sway/config.d/31-nexa-access.conf
and Hyprland ~/.config/hypr/nexa-access.conf (pointer size, animations off),
and turns the Living backgrounds' reduced motion on with it."""
import json
import os

from . import compositor

TEXT_SCALES = (1.0, 1.15, 1.3, 1.5)
CURSOR_SIZES = (24, 32, 48, 64)
DEFAULTS = {"text_scale": 1.0, "cursor_size": 24, "reduce_motion": False, "high_contrast": False,
            "theme_before_contrast": None}


def _config_dir(home=None):
    if home:
        return os.path.join(home, ".config")
    return os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")


def prefs_path(home=None):
    return os.path.join(_config_dir(home), "nexa", "accessibility.json")


def clean(p):
    p = p if isinstance(p, dict) else {}
    out = dict(DEFAULTS)
    if p.get("text_scale") in TEXT_SCALES:
        out["text_scale"] = p["text_scale"]
    if p.get("cursor_size") in CURSOR_SIZES:
        out["cursor_size"] = p["cursor_size"]
    for k in ("reduce_motion", "high_contrast"):
        if isinstance(p.get(k), bool):
            out[k] = p[k]
    if isinstance(p.get("theme_before_contrast"), str):
        out["theme_before_contrast"] = p["theme_before_contrast"]
    return out


def load(home=None):
    try:
        with open(prefs_path(home)) as f:
            return clean(json.load(f))
    except (OSError, ValueError):
        return dict(DEFAULTS)


def sway_conf(p):
    return ("# Written by Settings > Accessibility; changes there replace this file.\n"
            f"seat * xcursor_theme Adwaita {p['cursor_size']}\n")


def hypr_conf(p):
    return ("# Written by Settings > Accessibility; changes there replace this file.\n"
            f"env = XCURSOR_SIZE,{p['cursor_size']}\n"
            f"exec = hyprctl setcursor Adwaita {p['cursor_size']}\n"
            f"animations {{\n    enabled = {'false' if p['reduce_motion'] else 'true'}\n}}\n")


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        f.write(text)
    os.replace(tmp, path)


def save(changes):
    """Merge changes, write everything, apply what can be applied now."""
    from . import magic, themes
    p = clean({**load(), **(changes or {})})
    current_theme = themes.load_settings()["theme"]
    theme = None
    if "high_contrast" in (changes or {}):
        if p["high_contrast"] and current_theme != "contrast":
            p["theme_before_contrast"] = current_theme
            theme = "contrast"
        elif not p["high_contrast"] and current_theme == "contrast":
            theme = p["theme_before_contrast"] if p["theme_before_contrast"] in themes.THEMES else themes.DEFAULT_THEME
    _write(prefs_path(), json.dumps(p, indent=1) + "\n")
    cfg = _config_dir()
    _write(os.path.join(cfg, "sway", "config.d", "31-nexa-access.conf"), sway_conf(p))
    _write(os.path.join(cfg, "hypr", "nexa-access.conf"), hypr_conf(p))
    if "reduce_motion" in (changes or {}):
        try:
            magic.set_living(reduced_motion=p["reduce_motion"])
        except Exception:
            pass
    # GTK settings, GSettings and (with a theme change) every colour; it
    # also reloads the compositor, which picks up the files above.
    themes.apply(name=theme)
    which = compositor.which()
    if which == "hyprland":
        compositor._call(["hyprctl", "setcursor", "Adwaita", str(p["cursor_size"])])
    return p
