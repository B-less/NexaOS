"""NexaOS themes: one palette restyles the top bar (waybar), notifications
(mako), the terminal (foot), window borders (sway) and the shell panels
(start menu, Control Centre), with adjustable transparency.

The choice lives in ~/.config/nexa/theme.json; `apply()` renders every
target's config into ~/.config and reloads whatever is running. Used by
Settings > Personalization and, at login, `nexa-theme apply --saved`.

    python3 -m nexa_common.themes apply <name> [--opacity 0.72]
    python3 -m nexa_common.themes apply --saved [--no-reload]
    python3 -m nexa_common.themes list
"""
import json
import os
import subprocess
import sys

# bg/surface are RGB triples (transparency is applied separately); the rest
# are hex colours. term = the 8 regular ANSI colours for the terminal.
THEMES = {
    "horizon": {
        "label": "Horizon",
        "tagline": "Deep space blue, the NexaOS default",
        "dark": True,
        "bg": (8, 12, 30), "surface": (20, 28, 58),
        "text": "#e6ecff", "muted": "#8f9bc4",
        "accent": "#4da3ff", "accent2": "#8a6bff",
        "warn": "#ffb547", "crit": "#ff5c7a",
        "term": ["#0b1020", "#ff5c7a", "#5ee6a8", "#ffd166", "#4da3ff", "#a98bff", "#4fd6ff", "#c9d4ff"],
    },
    "nebula": {
        "label": "Nebula",
        "tagline": "Violet clouds and starlight pink",
        "dark": True,
        "bg": (22, 10, 36), "surface": (44, 22, 66),
        "text": "#f3e8ff", "muted": "#b39ac9",
        "accent": "#c77dff", "accent2": "#ff6ec7",
        "warn": "#ffc46b", "crit": "#ff5d8f",
        "term": ["#1a0b2b", "#ff5d8f", "#7af5c3", "#ffd27a", "#9d7dff", "#d17bff", "#7ad8ff", "#ead9ff"],
    },
    "aurora": {
        "label": "Aurora",
        "tagline": "Northern lights over a polar night",
        "dark": True,
        "bg": (5, 22, 26), "surface": (12, 44, 48),
        "text": "#e3fffb", "muted": "#8ec4bd",
        "accent": "#3ddbd9", "accent2": "#6fff9e",
        "warn": "#ffd166", "crit": "#ff6b6b",
        "term": ["#071a1c", "#ff6b6b", "#6fff9e", "#ffd166", "#3ddbd9", "#b388ff", "#7ff0ff", "#d6fff8"],
    },
    "eclipse": {
        "label": "Eclipse",
        "tagline": "Pure black with a golden corona",
        "dark": True,
        "bg": (0, 0, 0), "surface": (24, 20, 14),
        "text": "#fff4e0", "muted": "#a8977c",
        "accent": "#ffb547", "accent2": "#ff7a45",
        "warn": "#ffd166", "crit": "#ff4d4d",
        "term": ["#000000", "#ff4d4d", "#9be37a", "#ffb547", "#6aa9ff", "#d38bff", "#5fd7d7", "#f2e6d0"],
    },
    "daybreak": {
        "label": "Daybreak",
        "tagline": "Light and airy, for bright rooms",
        "dark": False,
        "bg": (244, 247, 252), "surface": (255, 255, 255),
        "text": "#1d2433", "muted": "#5d6b85",
        "accent": "#2f6fed", "accent2": "#7c4dff",
        "warn": "#c77700", "crit": "#d6264a",
        "term": ["#1d2433", "#d6264a", "#1f9d55", "#b7791f", "#2f6fed", "#7c4dff", "#0e9aa7", "#e8ecf4"],
    },
}
DEFAULT_THEME = "horizon"
DEFAULT_OPACITY = 0.72
DEFAULT_GLASS = True  # app windows let the (blurred) desktop show through
# How glass looks: "tinted" (the theme's colour, calmer; the default) or
# "clear" (more see-through, frosted rather than coloured).
GLASS_TINTS = ("clear", "tinted")
DEFAULT_GLASS_TINT = "tinted"
# The light along a glass surface's top edge (white, this opaque), so glass
# reads as a layer above the content and controls stand apart from it.
EDGE_LIGHT = {"clear": 0.24, "tinted": 0.13}
MIN_OPACITY, MAX_OPACITY = 0.35, 1.0


