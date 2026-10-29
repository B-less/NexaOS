"""The Archive app's thin client for nexa-archiver (Rust, tools/nexa-archiver),
the same engine Files uses: it reads zip, tar and tar or single files
compressed with gzip, xz, bzip2 or zstd, never overwrites anything, keeps
every entry inside the folder it makes, and confines itself with Landlock.
(This app used to run `unzip -o` / `tar -xf`, which replaced files.)"""
import json
import os
import shutil
import subprocess

REPO_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
                          "build", "native", "nexa-archiver")
PATTERNS = ("*.zip", "*.tar", "*.tar.gz", "*.tgz", "*.tar.xz", "*.txz", "*.tar.bz2", "*.tbz2", "*.tar.zst",
            "*.tzst", "*.gz", "*.xz", "*.bz2", "*.zst")


class ArchiveError(Exception):
    pass


def binary():
    for candidate in (os.environ.get("NEXA_ARCHIVER"), "/usr/local/bin/nexa-archiver", REPO_BUILD):
        if candidate and os.access(candidate, os.X_OK):
            return candidate
    return shutil.which("nexa-archiver") or "nexa-archiver"


def _call(request, timeout=3600):
    try:
        proc = subprocess.run([binary()], input=json.dumps(request), capture_output=True, text=True, timeout=timeout)
    except FileNotFoundError:
        raise ArchiveError("The archive engine (nexa-archiver) is missing")
    except subprocess.TimeoutExpired:
        raise ArchiveError("The archive engine took too long")
    reply = None
    for line in proc.stdout.splitlines():
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if "progress" not in msg:
            reply = msg
    if reply is None:
        raise ArchiveError("The archive engine stopped unexpectedly")
    if not reply.get("ok"):
        raise ArchiveError(reply.get("error") or "That didn't work")
    return reply["result"]


def list_contents(path):
    """[{"name", "size", "is_dir"}] (the first 5000), plus how many in all."""
    result = _call({"op": "list", "archive": os.path.abspath(path)}, timeout=600)
    return [{"name": e["name"], "size": e["size"], "is_dir": e["dir"]} for e in result["entries"]], result["count"]


def extract_all(path, dest_dir):
    """Unpack into dest_dir: one item comes out as itself, several in a new
    folder named after the archive. Returns the engine's result (its "path"
    is what was made; "skipped" lists what was left out and why)."""
    return _call({"op": "extract", "archive": os.path.abspath(path), "dest_dir": os.path.abspath(dest_dir)})
