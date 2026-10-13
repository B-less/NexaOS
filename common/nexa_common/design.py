"""Hexa, the NexaOS design language: the shared tokens every part of the
desktop draws from (type, shape, spacing, motion), plus the hexagon geometry
used for window buttons and other hex shapes. Colours live in themes.py.
See docs/ROADMAP.md, "Our own identity". No GTK here.
"""
import math

# Type: Inter for the interface, JetBrains Mono for code and the terminal
# (both SIL Open Font License, packaged as font-inter / font-jetbrains-mono).
FONT_UI = "Inter"
FONT_MONO = "JetBrains Mono"
FONT_UI_SIZE = 10          # points; GTK's base size
FONT_MONO_SIZE = 11
TYPE_SCALE = {"caption": 8.5, "body": 10, "label": 10, "title": 12, "heading": 15, "display": 22}

# Shape and spacing (pixels)
RADIUS = {"small": 6, "medium": 10, "large": 16}
SPACING = (4, 8, 12, 16, 24, 32)

# Motion: one curve and three durations everywhere; "reduce motion" sets
# every duration to 0 (Settings > Accessibility).
DURATION_MS = {"state": 160, "panel": 240, "page": 320}
EASE_OUT = "cubic-bezier(0.22, 1, 0.36, 1)"

# Window buttons: hexagons, pointy-top.
HEX_BUTTON_RADIUS = 7.0
HEX_BUTTON_SIZE = (30, 28)


def hex_points(cx, cy, r, pointy_top=True):
    """Corner points of a regular hexagon centred on (cx, cy)."""
    start = -90 if pointy_top else 0
    return [(cx + r * math.cos(math.radians(start + 60 * i)),
             cy + r * math.sin(math.radians(start + 60 * i))) for i in range(6)]


def global_css(reduce_motion=False):
    """GTK CSS every Nexa app loads (through titlebar.py): the typeface,
    and the shared motion for state changes."""
    ms = 0 if reduce_motion else DURATION_MS["state"]
    return f"""
* {{ font-family: "{FONT_UI}", sans-serif; }}
textview.monospace, .monospace {{ font-family: "{FONT_MONO}", monospace; }}
button, row, notebook tab, entry, switch, scale slider {{
    transition: background-color {ms}ms {EASE_OUT}, color {ms}ms {EASE_OUT},
                box-shadow {ms}ms {EASE_OUT}, border-color {ms}ms {EASE_OUT};
}}
button:active {{ box-shadow: inset 0 1px 3px alpha(black, 0.25); }}
/* Text fields follow the theme everywhere (GTK's default is a flat grey). */
entry {{ background-image: none; background-color: @nexa_field; color: @nexa_text_strong;
         border: 1px solid @nexa_border; border-radius: {RADIUS["small"]}px; }}
entry:focus {{ border-color: @nexa_accent; }}
entry image {{ color: @nexa_muted; }}
"""
