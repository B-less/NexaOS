"""Bookmarks, in ~/.local/share/nexa-browser/bookmarks.json (0600).
No GTK here, so it can be unit-tested."""
import json
import os
import time

from .prefs import data_dir


class Bookmarks:
    def __init__(self, path=None):
        self.path = path or os.path.join(data_dir(), "bookmarks.json")
        self.items = []
        try:
            with open(self.path) as f:
                data = json.load(f)
            self.items = [b for b in data if isinstance(b, dict) and b.get("url")]
        except (OSError, ValueError, TypeError):
            pass

    def has(self, url):
        return any(b["url"] == url for b in self.items)

    def add(self, url, title="", now=None):
        if not url or self.has(url) or url.startswith(("about:", "data:")):
            return False
        self.items.insert(0, {"url": url, "title": title or url, "added": now or time.time()})
        self.save()
        return True

    def remove(self, url):
        n = len(self.items)
        self.items = [b for b in self.items if b["url"] != url]
        if len(self.items) != n:
            self.save()
            return True
        return False

    def toggle(self, url, title=""):
        """Add, or remove if it was there. True if it is bookmarked now."""
        if self.remove(url):
            return False
        return self.add(url, title)

    def search(self, query=""):
        words = query.lower().split()
        return [b for b in self.items if all(w in b["url"].lower() or w in b["title"].lower() for w in words)]

    def save(self):
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        tmp = self.path + ".tmp"
        with open(os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as f:
            json.dump(self.items, f, indent=1)
        os.replace(tmp, self.path)
