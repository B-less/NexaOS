"""Shared window-chrome titlebar (minimize/maximize/close) for every Nexa GTK app."""
import os
import subprocess

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk

CSS = b"""
headerbar.nexa-titlebar { background-image: none; background-color: #252525; color: #e0e0e0; border: none; box-shadow: none; }
headerbar.nexa-titlebar .title { color: #e0e0e0; font-weight: 600; }
headerbar.nexa-titlebar button { background-image: none; background-color: transparent; border: none; box-shadow: none; padding: 4px 10px; }
headerbar.nexa-titlebar button:hover { background-image: none; background-color: #3a3a3a; }
headerbar.nexa-titlebar button.nexa-titlebar-close:hover { background-image: none; background-color: #f7768e; }
"""

_provider = None


def _ensure_css():
    global _provider
    if _provider is not None:
        return
    _provider = Gtk.CssProvider()
    _provider.load_from_data(CSS)
    Gtk.StyleContext.add_provider_for_screen(
        Gdk.Screen.get_default(), _provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
    )


def _icon_button(icon_name, tooltip, style_class=None):
    btn = Gtk.Button()
    btn.set_image(Gtk.Image.new_from_icon_name(icon_name, Gtk.IconSize.MENU))
    btn.set_relief(Gtk.ReliefStyle.NONE)
    btn.set_tooltip_text(tooltip)
    btn.set_focus_on_click(False)
    if style_class:
        btn.get_style_context().add_class(style_class)
    return btn


# Sway criteria for this app's own window(s). Plain `swaymsg move ...` acts
# on whatever is focused, which by the time a click is handled may be
# another window entirely.
_SELF = f"[pid={os.getpid()}]"

# Largest share of the screen a window may open at; bigger defaults (e.g.
# 800x900 on a 768-pixel-tall screen) would otherwise open cut off.
MAX_SCREEN_FRACTION = 0.92


def _sway(*commands):
    try:
        return subprocess.run(["swaymsg", f"{_SELF} " + ", ".join(commands)],
                              capture_output=True, timeout=3).returncode == 0
    except (OSError, subprocess.SubprocessError):
        return False


def _minimize(window):
    # Sway (like most wlroots compositors) does not act on xdg_toplevel's
    # set_minimized request -- there's no taskbar to restore from, so
    # window.iconify() would silently do nothing. Sway's scratchpad *is*
    # a real hide/restore mechanism, so minimize is wired to that instead
    # (restore via the waybar sway/scratchpad module or $mod+minus).
    if not _sway("move scratchpad"):
        window.iconify()


def _toggle_maximize(window):
    # Every Nexa window is floating, and sway ignores maximize requests from
    # floating windows, so GTK's maximize() did nothing. Instead, resize the
    # window to the whole workspace (the area under the top bar) and put it
    # back at its old size on the second click.
    saved = getattr(window, "_nexa_restore_size", None)
    if saved is None:
        window._nexa_restore_size = window.get_size()
        if not _sway("resize set width 100 ppt height 100 ppt", "move position 0 0"):
            window._nexa_restore_size = None
            window.maximize()
    else:
        window._nexa_restore_size = None
        w, h = saved
        if not _sway(f"resize set width {w} px height {h} px", "move position center"):
            window.unmaximize()


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


def attach_titlebar(window, title):
    """Give `window` a custom titlebar with minimize/maximize/close buttons."""
    _ensure_css()

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
    _fit_to_screen(window)
    window.connect("realize", _fit_to_screen)
    return bar