def _home():
    return os.environ.get("HOME", "/home/user")


def _config_dir(home=None):
    return os.path.join(home or _home(), ".config")


def _data_dir(home=None):
    return os.path.join(home or _home(), ".local", "share")


def settings_path(home=None):
    return os.path.join(_config_dir(home), "nexa", "theme.json")


def clamp_opacity(value):
    try:
        value = float(value)
    except (TypeError, ValueError):
        return DEFAULT_OPACITY
    return max(MIN_OPACITY, min(MAX_OPACITY, value))


def load_settings(home=None):
    try:
        with open(settings_path(home)) as f:
            data = json.load(f)
    except (OSError, ValueError):
        data = {}
    name = data.get("theme") if data.get("theme") in THEMES else DEFAULT_THEME
    glass = data.get("glass", DEFAULT_GLASS)
    style = data.get("icon_style") if data.get("icon_style") in ICON_STYLES else "colour"
    tint = data.get("glass_tint") if data.get("glass_tint") in GLASS_TINTS else DEFAULT_GLASS_TINT
    return {"theme": name, "opacity": clamp_opacity(data.get("opacity", DEFAULT_OPACITY)),
            "glass": glass if isinstance(glass, bool) else DEFAULT_GLASS, "icon_style": style, "glass_tint": tint}


def _hex(rgb):
    return "#%02x%02x%02x" % rgb


def _rgba(rgb, alpha):
    return "rgba(%d, %d, %d, %.2f)" % (rgb[0], rgb[1], rgb[2], alpha)


def _hex_alpha(rgb, alpha):
    return "#%02x%02x%02x%02x" % (rgb[0], rgb[1], rgb[2], int(round(alpha * 255)))


def _rgb_of(hex_colour):
    h = hex_colour.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


# ---- renderers -----------------------------------------------------------

def waybar_css(t, opacity):
    hover = "rgba(255, 255, 255, 0.10)" if t["dark"] else "rgba(0, 0, 0, 0.06)"
    pill = _rgba(t["surface"], min(1.0, opacity * 0.9))
    return f"""/* Generated by nexa-theme ({t['label']}); edits are overwritten. */
* {{
    font-family: "Inter", "Noto Sans", "Font Awesome 6 Free", "Font Awesome 6 Brands", sans-serif;
    font-size: 13px;
    min-height: 0;
    border: none;
    border-radius: 0;
}}
window#waybar {{
    background: {_rgba(t['bg'], opacity)};
    color: {t['text']};
    border-bottom: 1px solid {_rgba(_rgb_of(t['accent']), 0.25)};
    box-shadow: inset 0 -1px rgba(255, 255, 255, 0.10);
}}
tooltip {{
    background: {_rgba(t['surface'], 0.97)};
    border: 1px solid {t['accent']};
    border-radius: 10px;
}}
tooltip label {{ color: {t['text']}; }}
#custom-start {{
    color: {t['accent']};
    font-size: 16px;
    padding: 0 14px;
    margin: 4px 4px 4px 8px;
    border-radius: 10px;
    background: {pill};
}}
#custom-start:hover {{ background: {t['accent']}; color: {_hex(t['bg'])}; }}
#workspaces {{ margin: 4px 0; }}
#workspaces button {{
    color: {t['muted']};
    padding: 0 9px;
    margin: 0 2px;
    border-radius: 9px;
    background: transparent;
}}
#workspaces button:hover {{ background: {hover}; }}
#workspaces button.focused {{
    color: {_hex(t['bg']) if t['dark'] else '#ffffff'};
    background: {t['accent']};
}}
#workspaces button.urgent {{ background: {t['crit']}; color: #ffffff; }}
#clock {{
    color: {t['text']};
    font-weight: 600;
    padding: 0 14px;
}}
#network, #backlight, #pulseaudio, #battery, #custom-notify, #custom-control, #scratchpad, #mode {{
    color: {t['text']};
    padding: 0 11px;
    margin: 4px 2px;
    border-radius: 10px;
    background: {pill};
}}
#network:hover, #backlight:hover, #pulseaudio:hover, #battery:hover,
#custom-notify:hover, #custom-control:hover {{ background: {hover}; }}
#network.disconnected {{ color: {t['muted']}; }}
#battery.charging, #battery.plugged {{ color: {t['accent']}; }}
#battery.warning:not(.charging) {{ color: {t['warn']}; }}
#battery.critical:not(.charging) {{ color: #ffffff; background: {t['crit']}; }}
#custom-notify.dnd {{ color: {t['muted']}; }}
#custom-control {{ color: {t['accent']}; margin-right: 8px; }}
"""


