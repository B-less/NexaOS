"""Nexa MicroBrain from the desktop: requests over its socket
($XDG_RUNTIME_DIR/nexa-microbrain.sock), and the actions it returns,
carried out here. The service is sandboxed and never starts programs
itself; this side only knows a fixed set of action types, so nothing
MicroBrain (or a stored workspace) says can become an arbitrary command."""
import json
import os
import socket
import subprocess

APPS_DIRS = ("~/.local/share/applications", "/usr/local/share/applications", "/usr/share/applications")


class BrainError(Exception):
    pass


def sock_path():
    run = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    return os.path.join(run, "nexa-microbrain.sock")


def request(cmd, timeout=5, **fields):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect(sock_path())
        s.sendall((json.dumps({"cmd": cmd, **fields}) + "\n").encode())
        data = b""
        while not data.endswith(b"\n"):
            chunk = s.recv(65536)
            if not chunk:
                break
            data += chunk
    except OSError:
        raise BrainError("Nexa MicroBrain isn't running.") from None
    finally:
        s.close()
    try:
        reply = json.loads(data.decode() or "{}")
    except ValueError:
        raise BrainError("MicroBrain gave an answer that makes no sense.") from None
    if reply.get("ok") is False:
        raise BrainError(reply.get("error") or "That didn't work.")
    return reply


def ask(query):
    # Nexus asks on every keystroke: never wait long.
    return request("ask", timeout=0.4, query=query).get("results", {"hits": [], "note": None})


def desktop_file(app_id):
    """The installed .desktop file for app_id, or None (never a made-up path)."""
    if not app_id or "/" in app_id or app_id.startswith("."):
        return None
    for d in APPS_DIRS:
        path = os.path.join(os.path.expanduser(d), app_id + ".desktop")
        if os.path.isfile(path):
            return path
    return None


def launch(app_id, paths=()):
    import gi
    gi.require_version("Gio", "2.0")
    from gi.repository import Gio
    path = desktop_file(app_id)
    if not path:
        raise BrainError(f"{app_id} isn't installed.")
    info = Gio.DesktopAppInfo.new_from_filename(path)
    files = [Gio.File.new_for_path(p) for p in paths if os.path.exists(p)]
    uris = [f.get_uri() for f in files] if files and info.supports_files() else []
    from gi.repository import GLib
    from . import exits
    # Started by us and not reaped by GLib, so Why can see how it ended.
    info.launch_uris_as_manager(uris, None, GLib.SpawnFlags.SEARCH_PATH | GLib.SpawnFlags.DO_NOT_REAP_CHILD,
                                None, None, lambda _info, pid, *_a: exits.watch_pid(pid, app_id), None)


def _spawn(argv):
    from . import exits
    try:
        exits.watch(subprocess.Popen(argv, start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
    except OSError as e:
        raise BrainError(f"Couldn't start {argv[0]}: {e}") from None


def restore_workspace(name):
    """Opens a workspace's apps; returns what couldn't come back."""
    plan = request("workspace-plan", name=name)
    problems = list(plan.get("problems", []))
    placed = []
    for item in plan.get("launches", []):
        try:
            launch(item["app"], item.get("paths", []))
            if item.get("rect"):
                placed.append((item["app"], item["rect"]))
        except BrainError as e:
            problems.append(str(e))
    if placed:
        import threading
        import time

        def place():
            from . import compositor
            time.sleep(2.5)  # give the windows a moment to appear
            for app_id, rect in placed:
                compositor.place_window(app_id, rect)
        threading.Thread(target=place, daemon=True).start()
    return problems


def save_workspace(name, folder=None):
    from . import compositor
    return request("workspace-save", name=name, folder=folder, windows=compositor.window_rects())


def run(action):
    """Carry out one action from ZeroSearch (or a skill). Returns text to
    show, or "" when the action speaks for itself."""
    kind = action.get("type")
    if kind == "open_path":
        path = action.get("path", "")
        if not os.path.exists(path):
            raise BrainError(f"{path} isn't there.")
        _spawn(["nexa-files", path])
    elif kind == "open_trash":
        _spawn(["nexa-files", "trash:///"])
    elif kind == "launch":
        launch(action.get("app", ""), action.get("paths", []))
    elif kind == "settings":
        page = str(action.get("page", ""))
        _spawn(["nexa-settings"] + ([page] if page.isidentifier() else []))
    elif kind == "brightness":
        if "value" in action:
            arg = f"{max(1, min(100, int(action['value'])))}%"
        else:
            d = int(action.get("delta", 0))
            arg = f"{abs(d)}%{'+' if d > 0 else '-'}"
        _spawn(["brightnessctl", "set", arg])
    elif kind == "volume":
        if "mute" in action:
            _spawn(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "1" if action["mute"] else "0"])
        elif "value" in action:
            _spawn(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", f"{max(0, min(100, int(action['value'])))}%"])
        else:
            d = int(action.get("delta", 0))
            _spawn(["wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", f"{abs(d)}%{'+' if d > 0 else '-'}"])
    elif kind == "lock":
        _spawn(["nexa-lock"])
    elif kind == "power":
        what = action.get("what")
        if what not in ("suspend", "reboot", "poweroff"):
            raise BrainError("Not a power action.")
        from . import broker
        if what == "suspend":
            _spawn(["sh", "-c", "nexa-lock; exec nexa-brokerd call suspend"])
        else:
            broker.request(what, timeout=20)
    elif kind == "mode":
        r = request("mode", mode=action.get("mode", ""))
        return r.get("budget", {}).get("why", "")
    elif kind == "workspace":
        problems = restore_workspace(action.get("name", ""))
        return ("Couldn't restore everything: " + "; ".join(problems)) if problems else ""
    elif kind == "answer":
        return request("answer", topic=action.get("topic", "")).get("text", "")
    elif kind == "layer":  # Strata
        if "to" in action:
            n = int(action["to"])
            if not 1 <= n <= 5:
                raise BrainError("There are five layers: 1 to 5.")
            _spawn(["nexa-strata", "go", str(n)])
        else:
            _spawn(["nexa-strata", "dive", "+1" if int(action.get("step", 1)) > 0 else "-1"])
    elif kind == "quiet":  # Do not disturb (Nexa Signals)
        _spawn(["nexa-signalctl", "mode", "-a" if action.get("on") else "-r", "do-not-disturb"])
        return "Do not disturb is on: notifications wait in Deck's Signals card." if action.get("on") else "Do not disturb is off."
    else:
        raise BrainError("NexaOS doesn't know how to do that.")
    return ""
