"""Bookmarks, in ~/.local/share/nexa-browser/bookmarks.json (0600), each
with a folder ("" for none, "Work/Tools" for a folder in a folder).
Importing and exporting other browsers' HTML files is nexa-webdata's work.
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
        for b in self.items:
            b.setdefault("folder", "")
        self.extra_folders = []  # made but still empty

    def has(self, url):
        return any(b["url"] == url for b in self.items)

    def add(self, url, title="", now=None, folder=""):
        if not url or self.has(url) or url.startswith(("about:", "data:")):
            return False
        self.items.insert(0, {"url": url, "title": title or url, "added": now or time.time(), "folder": folder})
        self.save()
        return True

    def get(self, url):
        return next((b for b in self.items if b["url"] == url), None)

    def edit(self, url, title=None, new_url=None, folder=None):
        """Change a bookmark. False if it's gone, or new_url is taken."""
        b = self.get(url)
        if b is None or (new_url and new_url != url and self.has(new_url)):
            return False
        if title is not None:
            b["title"] = title.strip() or b["url"]
        if new_url:
            b["url"] = new_url
        if folder is not None:
            b["folder"] = clean_folder(folder)
        self.save()
        return True

    def folders(self):
        """Every folder, parents included, sorted."""
        out = set(self.extra_folders)
        for b in self.items:
            parts = [p for p in b.get("folder", "").split("/") if p]
            for n in range(1, len(parts) + 1):
                out.add("/".join(parts[:n]))
        return sorted(out, key=str.lower)

    def in_folder(self, folder, query=""):
        """Bookmarks directly in `folder` (None: all), matching the search."""
        return [b for b in self.search(query) if folder is None or b.get("folder", "") == folder]

    def add_folder(self, name, parent=""):
        f = clean_folder(f"{parent}/{name}" if parent else name)
        if f and f not in self.folders():
            self.extra_folders.append(f)
        return f

    def rename_folder(self, old, new):
        new = clean_folder(new)
        if not new or old == new:
            return False
        for b in self.items:
            f = b.get("folder", "")
            if f == old or f.startswith(old + "/"):
                b["folder"] = new + f[len(old):]
        self.extra_folders = [new + f[len(old):] if f == old or f.startswith(old + "/") else f for f in self.extra_folders]
        self.save()
        return True

    def delete_folder(self, folder):
        """The folder and everything in it. Returns how many bookmarks went."""
        n = len(self.items)
        self.items = [b for b in self.items if not (b.get("folder", "") == folder or b.get("folder", "").startswith(folder + "/"))]
        self.extra_folders = [f for f in self.extra_folders if not (f == folder or f.startswith(folder + "/"))]
        self.save()
        return n - len(self.items)

    def add_many(self, items, under=""):
        """Imported bookmarks (in folder `under`); ones already here are skipped."""
        added = 0
        for it in items:
            url = it.get("url")
            if not url or self.has(url):
                continue
            folder = "/".join(p for p in (under, it.get("folder", "")) if p)
            self.items.append({"url": url, "title": it.get("title") or url, "added": it.get("added") or time.time(),
                               "folder": clean_folder(folder)})
            added += 1
        if added:
            self.save()
        return added

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
        return [b for b in self.items if all(w in b["url"].lower() or w in b["title"].lower() or w in b.get("folder", "").lower() for w in words)]

    def save(self):
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        tmp = self.path + ".tmp"
        with open(os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as f:
            json.dump(self.items, f, indent=1)
        os.replace(tmp, self.path)


def clean_folder(f):
    """"a//b/ " -> "a/b"; no empty parts, no dots-only parts."""
    return "/".join(p.strip() for p in (f or "").split("/") if p.strip() and p.strip() not in (".", ".."))[:300]
