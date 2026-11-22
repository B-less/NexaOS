"""Single-instance apps: opening a second file hands it to the running app
instead of starting another copy.

Each app's main() does:

    paths = [a for a in argv if not a.startswith("-")]
    inst = singleton.claim("nexa-music", paths)
    if inst is None:
        return                      # already running; the file was handed over
    ...make the window with `paths`...
    singleton.serve(inst, lambda ps: (win.open_paths(ps), win.present()))
    Gtk.main()

`claim` is pure sockets and runs before Gtk.init(): if the app is already
running it sends the files to it and returns None (exit); otherwise it binds
the app's socket and returns it (become the one instance). `serve` then, once
the window exists, delivers any later launch's files to `on_open(paths)` on
the GLib main loop.

The socket is a pathname under XDG_RUNTIME_DIR, so a nexa-run-confined app
(which is granted /run/user/<uid>) can reach it, and Landlock signal and
abstract-socket scoping don't apply to it.
"""
import atexit
import json
import os
import socket

# claim() returns this (truthy) when it couldn't set the socket up at all:
# run normally, just without single-instance, rather than fail to start.
ALONE = object()


def _addr(app_id):
    base = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    return os.path.join(base, f"{app_id}.instance")


def claim(app_id, paths):
    """None if an instance was already running (files handed to it; exit).
    Otherwise a listening socket (this process is the instance) or ALONE if
    the socket couldn't be set up (run without single-instance)."""
    addr = _addr(app_id)
    files = [os.path.abspath(p) for p in paths]
    # Already running? Hand the files over.
    try:
        c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        c.settimeout(2)
        c.connect(addr)
        c.sendall((json.dumps(files) + "\n").encode())
        c.close()
        return None
    except OSError:
        pass  # nobody listening (or a stale socket): become the instance
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        try:
            s.bind(addr)
        except OSError:
            os.unlink(addr)  # stale socket from a crashed instance
            s.bind(addr)
        s.listen(8)
        s.setblocking(False)
    except OSError:
        s.close()
        return ALONE
    atexit.register(_cleanup, addr)
    return s


def serve(sock, on_open):
    """Deliver each later launch's files to on_open(paths) on the GLib loop."""
    if not isinstance(sock, socket.socket):
        return  # ALONE: single-instance isn't active
    from gi.repository import GLib

    def ready(fd, _cond):
        try:
            conn, _ = sock.accept()
        except OSError:
            return True
        try:
            conn.settimeout(2)
            data = b""
            while not data.endswith(b"\n") and len(data) < 1 << 20:
                chunk = conn.recv(65536)
                if not chunk:
                    break
                data += chunk
        except OSError:
            data = b""
        finally:
            conn.close()
        try:
            paths = json.loads(data.decode() or "[]")
        except (ValueError, UnicodeDecodeError):
            paths = []
        if isinstance(paths, list) and paths:
            GLib.idle_add(_deliver, on_open, [str(p) for p in paths])
        return True  # keep watching

    GLib.io_add_watch(sock.fileno(), GLib.IO_IN, ready)


def _deliver(on_open, paths):
    on_open(paths)
    return False


def _cleanup(addr):
    try:
        os.unlink(addr)
    except OSError:
        pass
