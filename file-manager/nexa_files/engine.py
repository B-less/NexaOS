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


def _find(name, env):
    built = os.path.join(os.path.dirname(REPO_BUILD), name)
    for candidate in (os.environ.get(env), f"/usr/local/bin/{name}", built):
        if candidate and os.access(candidate, os.X_OK):
            return candidate
    return shutil.which(name) or name


def binary():
    return _find("nexa-fileops", "NEXA_FILEOPS")


def archiver():
    """nexa-archiver (Rust, tools/nexa-archiver): compress and extract."""
    return _find("nexa-archiver", "NEXA_ARCHIVER")


def run(op, progress=None, started=None, program=None, found=None, **args):
    """Run one operation. progress(dict) gets {"done", "total", "current"}
    while bytes move; started(Popen) lets the caller cancel it. program:
    another engine with the same protocol (nexa-archiver)."""
    program = program or binary()
    try:
        proc = subprocess.Popen([program], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    except OSError as e:
        raise EngineError(f"the file engine ({os.path.basename(program)}) is missing: {e}")
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
        elif "found" in msg:
            if found:
                found(msg["found"])
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


def chmod(paths, mode):
    """Set read/write/run bits (0o000-0o777); undo puts the old ones back."""
    return run("chmod", paths=list(paths), mode=int(mode))


def rename_many(items):
    """items: [(path, new name)]; all at once, so names can be swapped."""
    return run("rename_many", items=[{"path": p, "name": n} for p, n in items])


def size(paths):
    return run("size", paths=list(paths))


def trail(query="", limit=200):
    """FileTrail: changes Files made, newest first, each followed to where
    the thing is now (tools/nexa-sys/src/filetrail.rs)."""
    return run("trail", query=query, limit=limit)["items"]


def trail_forget():
    return run("trail_forget")


# ---- Archives (nexa-archiver) ----

ARCHIVE_SUFFIXES = (".zip", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tbz",
                    ".tar.zst", ".tzst", ".gz", ".xz", ".bz2", ".zst")


def is_archive(name):
    return name.lower().endswith(ARCHIVE_SUFFIXES)


def extract(archive, dest_dir, **kw):
    """Unpack next to it: one item comes out as itself, several in a new
    folder named after the archive. Never replaces anything."""
    return run("extract", archive=archive, dest_dir=dest_dir, program=archiver(), **kw)


def compress(paths, fmt="zip", name=None, dest_dir=None, **kw):
    """Pack paths into a new .zip or .tar.gz (a free name, never replacing)."""
    args = dict(paths=list(paths), format=fmt)
    if name:
        args["name"] = name
    if dest_dir:
        args["dest_dir"] = dest_dir
    return run("compress", program=archiver(), **args, **kw)


def list_archive(archive):
    return run("list", archive=archive, program=archiver())


def search(directory, query, found, hidden=False, limit=2000, started=None):
    """Names under directory containing every word of query; found(list)
    gets the matches in batches while the engine looks."""
    return run("search", dir=directory, query=query, hidden=hidden, limit=limit, found=found, started=started)
