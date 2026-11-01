"""Client for nexa-pdfdoc, the PDF Viewer's engine (C, tools-c/nexa-pdfdoc):
details and contents, search, page thumbnails and page text, each in its
own confined process. A password goes on its standard input."""
import json
import os
import shutil
import subprocess

REPO_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "build", "native", "nexa-pdfdoc")


class EngineError(Exception):
    def __init__(self, message, code=""):
        super().__init__(message)
        self.code = code


def binary():
    for c in (os.environ.get("NEXA_PDFDOC"), "/usr/local/bin/nexa-pdfdoc", REPO_BUILD):
        if c and os.access(c, os.X_OK):
            return c
    return shutil.which("nexa-pdfdoc") or "nexa-pdfdoc"


def run(args, password=None, on_line=None, started=None, timeout=300):
    """Run one command; on_line(dict) gets the progress lines. Returns the result."""
    argv = [binary()] + (["--password-stdin"] if password else []) + list(args)
    try:
        proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    except OSError as e:
        raise EngineError(f"The PDF engine (nexa-pdfdoc) is missing: {e}")
    if started:
        started(proc)
    proc.stdin.write((password + "\n") if password else "")
    proc.stdin.close()
    reply = None
    for line in proc.stdout:
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if "ok" in msg:
            reply = msg
        elif on_line:
            on_line(msg)
    proc.wait()
    if reply is None:
        raise EngineError("Stopped." if proc.returncode and proc.returncode < 0 else "The PDF engine stopped on this document.", "crashed")
    if not reply.get("ok"):
        raise EngineError(reply.get("error") or "That didn't work.", reply.get("code") or "")
    return reply["result"]


def info(path, password=None):
    return run(["info", path], password)


def search(path, text, case=False, words=False, password=None, on_found=None, started=None):
    args = ["search", path] + (["--case"] if case else []) + (["--words"] if words else []) + ["--", text]
    return run(args, password, on_line=lambda m: on_found and "found" in m and on_found(m["found"]), started=started)


def thumbs(path, outdir, width, pages, password=None, on_thumb=None):
    return run(["thumbs", path, outdir, str(width)] + [str(p) for p in pages], password,
               on_line=lambda m: on_thumb and "thumb" in m and on_thumb(m["thumb"]))


def text(path, page, password=None):
    return run(["text", path, str(page)], password)["text"]
