"""Privacy Center: which installed apps may use what, and switching
permissions off. The switches are stored in
~/.config/nexa/app-permissions.json, which nexa-run (tools/nexa-run, Rust)
reads every time it starts an app: it can only take permissions away, never
add one an app didn't declare. No GTK here (the page is in Settings)."""
import json
import os

REGISTRY = "/var/lib/nexa-packages"
APPS_DIR = "/usr/share/applications"
ICON_DIR = "/usr/share/icons/hicolor/scalable/apps"
NEXAOS_KEY = "7003a4c26d287fdf"

# Plain words for each permission, and whether nexa-run can enforce it.
PERMISSIONS = {
    "files.home": ("Your whole home folder", True),
    "files.documents": ("Your Documents folder", True),
    "files.pictures": ("Your Pictures folder", True),
    "files.music": ("Your Music folder", True),
    "files.videos": ("Your Videos folder", True),
    "network": ("The internet", True),
    "camera": ("The camera", True),
    "microphone": ("The microphone", False),
    "location": ("Your location", False),
    "notifications": ("Notifications", False),
    "background": ("Running in the background", False),
}


def prefs_path(home=None):
    return os.path.join(home or os.path.expanduser("~"), ".config", "nexa", "app-permissions.json")


def load_prefs(home=None):
    try:
        with open(prefs_path(home)) as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def save_prefs(prefs, home=None):
    path = prefs_path(home)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(prefs, f, indent=2, sort_keys=True)
    os.replace(tmp, path)


def _desktop_name(app_id, apps_dir):
    try:
        with open(os.path.join(apps_dir, f"nexa-{app_id}.desktop")) as f:
            for line in f:
                if line.startswith("Name="):
                    return line[5:].strip()
    except OSError:
        pass
    return app_id


def list_apps(registry=REGISTRY, apps_dir=APPS_DIR, icon_dir=ICON_DIR, home=None):
    """Installed .nexa apps, by name: id, name, version, who signed it,
    icon, and each declared permission with whether it is allowed now."""
    prefs = load_prefs(home)
    apps = []
    try:
        names = sorted(n for n in os.listdir(registry) if n.endswith(".json"))
    except OSError:
        names = []
    for n in names:
        try:
            with open(os.path.join(registry, n)) as f:
                rec = json.load(f)
        except (OSError, ValueError):
            continue
        app_id = rec.get("id") or n[:-5]
        denied = set((prefs.get(app_id) or {}).get("denied") or [])
        perms = []
        for p in rec.get("permissions") or []:
            label, enforced = PERMISSIONS.get(p, (p, False))
            perms.append({"id": p, "label": label, "enforced": enforced, "allowed": p not in denied})
        icon = next((os.path.join(icon_dir, f"nexa-{app_id}{e}") for e in (".svg", ".png")
                     if os.path.exists(os.path.join(icon_dir, f"nexa-{app_id}{e}"))), None)
        publisher = rec.get("publisher")
        signed = (f"signed by {'NexaOS' if publisher == NEXAOS_KEY else 'publisher ' + str(publisher)}"
                  if rec.get("trust") == "trusted" else "not verified (you chose to install it)")
        apps.append({"id": app_id, "name": _desktop_name(app_id, apps_dir), "version": str(rec.get("version", "")),
                     "signed": signed, "icon": icon, "permissions": perms})
    return sorted(apps, key=lambda a: a["name"].lower())


def set_allowed(app_id, permission, allowed, home=None):
    """Switch one permission on or off for one app; takes effect the next
    time the app starts."""
    prefs = load_prefs(home)
    entry = prefs.get(app_id) if isinstance(prefs.get(app_id), dict) else {}
    denied = set(entry.get("denied") or [])
    if allowed:
        denied.discard(permission)
    else:
        denied.add(permission)
    if denied:
        prefs[app_id] = {"denied": sorted(denied)}
    else:
        prefs.pop(app_id, None)
    save_prefs(prefs, home)


def summary(app):
    allowed = [p["label"] for p in app["permissions"] if p["allowed"]]
    if not app["permissions"]:
        return "Uses only its own files"
    if not allowed:
        return "Everything switched off"
    return "Can use: " + ", ".join(a[0].lower() + a[1:] for a in allowed)
