"""Shared window-chrome titlebar (minimize/maximize/close) for every Nexa GTK app."""
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


def _minimize(window):
    # Sway (like most wlroots compositors) does not act on xdg_toplevel's
    # set_minimized request -- there's no taskbar to restore from, so
    # window.iconify() would silently do nothing. Sway's scratchpad *is*
    # a real hide/restore mechanism, so minimize is wired to that instead
    # (restore via the waybar sway/scratchpad module or $mod+minus).
    try:
        subprocess.Popen(["swaymsg", "move", "scratchpad"])
    except FileNotFoundError:
        window.iconify()


def _toggle_maximize(window):
    if window.is_maximized():
        window.unmaximize()
    else:
        window.maximize()


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
    return bar