def sourceview_scheme(t):
    """~/.local/share/gtksourceview-4/styles/nexa.xml: the Editor's text area
    and syntax colours in the current theme (it used a fixed grey scheme)."""
    pal = app_palette(t)
    term = t["term"]
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!-- Generated by nexa-theme ({t['label']}); edits are overwritten. -->
<style-scheme id="nexa" name="NexaOS" version="1.0">
  <style name="text" foreground="{pal['text_strong']}" background="{pal['bg_deep']}"/>
  <style name="current-line" background="{pal['surface']}"/>
  <style name="current-line-number" foreground="{pal['accent']}" background="{pal['surface']}" bold="true"/>
  <style name="line-numbers" foreground="{pal['muted']}" background="{pal['bg']}"/>
  <style name="selection" foreground="{pal['selected_text']}" background="{pal['selected']}"/>
  <style name="cursor" foreground="{pal['accent']}"/>
  <style name="bracket-match" foreground="{pal['accent']}" bold="true"/>
  <style name="search-match" background="{pal['selected']}"/>
  <style name="def:comment" foreground="{pal['muted']}" italic="true"/>
  <style name="def:keyword" foreground="{t['accent2']}" bold="true"/>
  <style name="def:statement" foreground="{t['accent2']}"/>
  <style name="def:type" foreground="{t['accent']}"/>
  <style name="def:function" foreground="{term[6]}"/>
  <style name="def:string" foreground="{term[2]}"/>
  <style name="def:special-char" foreground="{term[3]}"/>
  <style name="def:number" foreground="{term[3]}"/>
  <style name="def:boolean" foreground="{term[3]}"/>
  <style name="def:constant" foreground="{term[3]}"/>
  <style name="def:preprocessor" foreground="{term[5]}"/>
  <style name="def:identifier" foreground="{pal['text']}"/>
  <style name="def:error" foreground="{t['crit']}" underline="error"/>
  <style name="def:heading" foreground="{t['accent']}" bold="true"/>
</style-scheme>
"""


def swaylock_conf(t):
    """~/.config/swaylock/config: colours for nexa-lock (swaylock-effects).
    Colours are RRGGBBAA; the ring is the accent, green while checking, red
    on a wrong password."""
    def c(hex_colour, alpha="ff"):
        return hex_colour.lstrip("#") + alpha
    bg = _hex(t["bg"])
    ok = t["term"][2]
    return f"""# Generated by nexa-theme ({t['label']}); edits are overwritten.
color={c(bg)}
inside-color={c(bg, "aa")}
inside-clear-color={c(bg, "aa")}
inside-ver-color={c(bg, "aa")}
inside-wrong-color={c(bg, "aa")}
ring-color={c(t['accent'])}
ring-clear-color={c(t['accent2'])}
ring-ver-color={c(ok)}
ring-wrong-color={c(t['crit'])}
key-hl-color={c(t['accent2'])}
bs-hl-color={c(t['crit'])}
line-color=00000000
line-clear-color=00000000
line-ver-color=00000000
line-wrong-color=00000000
separator-color=00000000
text-color={c(t['text'])}
text-clear-color={c(t['text'])}
text-ver-color={c(t['text'])}
text-wrong-color={c(t['crit'])}
"""


def lock_conf(t):
    """~/.config/nexa/lock.conf: colours for nexa-locker (C), key=#rrggbb."""
    lines = [f"# Generated by nexa-theme ({t['label']}); edits are overwritten.",
             f"bg={_hex(t['bg'])}", f"surface={_hex(t['surface'])}", f"text={t['text']}",
             f"muted={t['muted']}", f"accent={t['accent']}", f"accent2={t['accent2']}",
             f"crit={t['crit']}", f"ok={t['term'][2]}", f"warn={t['warn']}",
             "wallpaper=/usr/share/backgrounds/nexaos/wallpaper.png"]
    return "\n".join(lines) + "\n"


