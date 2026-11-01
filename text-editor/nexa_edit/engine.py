"""Client for nexa-textio, the Editor's engine (Rust, tools/nexa-textio).

Opening and saving files (encodings, line endings, safe saves, noticing
other programs' changes) and the drafts of unsaved changes all happen
there. This module only speaks its one-JSON-line protocol."""
import json
import os
import shutil
import subprocess

REPO_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
                          "build", "native", "nexa-textio")

ENCODINGS = {
    "utf-8": "UTF-8",
    "utf-8-bom": "UTF-8 with BOM",
    "utf-16le": "UTF-16 LE",
    "utf-16be": "UTF-16 BE",
    "windows-1252": "Windows-1252",
}
LINE_ENDINGS = {"lf": "LF", "crlf": "CRLF", "cr": "CR"}


class EngineError(Exception):
    def __init__(self, message, code=""):
        super().__init__(message)
        self.code = code


def binary():
    for candidate in (os.environ.get("NEXA_TEXTIO"), "/usr/local/bin/nexa-textio", REPO_BUILD):
        if candidate and os.access(candidate, os.X_OK):
            return candidate
    return shutil.which("nexa-textio") or "nexa-textio"


def run(op, **args):
    try:
        proc = subprocess.run([binary()], input=json.dumps(dict(op=op, **args)), capture_output=True, text=True)
    except OSError as e:
        raise EngineError(f"The Editor's file engine (nexa-textio) is missing: {e}")
    reply = None
    for line in proc.stdout.splitlines():
        try:
            reply = json.loads(line)
        except ValueError:
            continue
    if reply is None:
        raise EngineError("The Editor's file engine stopped unexpectedly.")
    if not reply.get("ok"):
        raise EngineError(reply.get("error") or "That didn't work.", reply.get("code") or "")
    return reply["result"]
