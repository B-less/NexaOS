"""Browsing history, kept in ~/.local/share/nexa-browser/history.json.
No GTK here, so it can be unit-tested."""
import json
import os
import time

MAX_ENTRIES = 5000
SKIP_SCHEMES = ("about:", "data:", "blob:", "file:")


def default_path():
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "nexa-browser", "history.json")


class History:
    """url -> {title, visits, last}. Saved on every change: writes are a few
    hundred KB at most and happen once per page load."""

    def __init__(self, path=None):
        self.path = path or default_path()
        self.entries = {}
        try:
            with open(self.path) as f:
                data = json.load(f)
            if isinstance(data, dict):
                self.entries = {u: e for u, e in data.items() if isinstance(e, dict)}
        except (OSError, ValueError):
            pass

    def add(self, url, title="", now=None):
        if not url or url.startswith(SKIP_SCHEMES):
            return
        e = self.entries.setdefault(url, {"title": "", "visits": 0, "last": 0})
        e["visits"] += 1
        e["last"] = now if now is not None else time.time()
        if title:
            e["title"] = title
        self._trim()
        self.save()

    def set_title(self, url, title):
        e = self.entries.get(url)
        if e is not None and title and e.get("title") != title:
            e["title"] = title
            self.save()

    def recent(self, limit=50):
        return self._rows(sorted(self.entries.items(), key=lambda kv: -kv[1]["last"])[:limit])

    def top(self, limit=8):
        """Most-visited pages (one per site), for the new-tab page."""
        seen, out = set(), []
        for u, e in sorted(self.entries.items(), key=lambda kv: (-kv[1]["visits"], -kv[1]["last"])):
            site = u.split("://", 1)[-1].split("/", 1)[0]
            if site not in seen:
                seen.add(site)
                out.append((u, e))
            if len(out) >= limit:
                break
        return self._rows(out)

    def search(self, query, limit=8):
        """Entries whose URL or title contains every word of the query,
        most-visited first."""
        words = query.lower().split()
        if not words:
            return []
        hits = [(u, e) for u, e in self.entries.items()
                if all(w in u.lower() or w in e.get("title", "").lower() for w in words)]
        hits.sort(key=lambda kv: (-kv[1]["visits"], -kv[1]["last"]))
        return self._rows(hits[:limit])

    def clear(self):
        self.entries = {}
        self.save()

    def _rows(self, items):
        return [{"url": u, "title": e.get("title") or u, "visits": e["visits"], "last": e["last"]}
                for u, e in items]

    def _trim(self):
        if len(self.entries) > MAX_ENTRIES:
            keep = sorted(self.entries.items(), key=lambda kv: -kv[1]["last"])[:MAX_ENTRIES]
            self.entries = dict(keep)

    def save(self):
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        tmp = self.path + ".tmp"
        with open(os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as f:
            json.dump(self.entries, f)
        os.replace(tmp, self.path)