def mako_config(t, opacity):
    return f"""# Generated by nexa-theme ({t['label']}); edits are overwritten.
font=Inter 11
background-color={_hex_alpha(t['surface'], max(opacity, 0.85))}
text-color={t['text']}
border-color={t['accent']}
border-size=2
border-radius=14
padding=14
margin=12
width=360
default-timeout=6000
max-history=50

[urgency=critical]
border-color={t['crit']}
default-timeout=0

[mode=do-not-disturb]
invisible=1
"""


def foot_ini(t, opacity):
    term = t["term"]
    lines = [
        f"# Generated by nexa-theme ({t['label']}); edits are overwritten.",
        "[main]",
        "font=JetBrains Mono:size=11,monospace:size=11",
        "pad=10x10",
        "",
        "[colors-dark]",  # foot 1.2x: [colors] is deprecated
        f"alpha={opacity:.2f}",
        f"background={_hex(t['bg'])[1:]}",
        f"foreground={t['text'][1:]}",
    ]
    for i, c in enumerate(term):
        lines.append(f"regular{i}={c[1:]}")
    for i, c in enumerate(term):  # same palette for bright colours
        lines.append(f"bright{i}={c[1:]}")
    # foot's own title bar (like the Nexa apps' titlebars), so the terminal
    # can be dragged, minimized and closed on Hyprland too, which draws no
    # title bars for apps. Colours are AARRGGBB.
    surface = _hex(t["surface"])[1:]
    lines += [
        "",
        "[csd]",
        "preferred=client",
        "size=30",
        "font=Inter:size=10,sans:size=10",
        f"color=ff{surface}",
        "button-width=36",
        f"button-color=ff{t['text'][1:]}",
        f"button-close-color=ff{t['crit'][1:]}",
        "double-click-to-maximize=yes",
    ]
    return "\n".join(lines) + "\n"


def sway_conf(t):
    inactive = _hex(t["surface"])
    text_on_accent = _hex(t["bg"]) if t["dark"] else "#ffffff"
    return f"""# Generated by nexa-theme ({t['label']}); edits are overwritten.
# class                 border      background  text        indicator   child_border
client.focused          {t['accent']} {t['accent']} {text_on_accent} {t['accent2']} {t['accent']}
client.focused_inactive {inactive} {inactive} {t['muted']} {inactive} {inactive}
client.unfocused        {inactive} {inactive} {t['muted']} {inactive} {inactive}
client.urgent           {t['crit']} {t['crit']} #ffffff {t['crit']} {t['crit']}
"""


def _mix(a, b, t):
    """Blend two colours ("#rrggbb" or RGB tuples); t=0 -> a, t=1 -> b."""
    a = _rgb_of(a) if isinstance(a, str) else a
    b = _rgb_of(b) if isinstance(b, str) else b
    return _hex(tuple(int(round(x + (y - x) * t)) for x, y in zip(a, b)))


def glass_alpha(compositor_name, tint=DEFAULT_GLASS_TINT):
    """How solid glass windows are: Hyprland blurs what is behind them, so
    they can be clearer; Sway can't blur, so they stay nearly solid there.
    Clear glass lets more through than tinted."""
    blurred = compositor_name == "hyprland"
    if tint == "clear":
        return 0.62 if blurred else 0.86
    return 0.84 if blurred else 0.94


