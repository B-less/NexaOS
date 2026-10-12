"""Shared window-chrome titlebar (minimize/maximize/close) for every Nexa GTK app."""
import os
import sys

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gio, GLib, Gtk, Gdk

from . import compositor, themes

CSS = b"""
headerbar.nexa-titlebar { background-image: none; background-color: @nexa_titlebar; color: @nexa_text; border: none; box-shadow: none; }
headerbar.nexa-titlebar .title { color: @nexa_text_strong; font-weight: 600; }
headerbar.nexa-titlebar button { background-image: none; background-color: transparent; border: none; box-shadow: none; padding: 4px 10px; }
headerbar.nexa-titlebar button:hover { background-image: none; background-color: @nexa_hover; }
headerbar.nexa-titlebar button.nexa-titlebar-close:hover { background-image: none; background-color: @nexa_crit; }
"""

_provider = None
_palette = None
_monitor = None


def _load_palette(*_args):
    """(Re)define the @nexa_* colours from the saved theme. Called again when
    the theme changes, which restyles every open window of this app."""
    settings = themes.load_settings()
    glass = glass_alpha() if settings["glass"] else None
    _palette.load_from_data(themes.app_palette_css(themes.THEMES[settings["theme"]], glass).encode())


def glass_alpha():
    """How solid glass windows are: Hyprland blurs what is behind them, so
    they can be clearer; Sway can't blur, so they stay nearly solid there."""
    return 0.84 if compositor.which() == "hyprland" else 0.94


def _ensure_css():
    global _provider, _palette, _monitor
    if _provider is not None:
        return
    screen = Gdk.Screen.get_default()
    # The palette goes in first and at a lower priority than the apps' own
    # stylesheets, which only refer to its colours by name.
    _palette = Gtk.CssProvider()
    _load_palette()
    Gtk.StyleContext.add_provider_for_screen(screen, _palette, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION - 1)
    _provider = Gtk.CssProvider()
    _provider.load_from_data(CSS)
    Gtk.StyleContext.add_provider_for_screen(screen, _provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
    # Each toplevel paints the window colour once. One level above the apps'
    # own stylesheets: GTK only weighs selector specificity inside a single
    # stylesheet, so their plain `window { ... }` rules would otherwise win.
    glass = Gtk.CssProvider()
    glass.load_from_data(b"""
window.background { background-color: @nexa_glass; }
/* micro-animations: buttons, rows and tabs ease between states */
button, row, notebook tab, entry, switch, scale slider {
    transition: background-color 160ms ease-out, color 160ms ease-out,
                box-shadow 200ms ease-out, border-color 160ms ease-out;
}
button:active { box-shadow: inset 0 1px 3px alpha(black, 0.25); }
""")
    Gtk.StyleContext.add_provider_for_screen(screen, glass, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 1)
    # Follow theme changes made in Settings while the app is open.
    _monitor = Gio.File.new_for_path(themes.settings_path()).monitor_file(Gio.FileMonitorFlags.NONE, None)
    _monitor.connect("changed", _load_palette)


def _icon_button(icon_name, tooltip, style_class=None):
    btn = Gtk.Button()
    btn.set_image(Gtk.Image.new_from_icon_name(icon_name, Gtk.IconSize.MENU))
    btn.set_relief(Gtk.ReliefStyle.NONE)
    btn.set_tooltip_text(tooltip)
    btn.set_focus_on_click(False)
    if style_class:
        btn.get_style_context().add_class(style_class)
    return btn


# Largest share of the screen a window may open at; bigger defaults (e.g.
# 800x900 on a 768-pixel-tall screen) would otherwise open cut off.
MAX_SCREEN_FRACTION = 0.92


def _minimize(window):
    # Wayland compositors ignore xdg_toplevel's set_minimized, so this hides
    # the app's own window (by pid) in sway's scratchpad or Hyprland's
    # special "minimized" workspace; the Dock brings it back.
    if not compositor.minimize(os.getpid()):
        window.iconify()


def _toggle_maximize(window):
    # Every Nexa window floats, and sway ignores maximize requests from
    # floating windows, so GTK's maximize() did nothing there. On sway the
    # window is resized to the whole workspace and put back on the second
    # click; Hyprland has a real maximize mode that toggles.
    if compositor.which() == "hyprland":
        if not compositor.toggle_maximize(os.getpid()):
            window.unmaximize() if window.is_maximized() else window.maximize()
        return
    saved = getattr(window, "_nexa_restore_size", None)
    window._nexa_restore_size = None if saved else window.get_size()
    if not compositor.toggle_maximize(os.getpid(), restore_size=saved):
        window._nexa_restore_size = None
        window.unmaximize() if saved else window.maximize()


def _fit_to_screen(window):
    """Shrink a default size that wouldn't fit on this screen."""
    display = Gdk.Display.get_default()
    if display is None or display.get_n_monitors() == 0:
        return
    gdk_win = window.get_window()
    monitor = display.get_monitor_at_window(gdk_win) if gdk_win else display.get_monitor(0)
    area = monitor.get_workarea()
    w, h = window.get_default_size()
    max_w, max_h = int(area.width * MAX_SCREEN_FRACTION), int(area.height * MAX_SCREEN_FRACTION)
    if w > max_w or h > max_h:
        window.set_default_size(min(w, max_w) if w > 0 else w, min(h, max_h) if h > 0 else h)
        window.resize(min(w, max_w) if w > 0 else max_w, min(h, max_h) if h > 0 else max_h)


def _set_app_id():
    """Report a Wayland app_id matching the app's .desktop file
    (python3 -m nexa_files -> "nexa-files"), so the Dock can tell which
    apps are running; otherwise every Nexa app shows up as "python3"."""
    spec = getattr(sys.modules.get("__main__"), "__spec__", None)
    if spec and spec.name.startswith("nexa_"):
        GLib.set_prgname(spec.name.split(".")[0].replace("_", "-"))


def attach_titlebar(window, title):
    """Give `window` a custom titlebar with minimize/maximize/close buttons."""
    _ensure_css()
    if not window.get_realized():
        _set_app_id()  # read by GTK when the window is first shown

    bar = Gtk.HeaderBar()
    bar.set_show_close_button(False)
    bar.set_title(title)
    bar.get_style_context().add_class("nexa-titlebar")

    min_btn = _icon_button("window-minimize-symbolic", "Minimize")
    min_btn.connect("clicked", lambda *_a: _minimize(window))

    max_btn = _icon_button("window-maximize-symbolic", "Maximize")
    max_btn.connect("clicked", lambda *_a: _toggle_maximize(window))

    close_btn = _icon_button("window-close-symbolic", "Close", "nexa-titlebar-close")
    close_btn.connect("clicked", lambda *_a: window.close())

    bar.pack_end(close_btn)
    bar.pack_end(max_btn)
    bar.pack_end(min_btn)

    window.set_titlebar(bar)
    # An RGBA visual lets the compositor show (and blur) what is behind the
    # window where the theme's glass colours are see-through.
    screen = window.get_screen()
    visual = screen.get_rgba_visual()
    if visual is not None and screen.is_composited():
        window.set_visual(visual)
    _fit_to_screen(window)
    window.connect("realize", _fit_to_screen)
    return bar
