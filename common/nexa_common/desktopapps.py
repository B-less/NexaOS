"""Installed applications, read from their .desktop files: listing,
search and launching. Used by the start menu."""
import configparser
import glob
import os
import shlex
import subprocess

APPS_DIRS = ("/usr/share/applications", "/usr/local/share/applications")

# Halves of other apps or legacy sessions, never launched on their own.
SKIP_NAMES = {"Foot Server", "Foot Client", "Nexa Framebuffer Desktop"}


def load_apps(dirs=APPS_DIRS):
    apps, seen = [], set()
    for d in dirs:
        for path in sorted(glob.glob(os.path.join(d, "*.desktop"))):
            cp = configparser.ConfigParser(interpolation=None, strict=False)
            try:
                cp.read(path)
            except configparser.Error:
                continue
            if "Desktop Entry" not in cp:
                continue
            e = cp["Desktop Entry"]
            if e.get("Type", "Application") != "Application":
                continue
            if e.getboolean("NoDisplay", fallback=False) or e.getboolean("Hidden", fallback=False):
                continue
            name, exec_line = e.get("Name"), e.get("Exec")
            if not name or not exec_line or name in SKIP_NAMES or name in seen:
                continue
            seen.add(name)
            apps.append({
                "id": os.path.basename(path)[:-len(".desktop")],
                "name": name,
                "exec": exec_line,
                "icon": e.get("Icon", "application-x-executable"),
                "comment": e.get("Comment", ""),
                "keywords": " ".join(filter(None, [e.get("GenericName", ""), e.get("Keywords", ""),
                                                   e.get("Categories", "")])),
            })
    return sorted(apps, key=lambda a: a["name"].lower())


def search(apps, query):
    """Name matches first (prefix before substring), then description and
    keyword matches; every word of the query has to appear somewhere."""
    words = query.lower().split()
    if not words:
        return list(apps)
    scored = []
    for a in apps:
        name = a["name"].lower()
        hay = " ".join([name, a["comment"].lower(), a["keywords"].lower().replace(";", " ")])
        if not all(w in hay for w in words):
            continue
        if name.startswith(query.lower().strip()):
            rank = 0
        elif all(w in name for w in words):
            rank = 1
        else:
            rank = 2
        scored.append((rank, name, a))
    return [a for _, _, a in sorted(scored, key=lambda x: (x[0], x[1]))]


def command(exec_line):
    # %f/%u-style field codes need a file or URL, and nothing is passed here.
    return [a for a in shlex.split(exec_line) if not (len(a) == 2 and a.startswith("%"))]


def launch(exec_line):
    argv = command(exec_line)
    if argv:
        subprocess.Popen(argv, start_new_session=True,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