def app_palette(t, glass=None, tint=DEFAULT_GLASS_TINT):
    """Named colour roles every Nexa app's stylesheet refers to (as
    @nexa_<role>), so the apps follow the theme instead of fixed greys.
    glass (0..1, e.g. 0.84) is how solid app windows are, so the desktop
    shows through; text, fields and buttons stay solid."""
    dark = t["dark"]
    bg, surface, text = _hex(t["bg"]), _hex(t["surface"]), t["text"]
    lift = "#ffffff" if dark else "#000000"
    pal = {
        "bg": _mix(bg, surface, 0.35),
        "bg_deep": bg,
        "surface": _mix(bg, surface, 0.7),
        "surface_hi": _mix(surface, lift, 0.06),
        "hover": _mix(surface, lift, 0.13),
        "field": _mix(bg, surface, 0.55),
        "border": _mix(surface, text, 0.16),
        "text": _mix(text, t["muted"], 0.15),
        "text_strong": text,
        "muted": t["muted"],
        "disabled": _mix(t["muted"], bg, 0.45),
        "accent": t["accent"],
        "accent_hover": _mix(t["accent"], lift, 0.18),
        "on_accent": bg if dark else "#ffffff",
        "selected": _mix(surface, t["accent"], 0.32),
        "selected_text": text,
        "crit": t["crit"],
        "crit_hover": _mix(t["crit"], lift, 0.18),
        "ok": t["term"][2],
        "warn": t["warn"],
    }
    # "glass" is painted once, on each toplevel window (see titlebar.py).
    # In glass mode the many inner widgets that paint @nexa_bg go clear, as
    # stacked translucent layers would add up to solid again.
    pal["glass"] = pal["bg"]
    pal["titlebar"] = pal["surface"]  # outside the window's own background, so it carries the glass itself
    pal["edge"] = "rgba(255, 255, 255, 0.00)"  # the edge light: only on glass
    if glass:
        def rgba(hex_colour, alpha):
            r, g, b = _rgb_of(hex_colour)
            return f"rgba({r}, {g}, {b}, {alpha:.2f})"
        # Clear glass is frosted: a touch of light instead of the theme's colour.
        frost = (lambda c: _mix(c, "#ffffff", 0.07 if dark else 0.35)) if tint == "clear" else (lambda c: c)
        pal["glass"] = rgba(frost(pal["bg"]), glass)
        pal["titlebar"] = rgba(frost(pal["surface"]), glass)
        pal["bg"] = rgba(pal["bg"], 0)
        pal["surface"] = rgba(pal["surface"], 0.45 if tint == "clear" else 0.6)
        pal["edge"] = f"rgba(255, 255, 255, {EDGE_LIGHT.get(tint, EDGE_LIGHT[DEFAULT_GLASS_TINT]):.2f})"
    return pal


def app_palette_css(t, glass=None, tint=DEFAULT_GLASS_TINT):
    return "".join(f"@define-color nexa_{k} {v};\n" for k, v in app_palette(t, glass, tint).items())


def hypr_glass_conf(t, glass, tint):
    """~/.config/hypr/nexa-glass.conf, sourced at the *end* of hyprland.conf
    (after its own blur settings): the blur for clear or tinted glass, and
    window borders lit from the top edge."""
    if not glass:
        return f"# Generated by nexa-theme ({t['label']}): glass windows are off.\n"
    a1, a2 = t["accent"].lstrip("#"), t["accent2"].lstrip("#")
    clear = tint == "clear"
    light = "ffffff88" if clear else "ffffff55"
    return f"""# Generated by nexa-theme ({t['label']}, {tint} glass); edits are overwritten.
decoration {{
    blur {{
        size = {10 if clear else 8}
        passes = 3
        vibrancy = {0.28 if clear else 0.17}
        brightness = {1.0 if clear else 0.86}
        noise = {0.012 if clear else 0.0117}
    }}
}}
general {{
    col.active_border = rgba({light}) rgba({a1}ee) rgba({a2}cc) 90deg
}}
"""


def hypr_conf(t):
    """~/.config/hypr/nexa-theme.conf, sourced by /etc/hypr/hyprland.conf."""
    a1, a2 = t["accent"].lstrip("#"), t["accent2"].lstrip("#")
    inactive = "%02x%02x%02x" % t["surface"]
    return f"""# Generated by nexa-theme ({t['label']}); edits are overwritten.
general {{
    col.active_border = rgba({a1}ee) rgba({a2}ee) 45deg
    col.inactive_border = rgba({inactive}aa)
}}
"""


