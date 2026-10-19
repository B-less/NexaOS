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


def focus_app(app_id):
    """Bring an app's window forward, restoring it if minimised."""
    wm = which()
    if wm == "hyprland":
        mine = [w for w in windows() if w["app_id"] == app_id.lower()]
        if not mine:
            return False
        pid = mine[0]["pid"]
        if mine[0]["minimized"]:
            active = json.loads(_call(["hyprctl", "activeworkspace", "-j"]) or "{}").get("id", 1)
            _hypr("movetoworkspace", f"{active},pid:{pid}")
        return _hypr("focuswindow", f"pid:{pid}")
    if wm == "sway":
        sel = f'[app_id="{app_id}"]'
        return _sway(f"{sel} focus") or _sway(f"{sel} scratchpad show")
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
        _call(["pkill", "-x", "swaybg"])
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
        return _call(["hyprctl", "reload"]) is not None
    if wm == "sway":
        return _sway("reload")
    return False


def logout():
    """End the desktop session (back to the login screen)."""
    wm = which()
    if wm == "hyprland":
        return _hypr("exit")
    if wm == "sway":
        return _sway("exit")
    return False
