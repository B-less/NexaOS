"""Ad and tracker blocking: the lists (built-in, plus any added in
Settings) are compiled by nexa-webfilter (Rust, tools/nexa-webfilter) into
WebKit's content-blocker rules; WebKit then blocks inside the engine. This
module only finds the lists, runs the compiler and caches the result.
No GTK here, so it can be unit-tested."""
import hashlib
import json
import os
import re
import shutil
import subprocess
import time
import urllib.request

from .prefs import data_dir

BUILT_IN = os.path.join(os.path.dirname(__file__), "filters", "nexa-basic.txt")
REPO_BUILD = os.path.join(os.path.dirname(__file__), "..", "..", "build", "native", "nexa-webfilter")
MAX_LIST = 16 << 20
REFRESH_DAYS = 7


class BlockerError(Exception):
    pass


def binary():
    for c in (os.environ.get("NEXA_WEBFILTER"), "/usr/local/bin/nexa-webfilter", REPO_BUILD):
        if c and os.access(c, os.X_OK):
            return c
    return shutil.which("nexa-webfilter") or "nexa-webfilter"


def lists_dir():
    return os.path.join(data_dir(), "filters")


def list_file(url):
    name = re.sub(r"[^a-z0-9]+", "-", url.lower()).strip("-")[-60:]
    return os.path.join(lists_dir(), name + ".txt")


def list_files(prefs):
    files = [BUILT_IN]
    for item in prefs["filter_lists"]:
        f = list_file(item["url"])
        if os.path.isfile(f):
            files.append(f)
    return files


def compile_rules(prefs):
    """(identifier, rules JSON bytes, summary). The identifier changes when
    the lists or the sites with blocking off change, so WebKit recompiles only then."""
    files = list_files(prefs)
    h = hashlib.sha256()
    for f in files:
        st = os.stat(f)
        h.update(f"{f}:{st.st_size}:{st.st_mtime_ns}".encode())
    h.update(json.dumps(prefs["blocking_off"]).encode())
    ident = "nexa-" + h.hexdigest()[:20]
    cache = os.path.join(data_dir(), "rules", ident + ".json")
    if os.path.isfile(cache):
        with open(cache, "rb") as fh:
            return ident, fh.read(), None
    argv = [binary(), "compile"]
    for site in prefs["blocking_off"]:
        argv += ["--allow", site]
    try:
        r = subprocess.run(argv + files, capture_output=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise BlockerError(f"the blocker (nexa-webfilter) didn't run: {e}") from None
    if r.returncode != 0:
        raise BlockerError(r.stderr.decode(errors="replace")[:300])
    try:
        summary = json.loads(r.stderr.decode().strip().splitlines()[-1])
    except (ValueError, IndexError):
        summary = None
    os.makedirs(os.path.dirname(cache), exist_ok=True)
    for old in os.listdir(os.path.dirname(cache)):  # one cached set at a time
        if old.endswith(".json") and old != ident + ".json":
            os.remove(os.path.join(os.path.dirname(cache), old))
    with open(cache, "wb") as fh:
        fh.write(r.stdout)
    return ident, r.stdout, summary


def download_list(url, timeout=30):
    """Fetch a filter list (https only) into the lists folder."""
    if not url.startswith("https://"):
        raise BlockerError("Filter lists are fetched over https only.")
    req = urllib.request.Request(url, headers={"User-Agent": "NexaBrowser-filterlists"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = resp.read(MAX_LIST + 1)
    except OSError as e:
        raise BlockerError(f"Couldn't download it: {e}") from None
    if len(data) > MAX_LIST:
        raise BlockerError("That list is too big (over 16 MB).")
    os.makedirs(lists_dir(), exist_ok=True)
    tmp = list_file(url) + ".tmp"
    with open(tmp, "wb") as fh:
        fh.write(data)
    os.replace(tmp, list_file(url))
    return len(data)


def stale_lists(prefs, now=None):
    """Added lists not fetched in a week (or never)."""
    now = now or time.time()
    out = []
    for item in prefs["filter_lists"]:
        f = list_file(item["url"])
        if not os.path.isfile(f) or now - os.path.getmtime(f) > REFRESH_DAYS * 86400:
            out.append(item["url"])
    return out