# The NexaHexa icon set (assets/iconset) has a variant per theme
# (NexaHexa-<theme>: app tiles tinted to match). Used once it is installed in
# one of these folders; until then the icons stay Adwaita.
ICON_DIRS = ("/usr/share/icons", "/usr/local/share/icons")
# How app icons look (Settings > Personalization): full colour, dark (a dark
# hexagon, the glyph in the app's colours) or tinted (every glyph in the
# theme's accent).
ICON_STYLES = ("colour", "dark", "tinted")


def icon_theme_for(name, home=None, style="colour"):
    """The icon theme that goes with theme `name` in icon style `style`."""
    dirs = list(ICON_DIRS) + [os.path.join(home or _home(), ".local", "share", "icons")]
    wanted = ([f"NexaHexa-{name}-{style}"] if style in ICON_STYLES and style != "colour" else []) + [f"NexaHexa-{name}", "NexaHexa"]
    for candidate in wanted:
        if any(os.path.isfile(os.path.join(d, candidate, "index.theme")) for d in dirs):
            return candidate
    return "Adwaita"


def follow_icon_theme(home=None):
    """Switch this (GTK) process's icons to the current theme's set, so open
    windows change icons along with colours. Call it from a theme-change
    handler; does nothing outside GTK or when the set isn't installed."""
    try:
        import gi
        gi.require_version("Gtk", "3.0")
        from gi.repository import Gtk
        settings = Gtk.Settings.get_default()
    except (ImportError, ValueError):
        return None
    if settings is None:
        return None
    current = load_settings(home)
    wanted = icon_theme_for(current["theme"], home, current["icon_style"])
    if settings.get_property("gtk-icon-theme-name") != wanted:
        settings.set_property("gtk-icon-theme-name", wanted)
    return wanted


def hexa_tile(icon_name):
    """True when `icon_name` resolves to a NexaHexa app tile (already a
    hexagon): Nexus and the desktop then show it as it is, without drawing
    their own hexagon behind it (a hexagon inside a hexagon)."""
    # Any name that resolves into the set counts: Alpine apps (the foot
    # terminal) get NexaHexa tiles through aliases (stage-iconset.sh).
    if not icon_name or str(icon_name).endswith("-symbolic") or str(icon_name).startswith("/"):
        return False
    try:
        import gi
        gi.require_version("Gtk", "3.0")
        from gi.repository import Gtk
        theme = Gtk.IconTheme.get_default()
        info = theme.lookup_icon(icon_name, 64, 0) if theme else None
    except (ImportError, ValueError):
        return False
    path = (info.get_filename() or "") if info else ""
    return "/NexaHexa" in path and not path.endswith("-symbolic.svg")


def gtk_settings(t, icon_theme="Adwaita"):
    """~/.config/gtk-3.0/settings.ini: without it GTK falls back to its light
    default, so dialogs, file pickers, menus and scrollbars came up light
    inside the dark apps."""
    return f"""# Generated by nexa-theme ({t['label']}); edits are overwritten.
[Settings]
gtk-application-prefer-dark-theme={'true' if t['dark'] else 'false'}
gtk-theme-name=Adwaita
gtk-icon-theme-name={icon_theme}
gtk-cursor-theme-name=Adwaita
gtk-cursor-theme-size=24
gtk-font-name=Inter, Noto Sans 10
gtk-enable-animations=true
"""


