"""The few window-manager actions Nexa apps need, for both sessions NexaOS
runs: Hyprland (GPU machines: animations, blur) and Sway (fallback).

Every function quietly does nothing outside a session, so apps still run
anywhere; callers fall back to plain GTK where that makes sense.
"""
import json
import os
import subprocess

MINIMIZED = "special:minimized"   # Hyprland's stand-in for sway's scratchpad

_run = subprocess.run


def which():
    if os.environ.get("HYPRLAND_INSTANCE_SIGNATURE"):
        return "hyprland"
    if os.environ.get("SWAYSOCK"):
        return "sway"
    return None


def _call(argv, timeout=3):
    try:
        r = _run(argv, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.SubprocessError):
        return None
    return r.stdout if r.returncode == 0 else None


def _hypr(*dispatch):
    out = _call(["hyprctl", "dispatch", *dispatch])
    return out is not None and "ok" in out.lower()


def _sway(command):
    return _call(["swaymsg", command]) is not None


# ---- windows -------------------------------------------------------------

def windows():
    """[{app_id, pid, minimized}] for every app window."""
    wm = which()
    if wm == "hyprland":
        try:
            clients = json.loads(_call(["hyprctl", "clients", "-j"]) or "[]")
        except ValueError:
            return []
        return [{"app_id": (c.get("class") or "").lower(), "pid": c.get("pid"),
                 "minimized": (c.get("workspace") or {}).get("name") == MINIMIZED}
                for c in clients if c.get("pid")]
    if wm == "sway":
        try:
            tree = json.loads(_call(["swaymsg", "-t", "get_tree"]) or "{}")
        except ValueError:
            return []
        out, stack = [], [(tree, False)]
        while stack:
            node, in_scratch = stack.pop()
            in_scratch = in_scratch or node.get("name") == "__i3_scratch"
            app_id = node.get("app_id") or (node.get("window_properties") or {}).get("class")
            if app_id and node.get("pid"):
                out.append({"app_id": app_id.lower(), "pid": node["pid"], "minimized": in_scratch})
            stack.extend((n, in_scratch) for n in node.get("nodes", []) + node.get("floating_nodes", []))
        return out
    return []


def window_rects():
    """[{app_id, rect: [x, y, w, h]}] for the windows on screen (not
    minimised), for saving a workspace."""
    wm = which()
    out = []
    if wm == "hyprland":
        try:
            clients = json.loads(_call(["hyprctl", "clients", "-j"]) or "[]")
        except ValueError:
            return []
        for c in clients:
            if c.get("pid") and (c.get("workspace") or {}).get("name") != MINIMIZED:
                at, size = c.get("at") or [0, 0], c.get("size") or [0, 0]
                out.append({"app_id": (c.get("class") or "").lower(), "rect": [at[0], at[1], size[0], size[1]]})
    elif wm == "sway":
        try:
            tree = json.loads(_call(["swaymsg", "-t", "get_tree"]) or "{}")
        except ValueError:
            return []
        stack = [(tree, False)]
        while stack:
            node, in_scratch = stack.pop()
            in_scratch = in_scratch or node.get("name") == "__i3_scratch"
            app_id = node.get("app_id") or (node.get("window_properties") or {}).get("class")
            if app_id and node.get("pid") and not in_scratch:
                r = node.get("rect") or {}
                out.append({"app_id": app_id.lower(), "rect": [r.get("x", 0), r.get("y", 0), r.get("width", 0), r.get("height", 0)]})
            stack.extend((n, in_scratch) for n in node.get("nodes", []) + node.get("floating_nodes", []))
    return out


def place_window(app_id, rect):
    """Best effort: put a floating window of `app_id` at rect [x, y, w, h]."""
    x, y, w, h = (int(v) for v in rect)
    wm = which()
    if wm == "hyprland":
        sel = f"class:^({app_id})$"
        return _hypr("resizewindowpixel", f"exact {w} {h},{sel}") and _hypr("movewindowpixel", f"exact {x} {y},{sel}")
    if wm == "sway":
        return _sway(f'[app_id="^{app_id}$"] resize set width {w} px height {h} px, move absolute position {x} {y}')
    return False


def minimize(pid):
    wm = which()
    if wm == "hyprland":
        return _hypr("movetoworkspacesilent", f"{MINIMIZED},pid:{pid}")
    if wm == "sway":
        return _sway(f"[pid={pid}] move scratchpad")
    return False


def toggle_maximize(pid, restore_size=None):
    """Hyprland: maximize mode (keeps the bar). Sway: fill the workspace, or
    go back to restore_size (w, h) when given."""
    wm = which()
    if wm == "hyprland":
        return _hypr("focuswindow", f"pid:{pid}") and _hypr("fullscreen", "1")
    if wm == "sway":
        if restore_size:
            w, h = restore_size
            return _sway(f"[pid={pid}] resize set width {w} px height {h} px, move position center")
        return _sway(f"[pid={pid}] resize set width 100 ppt height 100 ppt, move position 0 0")
    return False


