"""Nexa Browser's settings, in ~/.local/share/nexa-browser/prefs.json (0600).
No GTK here, so it can be unit-tested."""
import copy
import json
import os
from urllib.parse import urlsplit

SEARCH_ENGINES = {
    "duckduckgo": ("DuckDuckGo", "https://duckduckgo.com/?q="),
    "startpage": ("Startpage", "https://www.startpage.com/do/search?q="),
    "brave": ("Brave Search", "https://search.brave.com/search?q="),
    "ecosia": ("Ecosia", "https://www.ecosia.org/search?q="),
    "bing": ("Bing", "https://www.bing.com/search?q="),
    "google": ("Google", "https://www.google.com/search?q="),
}
PERMISSIONS = {"camera": "use your camera", "microphone": "use your microphone", "location": "know your location",
               "notifications": "show notifications", "media-keys": "play protected media"}
SUGGESTED_LISTS = [
    ("EasyList (ads)", "https://easylist.to/easylist/easylist.txt"),
    ("EasyPrivacy (tracking)", "https://easylist.to/easylist/easyprivacy.txt"),
]

DEFAULTS = {
    "search": "duckduckgo",
    "home": "",                # empty: the new-tab page
    "https_only": True,
    "block_ads": True,
    "blocking_off": [],        # sites where blocking is off
    "filter_lists": [],        # extra lists: {"name", "url"}
    "restore_session": True,
    "zoom": {},                # host -> zoom level
    "permissions": {},         # host -> {kind: true/false}
    "spellcheck": True,
}


def data_dir():
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "nexa-browser")


def host_of(uri):
    try:
        return (urlsplit(uri).hostname or "").lower()
    except ValueError:
        return ""


def site_of(uri_or_host):
    """The site a page belongs to: its host without "www."."""
    h = host_of(uri_or_host) if "://" in uri_or_host else uri_or_host.lower()
    return h[4:] if h.startswith("www.") else h


class Prefs:
    def __init__(self, path=None):
        self.path = path or os.path.join(data_dir(), "prefs.json")
        self.data = copy.deepcopy(DEFAULTS)
        try:
            with open(self.path) as f:
                saved = json.load(f)
            if isinstance(saved, dict):
                for k, v in saved.items():
                    if k in DEFAULTS and isinstance(v, type(DEFAULTS[k])):
                        self.data[k] = v
        except (OSError, ValueError):
            pass
        if self.data["search"] not in SEARCH_ENGINES:
            self.data["search"] = DEFAULTS["search"]

    def __getitem__(self, key):
        return self.data[key]

    def set(self, key, value):
        if key not in DEFAULTS:
            raise KeyError(key)
        self.data[key] = value
        self.save()

    def save(self):
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        tmp = self.path + ".tmp"
        with open(os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as f:
            json.dump(self.data, f, indent=1, sort_keys=True)
        os.replace(tmp, self.path)

    def search_url(self):
        return SEARCH_ENGINES[self.data["search"]][1]

    # ---- per site
    def zoom_for(self, uri):
        return float(self.data["zoom"].get(site_of(uri), 1.0))

    def set_zoom(self, uri, level):
        site = site_of(uri)
        if not site:
            return
        level = round(max(0.3, min(3.0, level)), 2)
        if abs(level - 1.0) < 0.01:
            self.data["zoom"].pop(site, None)
        else:
            self.data["zoom"][site] = level
        self.save()

    def permission(self, uri, kind):
        """True / False if remembered for this site, else None (ask)."""
        return self.data["permissions"].get(site_of(uri), {}).get(kind)

    def set_permission(self, uri, kind, allowed):
        site = site_of(uri)
        if site and kind in PERMISSIONS:
            self.data["permissions"].setdefault(site, {})[kind] = bool(allowed)
            self.save()

    def forget_permissions(self, site=None):
        if site is None:
            self.data["permissions"] = {}
        else:
            self.data["permissions"].pop(site, None)
        self.save()

    def blocking_on(self, uri):
        site = site_of(uri)
        return self.data["block_ads"] and not any(site == s or site.endswith("." + s) for s in self.data["blocking_off"])

    def set_blocking(self, uri, on):
        site = site_of(uri)
        off = [s for s in self.data["blocking_off"] if s != site]
        if not on and site:
            off.append(site)
        self.data["blocking_off"] = sorted(off)
        self.save()