def panel_css(t, opacity):
    """GTK CSS for the start menu and Control Centre."""
    hover = "rgba(255, 255, 255, 0.08)" if t["dark"] else "rgba(0, 0, 0, 0.05)"
    field = "rgba(255, 255, 255, 0.07)" if t["dark"] else "rgba(0, 0, 0, 0.05)"
    panel_alpha = max(opacity, 0.80)
    return f"""
window {{ background-color: transparent; background-image: none; }}
#panel {{
    background-color: {_rgba(t['bg'], panel_alpha)};
    border: 1px solid {_rgba(_rgb_of(t['accent']), 0.35)};
    box-shadow: inset 0 1px rgba(255, 255, 255, 0.14);
    border-radius: 18px;
    padding: 18px;
}}
#title {{ color: {t['text']}; font-size: 15pt; font-weight: 700; }}
#subtitle, #muted {{ color: {t['muted']}; font-size: 9pt; }}
#section {{ color: {t['muted']}; font-size: 9pt; font-weight: 700; }}
label {{ color: {t['text']}; }}
entry {{
    background-color: {field}; background-image: none;
    color: {t['text']}; caret-color: {t['accent']};
    border: 1px solid {_rgba(_rgb_of(t['accent']), 0.45)};
    border-radius: 12px; padding: 8px 12px; box-shadow: none;
}}
button {{
    background-color: transparent; background-image: none;
    color: {t['text']}; border: none; border-radius: 12px;
    box-shadow: none; text-shadow: none; padding: 6px;
}}
button:hover {{ background-color: {hover}; }}
#tile {{ background-color: {field}; border-radius: 14px; padding: 10px 12px; }}
#tile:hover {{ background-color: {hover}; }}
#tile.on {{ background-color: {t['accent']}; }}
#tile.on label {{ color: {_hex(t['bg']) if t['dark'] else '#ffffff'}; }}
#power {{ background-color: {field}; border-radius: 12px; padding: 8px 10px; }}
#power:hover {{ background-color: {_rgba(_rgb_of(t['crit']), 0.35)}; }}
#notif {{ background-color: {field}; border-radius: 12px; padding: 10px 12px; }}
#accent {{ color: {t['accent']}; }}
scale trough {{ background-color: {field}; border-radius: 6px; min-height: 6px; }}
scale highlight {{ background-color: {t['accent']}; border-radius: 6px; }}
scale slider {{ background-color: {t['text']}; border-radius: 10px; min-width: 16px; min-height: 16px; }}
scrolledwindow, viewport {{ background-color: transparent; }}
"""


# ---- apply ---------------------------------------------------------------

def _write(path, content):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        f.write(content)
    os.replace(tmp, path)


