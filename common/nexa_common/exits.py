"""How apps started from the desktop ended, for Why (Nexa MicroBrain,
tools/nexa-microbrain/src/why.rs). Only abnormal ends are written down: a
signal (a crash, or something ending the app) or an error code. Nothing
about what the app did; the last 200 at most.

    ~/.local/state/nexa/exits.jsonl   {"at", "app", "pid", "signal" | "code", "ran_s"}

The desktop (Nexus and the panel; Orbit, in C, writes the same file) is
the parent of the apps it starts, so it can see how they ended: GLib's
child watch, no extra process."""
import json
import os
import time

KEEP = 200
_watching = {}


def path():
    base = os.environ.get("XDG_STATE_HOME") or os.path.expanduser("~/.local/state")
    return os.path.join(base, "nexa", "exits.jsonl")


def record(app, pid, status, ran_s, now=None):
    """Write down one ended app if it ended abnormally (status: a wait status)."""
    if os.WIFSIGNALED(status):
        entry = {"signal": os.WTERMSIG(status)}
    elif os.WIFEXITED(status) and os.WEXITSTATUS(status) != 0:
        entry = {"code": os.WEXITSTATUS(status)}
    else:
        return False
    entry.update({"at": int(now or time.time()), "app": str(app)[:64], "pid": int(pid), "ran_s": round(float(ran_s), 1)})
    p = path()
    try:
        os.makedirs(os.path.dirname(p), mode=0o700, exist_ok=True)
        with open(p, "a") as f:
            f.write(json.dumps(entry, sort_keys=True) + "\n")
        if os.path.getsize(p) > KEEP * 120:  # about 1.5 x KEEP lines: trim back to KEEP
            with open(p) as f:
                lines = f.readlines()[-KEEP:]
            tmp = p + ".tmp"
            with open(tmp, "w") as f:
                f.writelines(lines)
            os.replace(tmp, p)
    except OSError:
        return False
    return True


def note_ended(app, pid, sig, by="Pulse"):
    """The person ended this process themselves (Pulse's End task / Force
    quit): written down so Why can say so instead of guessing."""
    entry = {"at": int(time.time()), "app": str(app)[:64], "pid": int(pid), "signal": int(sig), "by": by, "ran_s": 0.0}
    p = path()
    try:
        os.makedirs(os.path.dirname(p), mode=0o700, exist_ok=True)
        with open(p, "a") as f:
            f.write(json.dumps(entry, sort_keys=True) + "\n")
    except OSError:
        return False
    return True


def watch_pid(pid, app):
    """Note how this child ends (needs a GLib main loop; without one, nothing)."""
    try:
        from gi.repository import GLib
    except ImportError:
        return
    started = time.monotonic()

    def done(pid, status, *_a):
        popen = _watching.pop(pid, None)
        if popen is not None:  # GLib reaped it: tell the Popen how it ended
            popen.returncode = os.waitstatus_to_exitcode(status)
        record(app, pid, status, time.monotonic() - started)
        GLib.spawn_close_pid(pid)

    GLib.child_watch_add(GLib.PRIORITY_DEFAULT_IDLE, pid, done)


def watch(popen, app=None):
    """A subprocess.Popen the desktop started: keep it (so Python doesn't
    reap it behind GLib's back) and note how it ends."""
    app = app or os.path.basename(popen.args[0] if isinstance(popen.args, (list, tuple)) else str(popen.args).split()[0])
    _watching[popen.pid] = popen
    watch_pid(popen.pid, app)
    return popen
