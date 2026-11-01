"""Client for nexa-musiclib, the Music app's engine (Rust, tools/nexa-musiclib):
the library (tags, lengths, covers, kept in an index) and playlists."""
import json
import os
import shutil
import subprocess

REPO_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "build", "native", "nexa-musiclib")


class EngineError(Exception):
    pass


def binary():
    for c in (os.environ.get("NEXA_MUSICLIB"), "/usr/local/bin/nexa-musiclib", REPO_BUILD):
        if c and os.access(c, os.X_OK):
            return c
    return shutil.which("nexa-musiclib") or "nexa-musiclib"


def run(op, progress=None, **args):
    try:
        proc = subprocess.Popen([binary()], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    except OSError as e:
        raise EngineError(f"The music engine (nexa-musiclib) is missing: {e}")
    proc.stdin.write(json.dumps(dict(op=op, **args)))
    proc.stdin.close()
    reply = None
    for line in proc.stdout:
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if "progress" in msg:
            if progress:
                progress(msg["progress"])
        else:
            reply = msg
    proc.wait()
    if reply is None:
        raise EngineError("The music engine stopped unexpectedly.")
    if not reply.get("ok"):
        raise EngineError(reply.get("error") or "That didn't work.")
    return reply["result"]


def scan(dirs=None, progress=None):
    return run("scan", progress=progress, dirs=list(dirs or []))


def tags(paths):
    return run("tags", paths=list(paths))