def _quiet(argv):
    try:
        subprocess.run(argv, capture_output=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        pass


GTK_CSS_BEGIN = "/* nexa-theme: colour names for every GTK app (begin) */"
GTK_CSS_END = "/* nexa-theme (end) */"


def gtk_css_block(t, edge=None):
    """@nexa_* colour names for every GTK 3 app, including .nexa apps in C
    or Rust (see docs/WRITING_NEXA_APPS.md). Only names: no styles. `edge`
    is the glass edge light (an alpha), None when glass is off."""
    css = app_palette_css(t)
    if edge:
        css = css.replace("@define-color nexa_edge rgba(255, 255, 255, 0.00);", f"@define-color nexa_edge rgba(255, 255, 255, {edge:.2f});")
    return f"{GTK_CSS_BEGIN}\n{css}{GTK_CSS_END}\n"


def merge_gtk_css(existing, block):
    """Replace our marked block in ~/.config/gtk-3.0/gtk.css, keeping
    anything the user wrote there themselves."""
    existing = existing or ""
    start, end = existing.find(GTK_CSS_BEGIN), existing.find(GTK_CSS_END)
    if start != -1 and end > start:
        return existing[:start] + block + existing[end + len(GTK_CSS_END):].lstrip("\n")
    return block + ("\n" + existing if existing.strip() else "")


GSETTINGS_KEYFILE = os.path.join("glib-2.0", "settings", "keyfile")


def gsettings_keyfile(path, values):
    """Set keys of [org/gnome/desktop/interface] in GLib's keyfile settings
    store (GSETTINGS_BACKEND=keyfile, set for the whole session: there is no
    dconf). On Wayland GTK takes the icon theme from GSettings and ignores
    settings.ini for it; running apps follow a change to this file live.
    Everything else in the file is kept."""
    section = "[org/gnome/desktop/interface]"
    try:
        with open(path) as f:
            lines = f.read().splitlines()
    except OSError:
        lines = []
    out, in_section, seen, done = [], False, False, set()
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("["):
            if in_section:
                out += [f"{k}={v}" for k, v in values.items() if k not in done]
                done.update(values)
            in_section = stripped == section
            seen = seen or in_section
        elif in_section and "=" in stripped and stripped.split("=", 1)[0].strip() in values:
            key = stripped.split("=", 1)[0].strip()
            out.append(f"{key}={values[key]}")
            done.add(key)
            continue
        out.append(line)
    if in_section:
        out += [f"{k}={v}" for k, v in values.items() if k not in done]
    elif not seen:
        out += ([""] if out else []) + [section] + [f"{k}={v}" for k, v in values.items()]
    _write(path, "\n".join(out) + "\n")


def icon_styles_available(home=None):
    """True when the NexaHexa icons are installed (the icon style matters)."""
    return icon_theme_for(load_settings(home)["theme"], home) != "Adwaita"


def apply(name=None, opacity=None, home=None, reload=True, glass=None, icon_style=None, glass_tint=None):
    """Render and install the theme; returns the settings that were applied."""
    current = load_settings(home)
    name = name if name in THEMES else current["theme"]
    opacity = clamp_opacity(current["opacity"] if opacity is None else opacity)
    glass = current["glass"] if glass is None else bool(glass)
    icon_style = icon_style if icon_style in ICON_STYLES else current["icon_style"]
    glass_tint = glass_tint if glass_tint in GLASS_TINTS else current["glass_tint"]
    t = THEMES[name]
    cfg = _config_dir(home)
    _write(os.path.join(cfg, "waybar", "style.css"), waybar_css(t, opacity))
    _write(os.path.join(cfg, "mako", "config"), mako_config(t, opacity))
    _write(os.path.join(cfg, "foot", "foot.ini"), foot_ini(t, opacity))
    _write(os.path.join(cfg, "sway", "config.d", "20-nexa-theme.conf"), sway_conf(t))
    icon_theme = icon_theme_for(name, home, icon_style)
    _write(os.path.join(cfg, "gtk-3.0", "settings.ini"), gtk_settings(t, icon_theme))
    gsettings_keyfile(os.path.join(cfg, GSETTINGS_KEYFILE),
                      {"icon-theme": f"'{icon_theme}'", "color-scheme": "'prefer-dark'" if t["dark"] else "'default'"})
    css_path = os.path.join(cfg, "gtk-3.0", "gtk.css")
    try:
        with open(css_path) as f:
            existing = f.read()
    except OSError:
        existing = ""
    _write(css_path, merge_gtk_css(existing, gtk_css_block(t, EDGE_LIGHT[glass_tint] if glass else None)))
    _write(os.path.join(cfg, "hypr", "nexa-theme.conf"), hypr_conf(t))
    _write(os.path.join(cfg, "hypr", "nexa-glass.conf"), hypr_glass_conf(t, glass, glass_tint))
    _write(os.path.join(cfg, "swaylock", "config"), swaylock_conf(t))
    _write(os.path.join(cfg, "nexa", "lock.conf"), lock_conf(t))
    _write(os.path.join(_data_dir(home), "gtksourceview-4", "styles", "nexa.xml"), sourceview_scheme(t))
    _write(settings_path(home), json.dumps({"theme": name, "opacity": round(opacity, 2), "glass": glass,
                                            "icon_style": icon_style, "glass_tint": glass_tint}) + "\n")
    if reload:
        _quiet(["pkill", "-USR2", "-u", str(os.getuid()), "-x", "waybar"])   # reload style
        _quiet(["makoctl", "reload"])
        from . import compositor
        compositor.reload()
    return {"theme": name, "opacity": opacity, "glass": glass, "icon_style": icon_style, "glass_tint": glass_tint}


def main(argv):
    if len(argv) >= 1 and argv[0] == "list":
        for key, t in THEMES.items():
            print(f"{key:10} {t['label']}: {t['tagline']}")
        return 0
    if len(argv) >= 1 and argv[0] == "apply":
        args = argv[1:]
        reload = "--no-reload" not in args
        args = [a for a in args if a != "--no-reload"]
        opacity = None
        if "--opacity" in args:
            i = args.index("--opacity")
            opacity = args[i + 1] if i + 1 < len(args) else None
            del args[i:i + 2]
        name = None if (not args or args[0] == "--saved") else args[0]
        if name is not None and name not in THEMES:
            print(f"unknown theme '{name}' (try: {', '.join(THEMES)})", file=sys.stderr)
            return 2
        applied = apply(name, opacity, reload=reload)
        print(f"applied {applied['theme']} at {applied['opacity']:.2f} opacity")
        return 0
    print("usage: nexa-theme list | apply <name>|--saved [--opacity N] [--no-reload]", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
