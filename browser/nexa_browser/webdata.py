"""Client for nexa-webdata (Rust, tools/nexa-webdata): bookmark import and
export, reader view, and the encrypted password vault. Requests go on its
standard input (a primary password too, never the command line)."""
import json
import os
import shutil
import subprocess

REPO_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "build", "native", "nexa-webdata")


class EngineError(Exception):
    def __init__(self, message, code=""):
        super().__init__(message)
        self.code = code


def binary():
    for c in (os.environ.get("NEXA_WEBDATA"), "/usr/local/bin/nexa-webdata", REPO_BUILD):
        if c and os.access(c, os.X_OK):
            return c
    return shutil.which("nexa-webdata") or "nexa-webdata"


def run(op, **args):
    args = {k: v for k, v in args.items() if v is not None}
    try:
        proc = subprocess.run([binary()], input=json.dumps(dict(op=op, **args)), capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise EngineError(f"The browser's data engine (nexa-webdata) isn't working: {e}")
    reply = None
    for line in proc.stdout.splitlines():
        try:
            reply = json.loads(line)
        except ValueError:
            continue
    if reply is None:
        raise EngineError("The browser's data engine stopped unexpectedly.")
    if not reply.get("ok"):
        raise EngineError(reply.get("error") or "That didn't work.", reply.get("code") or "")
    return reply["result"]
