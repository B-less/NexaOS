"""The open tabs, so a normal window comes back as it was (never private
or Tor windows). No GTK here, so it can be unit-tested."""
import json
import os

from .prefs import data_dir

MAX_TABS = 50
MAX_CLOSED = 25


def path():
    return os.path.join(data_dir(), "session.json")


def _keep(u):
    return isinstance(u, str) and u and not u.startswith(("about:", "data:", "blob:"))


def save(urls, active=0, file=None, pinned=(), closed=()):
    """pinned: the urls of pinned tabs; closed: [{url, title}], newest last."""
    urls = [u for u in urls if _keep(u)][:MAX_TABS]
    file = file or path()
    os.makedirs(os.path.dirname(file), exist_ok=True)
    tmp = file + ".tmp"
    closed = [{"url": c["url"], "title": str(c.get("title") or "")[:300]} for c in closed if isinstance(c, dict) and _keep(c.get("url"))][-MAX_CLOSED:]
    with open(os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as f:
        json.dump({"tabs": urls, "active": max(0, min(active, len(urls) - 1)), "pinned": [u for u in pinned if u in urls],
                   "closed": closed}, f)
    os.replace(tmp, file)


def load(file=None):
    try:
        with open(file or path()) as f:
            d = json.load(f)
        tabs = [u for u in d.get("tabs", []) if isinstance(u, str)][:MAX_TABS]
        return tabs, int(d.get("active", 0)) if tabs else 0
    except (OSError, ValueError, TypeError, AttributeError):
        return [], 0


def load_extra(file=None):
    """(pinned urls, recently closed [{url, title}]) from the saved session."""
    try:
        with open(file or path()) as f:
            d = json.load(f)
        pinned = [u for u in d.get("pinned", []) if _keep(u)]
        closed = [c for c in d.get("closed", []) if isinstance(c, dict) and _keep(c.get("url"))][-MAX_CLOSED:]
        return pinned, closed
    except (OSError, ValueError, TypeError, AttributeError):
        return [], []