def usable_area():
    """(x, y, width, height) of the screen in front, minus what the bar and
    the Dock keep for themselves; None if the compositor can't say. GTK on
    Wayland only knows the whole screen, so tall windows opened with their
    title bar under the top bar."""
    wm = which()
    try:
        if wm == "hyprland":
            mons = json.loads(_call(["hyprctl", "monitors", "-j"]) or "[]")
            m = next((m for m in mons if m.get("focused")), mons[0] if mons else None)
            if not m:
                return None
            scale = float(m.get("scale") or 1) or 1
            left, top, right, bottom = (list(m.get("reserved") or []) + [0, 0, 0, 0])[:4]
            w, h = m["width"] / scale, m["height"] / scale
            # Rotated screens report width and height the panel way round.
            if int(m.get("transform", 0)) % 2:
                w, h = h, w
            return (int(m["x"] + left), int(m["y"] + top), int(w - left - right), int(h - top - bottom))
        if wm == "sway":
            spaces = json.loads(_call(["swaymsg", "-t", "get_workspaces"]) or "[]")
            ws = next((w for w in spaces if w.get("focused")), None)
            if ws:
                r = ws["rect"]
                return (r["x"], r["y"], r["width"], r["height"])
    except (ValueError, KeyError, TypeError, IndexError, ZeroDivisionError):
        return None
    return None


def center_in_view(pid):
    """Hyprland centres new windows on the whole screen, title bar under the
    top bar if the window is tall: centre it in the free area instead.
    (Sway already centres floating windows there.)"""
    if which() != "hyprland":
        return True
    try:
        clients = json.loads(_call(["hyprctl", "clients", "-j"]) or "[]")
    except ValueError:
        return False
    mine = [c for c in clients if c.get("pid") == pid and c.get("floating")]
    # The new window is the one in front; with one window there's no doubt.
    target = next((c for c in mine if c.get("focusHistoryID") == 0), mine[0] if len(mine) == 1 else None)
    if not target:
        return False
    return _hypr("focuswindow", f"address:{target['address']}") and _hypr("centerwindow", "1")


def focus_app(app_id):
    """Bring an app's window forward, restoring it if minimised."""
    wm = which()
    if wm == "hyprland":
        mine = [w for w in windows() if w["app_id"] == app_id.lower()]
        if not mine:
            return False
        pid = mine[0]["pid"]
        if mine[0]["minimized"]:
            # Silently: a plain movetoworkspace moved the focus (and the
            # pointer) on its own, before the window was raised.
            active = json.loads(_call(["hyprctl", "activeworkspace", "-j"]) or "{}").get("id", 1)
            _hypr("movetoworkspacesilent", f"{active},pid:{pid}")
        ok = _hypr("focuswindow", f"pid:{pid}")
        # Every window floats; focusing one doesn't raise it, so a restored
        # window came back *behind* the one in front, under the pointer.
        _hypr("alterzorder", f"top,pid:{pid}")
        return ok
    if wm == "sway":
        sel = f'[app_id="{app_id}"]'
        mine = [w for w in windows() if w["app_id"] == app_id.lower()]
        if any(w["minimized"] for w in mine):
            # Out of the scratchpad: shown, focused and on top.
            return _sway(f"{sel} scratchpad show") or _sway(f"{sel} focus")
        return _sway(f"{sel} focus")
    return False


# ---- screen --------------------------------------------------------------

def screen_power(on):
    wm = which()
    if wm == "hyprland":
        return _hypr("dpms", "on" if on else "off")
    if wm == "sway":
        return _sway(f"output * power {'on' if on else 'off'}")
    return False


def set_wallpaper(path):
    wm = which()
    if wm == "sway":
        return _sway(f"output * bg {path} fill")
    if wm == "hyprland":
        # swaybg draws the wallpaper here; restart it on the new image.
        _call(["pkill", "-u", str(os.getuid()), "-x", "swaybg"])
        subprocess.Popen(["swaybg", "-i", path, "-m", "fill"], start_new_session=True,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return True
    return False


def set_keyboard_layout(code):
    wm = which()
    if wm == "hyprland":
        return _call(["hyprctl", "keyword", "input:kb_layout", code]) is not None
    if wm == "sway":
        return _sway(f"input type:keyboard xkb_layout {code}")
    return False


def reload():
    wm = which()
    if wm == "hyprland":
        ok = _call(["hyprctl", "reload"]) is not None
    elif wm == "sway":
        ok = _sway("reload")
    else:
        return False
    # A reload resets the screens to the compositor's config: put back what
    # Settings > Display chose (nexa-display; nothing saved, nothing done).
    _call(["nexa-display", "apply-saved"], timeout=10)
    return ok


LOGGED_OUT = "nexa-logged-out"


def logout():
    """End the desktop session. The session supervisor (the "sway" service)
    starts a new one, which opens locked: the marker left here tells
    nexa-login-lock to lock even where the boot doesn't ask for a password."""
    try:
        run_dir = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
        with open(os.path.join(run_dir, LOGGED_OUT), "w"):
            pass
    except OSError:
        pass
    # The compositor quits before it can answer, so "no reply" is normal
    # here: success means the command could be sent at all.
    wm = which()
    argv = {"hyprland": ["hyprctl", "dispatch", "exit"], "sway": ["swaymsg", "exit"]}.get(wm)
    if not argv:
        return False
    try:
        _run(argv, capture_output=True, text=True, timeout=3)
    except OSError:
        return False
    except subprocess.SubprocessError:
        pass
    return True
