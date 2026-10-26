"""Client for nexa-fileops, the Files app's engine (Rust, tools/nexa-fileops).

Every change to files - copy, move, rename, Trash, restore, delete, undo -
is one run of that program: it never overwrites anything, confines itself
with Landlock to the folders involved, reports progress while it works, and
hands back the steps that undo what it did. This module only speaks its
one-JSON-line protocol."""
import json
import os
import shutil
import subprocess

REPO_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
                          "build", "native", "nexa-fileops")


class EngineError(Exception):
    pass


def binary():
    for candidate in (os.environ.get("NEXA_FILEOPS"), "/usr/local/bin/nexa-fileops", REPO_BUILD):
        if candidate and os.access(candidate, os.X_OK):
            return candidate
    return shutil.which("nexa-fileops") or "nexa-fileops"


def run(op, progress=None, started=None, **args):
    """Run one operation. progress(dict) gets {"done", "total", "current"}
    while bytes move; started(Popen) lets the caller cancel it."""
    try:
        proc = subprocess.Popen([binary()], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    except OSError as e:
        raise EngineError(f"the file engine (nexa-fileops) is missing: {e}")
    if started:
        started(proc)
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
        raise EngineError("Cancelled." if proc.returncode and proc.returncode < 0 else "The file engine stopped unexpectedly.")
    if not reply.get("ok"):
        raise EngineError(reply.get("error") or "That didn't work.")
    return reply["result"]


def copy(sources, dest_dir, **kw):
    return run("copy", sources=list(sources), dest_dir=dest_dir, **kw)


def move(sources, dest_dir, **kw):
    return run("move", sources=list(sources), dest_dir=dest_dir, **kw)


def duplicate(paths, **kw):
    return run("duplicate", paths=list(paths), **kw)


def trash(paths, **kw):
    return run("trash", paths=list(paths), **kw)


def delete(paths, **kw):
    return run("delete", paths=list(paths), **kw)


def rename(path, name):
    return run("rename", path=path, name=name)


def new_folder(directory, name):
    return run("new_folder", dir=directory, name=name)


def new_file(directory, name):
    return run("new_file", dir=directory, name=name)


def restore(items, **kw):
    return run("restore", items=list(items), **kw)


def trash_list():
    return run("trash_list")["items"]


def empty_trash():
    return run("empty_trash")


def undo(steps):
    return run("undo", steps=steps)


def size(paths):
    return run("size", paths=list(paths))


def trail(query="", limit=200):
    """FileTrail: changes Files made, newest first, each followed to where
    the thing is now (tools/nexa-sys/src/filetrail.rs)."""
    return run("trail", query=query, limit=limit)["items"]


def trail_forget():
    return run("trail_forget")
