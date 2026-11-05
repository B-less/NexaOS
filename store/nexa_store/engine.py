"""The Nexa Store's thin client. The work happens in Rust: nexa-store
(tools/nexa-store) fetches and checks the signed catalogues and downloads,
and the system broker (nexa-brokerd) installs and removes apps, checking
each package's signature again. This module only runs them and reads
their JSON."""
import json
import os
import subprocess

BIN = os.environ.get("NEXA_STORE_BIN", "nexa-store")

# What each permission lets an app do, in plain words (docs/WRITING_NEXA_APPS.md).
PERMISSIONS = {
    "network": ("network-wireless-symbolic", "Use the internet"),
    "files.home": ("user-home-symbolic", "Read and change every file in your home folder"),
    "files.documents": ("folder-documents-symbolic", "Read and change your Documents folder"),
    "files.pictures": ("folder-pictures-symbolic", "Read and change your Pictures folder"),
    "files.music": ("folder-music-symbolic", "Read and change your Music folder"),
    "files.videos": ("folder-videos-symbolic", "Read and change your Videos folder"),
    "files.drives": ("drive-removable-media-symbolic", "Read and change USB sticks and other drives"),
    "camera": ("camera-web-symbolic", "Use the camera"),
    "microphone": ("audio-input-microphone-symbolic", "Use the microphone"),
    "location": ("find-location-symbolic", "Know where you are"),
    "notifications": ("preferences-system-notifications-symbolic", "Show notifications"),
    "background": ("system-run-symbolic", "Keep running after you close it"),
}

# Freedesktop categories, as the Store's sidebar names them (first match wins).
CATEGORIES = [
    ("AudioVideo", "Music & video"), ("Graphics", "Graphics"), ("Office", "Office"),
    ("Network", "Internet"), ("Game", "Games"), ("Development", "Developer tools"),
    ("Education", "Learning"), ("Science", "Science"), ("System", "System"), ("Utility", "Utilities"),
]


class StoreError(Exception):
    pass


def category_of(app):
    cats = [c for c in (app.get("categories") or "").split(";") if c]
    for key, label in CATEGORIES:
        if key in cats:
            return label
    return "Utilities"


def human_size(n):
    n = float(n or 0)
    for unit in ("bytes", "KB", "MB", "GB"):
        if n < 1000 or unit == "GB":
            return f"{n:.0f} {unit}" if unit == "bytes" else f"{n:.1f} {unit}"
        n /= 1000


def run(*args, progress=None, timeout=900):
    """Run nexa-store; progress(dict) gets its progress lines."""
    try:
        p = subprocess.Popen([BIN, *args], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    except OSError:
        raise StoreError("The Nexa Store's engine isn't installed on this system.")
    last = None
    try:
        for line in p.stdout:
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if isinstance(msg, dict) and "progress" in msg:
                if progress:
                    progress(msg["progress"])
            elif isinstance(msg, dict):
                last = msg
        p.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill()
        raise StoreError("The store took too long to answer - check the internet connection and try again.")
    if last is None:
        raise StoreError("The store's engine stopped without an answer.")
    if not last.get("ok"):
        raise StoreError(last.get("error") or "The store couldn't do that.")
    return last.get("result") or {}


def refresh():
    return run("refresh")


def list_apps():
    return run("list")


def installed():
    return run("installed").get("apps", [])


def download(app_id, progress=None):
    return run("download", app_id, progress=progress)


def install(path, allow_unverified, ask_password):
    """Through the system broker. Raises broker.Unverified when only the
    publisher's signature is missing, so the Store can ask first."""
    from nexa_common import broker
    return broker.run("install_app", {"path": path, "allow_unverified": allow_unverified}, ask_password)


def uninstall(app_id, ask_password):
    from nexa_common import broker
    return broker.run("uninstall_app", {"id": app_id}, ask_password)
