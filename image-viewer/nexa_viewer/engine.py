"""Clients for the engines the Image Viewer uses, all speaking one JSON line
(or tab-separated lines for nexa-thumb):

- nexa-imageinfo (Rust): a picture's details and lossless JPEG turning
- nexa-fileops (Rust, the Files engine): Trash and undo
- nexa-thumb (C): thumbnails for the strip"""
import json
import os
import shutil
import subprocess

REPO_NATIVE = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "build", "native")
PICTURES = {".png", ".jpg", ".jpeg", ".jpe", ".gif", ".bmp", ".webp", ".svg", ".tif", ".tiff", ".ico"}


class EngineError(Exception):
    def __init__(self, message, code=""):
        super().__init__(message)
        self.code = code


def find(name, env):
    for c in (os.environ.get(env), f"/usr/local/bin/{name}", os.path.join(REPO_NATIVE, name)):
        if c and os.access(c, os.X_OK):
            return c
    return shutil.which(name) or name


def _run(program, request, what):
    try:
        proc = subprocess.run([program], input=json.dumps(request), capture_output=True, text=True, timeout=120)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise EngineError(f"The {what} isn't working: {e}")
    reply = None
    for line in proc.stdout.splitlines():
        try:
            reply = json.loads(line)
        except ValueError:
            continue
    if reply is None:
        raise EngineError(f"The {what} stopped unexpectedly.")
    if not reply.get("ok"):
        raise EngineError(reply.get("error") or "That didn't work.", reply.get("code") or "")
    return reply["result"]


def info(path):
    return _run(find("nexa-imageinfo", "NEXA_IMAGEINFO"), {"op": "info", "path": path}, "picture engine")


def turn(path, how, expect=None):
    return _run(find("nexa-imageinfo", "NEXA_IMAGEINFO"), {"op": "turn", "path": path, "how": how, "expect": expect}, "picture engine")


def trash(path):
    """Move to the Trash; returns the undo steps."""
    return _run(find("nexa-fileops", "NEXA_FILEOPS"), {"op": "trash", "paths": [path]}, "file engine").get("undo") or []


def undo(steps):
    return _run(find("nexa-fileops", "NEXA_FILEOPS"), {"op": "undo", "steps": steps}, "file engine")


def thumbnails(paths, size=128):
    """{path: png} for the pictures nexa-thumb could do (synchronous)."""
    exe = find("nexa-thumb", "NEXA_THUMB")
    if not paths or not os.access(exe, os.X_OK):
        return {}
    try:
        out = subprocess.run([exe, "--size", str(size), "--"] + list(paths), capture_output=True, text=True, timeout=60).stdout
    except (OSError, subprocess.TimeoutExpired):
        return {}
    done = {}
    for line in out.splitlines():
        parts = line.split("\t")
        if parts[0] == "ok" and len(parts) >= 3:
            done[parts[1]] = parts[2]
    return done
