"""Thumbnails for the Files window, made by nexa-thumb (C, tools-c/nexa-thumb):
pictures, a frame of videos, the first page of PDFs, and folders showing
the pictures inside. The work runs in a background thread, one confined
nexa-thumb process per batch; results come back on the GTK main loop.

nexa-thumb prints "start" before each file, so if one file makes it hang or
crash, that file is marked failed (never tried again until it changes) and
the rest of the batch is retried."""
import os
import shutil
import subprocess
import threading

from gi.repository import GLib

REPO_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
                          "build", "native", "nexa-thumb")
PICTURES = {".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".svg", ".tif", ".tiff", ".ico", ".avif", ".heic"}
VIDEOS = {".mp4", ".mkv", ".webm", ".avi", ".mov", ".m4v", ".ogv", ".mpg", ".mpeg", ".wmv", ".3gp"}
DOCUMENTS = {".pdf"}
BATCH = 48
TIMEOUT = 60


def binary():
    for c in (os.environ.get("NEXA_THUMB"), "/usr/local/bin/nexa-thumb", REPO_BUILD):
        if c and os.access(c, os.X_OK):
            return c
    return shutil.which("nexa-thumb")


def wanted(path, is_dir):
    """Worth asking nexa-thumb about (folders always: they may hold pictures)."""
    if is_dir:
        return True
    return os.path.splitext(path)[1].lower() in PICTURES | VIDEOS | DOCUMENTS


def run(paths, size, accent="5b8cff", folder=False):
    """Thumbnails for paths, synchronously: {path: png}. Hangs and crashes
    are contained: the file that caused one is marked failed."""
    exe = binary()
    if not exe or not paths:
        return {}
    done, todo = {}, list(paths)
    while todo:
        argv = [exe, "--size", str(size), "--accent", accent] + (["--folder"] if folder else []) + ["--"] + todo
        current, finished = None, set()
        try:
            out = subprocess.run(argv, capture_output=True, text=True, timeout=TIMEOUT).stdout
        except subprocess.TimeoutExpired as e:
            out = e.stdout.decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        except OSError:
            return done
        for line in out.splitlines():
            parts = line.split("\t")
            if parts[0] == "start" and len(parts) >= 2:
                current = parts[1]
            elif parts[0] in ("ok", "none") and len(parts) >= 2:
                finished.add(parts[1])
                current = None
                if parts[0] == "ok" and len(parts) >= 3:
                    done[parts[1]] = parts[2]
        if current is None:
            break  # the whole batch went through
        # It died or hung on `current`: never try that file again, go on with the rest.
        if not folder:
            subprocess.run([exe, "--mark-failed", current], capture_output=True, timeout=10)
        todo = [p for p in todo if p not in finished and p != current]
    return done


class Thumbnailer:
    """Asks for thumbnails in the background; one request at a time per
    window, newer requests replace older ones still waiting."""

    def __init__(self):
        self._generation = 0
        self._lock = threading.Lock()

    def request(self, files, folders, size, accent, on_ready):
        """on_ready({path: png}) is called on the main loop, possibly several
        times (as batches finish), for the latest request only."""
        with self._lock:
            self._generation += 1
            gen = self._generation

        def work():
            for kind, items in ((False, files), (True, folders)):
                for i in range(0, len(items), BATCH):
                    if gen != self._generation:
                        return
                    found = run(items[i:i + BATCH], size, accent, folder=kind)
                    if found and gen == self._generation:
                        GLib.idle_add(lambda f=found: (gen == self._generation and on_ready(f)) and False)

        threading.Thread(target=work, daemon=True).start()

    def cancel(self):
        with self._lock:
            self._generation += 1
