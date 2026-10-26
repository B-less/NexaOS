"""FileTrail in Files: "Where did it go?". Everything Files moved, renamed,
copied or put in the Trash, newest first, followed to where it is now, with
Show and Put it back. The journal and the following are nexa-fileops'
(Rust, nexa_sys::filetrail); this module says it in words, trail_window.py
is the window."""
import os
import time

KIND = {"moved": "Moved", "renamed": "Renamed", "copied": "Copied", "trashed": "Put in the Trash",
        "restored": "Restored from the Trash", "created": "Created", "put back": "Put back",
        "deleted": "Deleted for good"}


def place(path, home=None):
    """A folder as people say it: "Documents/Work", "the USB stick"."""
    home = home or os.path.expanduser("~")
    folder = os.path.dirname(path)
    if folder == home:
        return "your home folder"
    if folder.startswith(home + "/"):
        return folder[len(home) + 1:]
    if folder.startswith(("/media/", "/run/media/", "/mnt/")):
        return folder
    return folder


def when(at, now=None):
    now = now or time.time()
    lt, ln = time.localtime(at), time.localtime(now)
    clock = time.strftime("%H:%M", lt)
    if lt.tm_yday == ln.tm_yday and lt.tm_year == ln.tm_year:
        return f"today {clock}"
    if now - at < 2 * 86400 and (ln.tm_yday - lt.tm_yday) % 366 == 1:
        return f"yesterday {clock}"
    return time.strftime("%d %b, %H:%M", lt)


def describe(item, home=None):
    """(what happened, where it is now) in words."""
    k = item["kind"]
    if k in ("moved", "put back"):
        what = f"{KIND[k]} from {place(item['from'], home)} to {place(item['to'], home)}"
    elif k == "renamed":
        what = f"Renamed from “{os.path.basename(item['from'])}”"
    elif k == "copied" and item.get("from"):
        what = f"Copied from {place(item['from'], home)} to {place(item['to'], home)}"
    elif k == "trashed":
        what = f"Put in the Trash from {place(item['from'], home)}"
    elif k == "deleted":
        what = f"Deleted for good from {place(item['to'], home)}"
    else:
        what = f"{KIND.get(k, k)} in {place(item['to'], home)}"
    status = item.get("status")
    if status == "here":
        now = f"Now in {place(item['now'], home)}"
    elif status == "in-trash":
        now = "In the Trash"
    elif status == "deleted":
        now = "Deleted for good with Files"
    elif status == "emptied":
        now = "Gone: the Trash was emptied"
    else:
        now = "Not there any more: moved or deleted outside Files"
    return f"{what} · {when(item['at'])}", now
