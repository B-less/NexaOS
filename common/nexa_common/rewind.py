"""Rewind (Magic Layer) from the desktop: the timeline of settings changes
that Nexa MicroBrain keeps (tools/nexa-microbrain/src/rewind.rs), and going
back. MicroBrain puts the files back; this side then makes the running
desktop follow them, from a fixed list (nothing here runs a command that
came from the history)."""
import os
import subprocess

from . import brain


def moments(limit=50):
    return brain.request("rewind", limit=limit).get("moments", [])


def _restart(name, argv):
    subprocess.run(["pkill", "-u", str(os.getuid()), "-x", name], check=False)
    try:
        subprocess.Popen(argv, start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError:
        pass


def reapply(kind):
    """Make the desktop follow a settings file that was put back."""
    if kind == "theme":
        from . import themes
        themes.apply()
    elif kind == "magic":
        from . import magic
        magic.notify()
    elif kind == "topbar":
        _restart("waybar", ["nexa-bar"])
    elif kind == "idle":
        _restart("swayidle", ["nexa-idle"])
    elif kind == "keyboard":
        from . import compositor, sysconf
        code = sysconf.read_keyboard_layout()
        if code:
            compositor.set_keyboard_layout(code)


def go_back(moment_id):
    """Everything changed at or after this moment back to how it was just
    before it. Returns the labels of what was put back."""
    reply = brain.request("rewind-to", id=int(moment_id))
    done = reply.get("restored", [])
    for kind in dict.fromkeys(r.get("apply", "none") for r in done):
        try:
            reapply(kind)
        except Exception:  # a running part that can't follow now picks it up at its next start
            pass
    return [r.get("label", "") for r in done]
