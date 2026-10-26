"""The open tabs, so a normal window comes back as it was (never private
or Tor windows). No GTK here, so it can be unit-tested."""
import json
import os

from .prefs import data_dir

MAX_TABS = 50


def path():
    return os.path.join(data_dir(), "session.json")


def save(urls, active=0, file=None):
    urls = [u for u in urls if u and not u.startswith(("about:", "data:", "blob:"))][:MAX_TABS]
    file = file or path()
    os.makedirs(os.path.dirname(file), exist_ok=True)
    tmp = file + ".tmp"
    with open(os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as f:
        json.dump({"tabs": urls, "active": max(0, min(active, len(urls) - 1))}, f)
    os.replace(tmp, file)


def load(file=None):
    try:
        with open(file or path()) as f:
            d = json.load(f)
        tabs = [u for u in d.get("tabs", []) if isinstance(u, str)][:MAX_TABS]
        return tabs, int(d.get("active", 0)) if tabs else 0
    except (OSError, ValueError, TypeError, AttributeError):
        return [], 0
