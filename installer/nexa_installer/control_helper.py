import json
import os
import shutil
import subprocess
import sys
import tempfile

from . import packaging

# Runs as root via `doas /usr/local/bin/nexa-install-helper` (see
# /etc/doas.conf), same shape as nexa-ai's and nexa-software's control
# helpers. Only ever places or removes files under a fixed set of system
# directories (/usr/local/bin, /usr/local/lib/nexa-<id>,
# /usr/share/applications, /usr/share/icons/hicolor), never over anything it
# didn't install itself (see REGISTRY_DIR) -- it never executes
# the installed payload itself, so installing a package here carries the
# same trust model as `apk add`: root places the files, but they only ever
# run later as the unprivileged "user" account, same as every other app.

BIN_DIR = "/usr/local/bin"
LIB_ROOT = "/usr/local/lib"
APPS_DIR = "/usr/share/applications"
ICON_DIR = "/usr/share/icons/hicolor/scalable/apps"
# One <id>.json per installed package, listing exactly the paths it owns.
# A package may only overwrite or remove paths recorded under its own id.
REGISTRY_DIR = "/var/lib/nexa-packages"
# A package's bin/<id> lands in /usr/local/bin, which comes first on PATH, so
# an id that matches any existing command would shadow it -- for root too
# (e.g. a fake `su` that records the password). Checked for unregistered ids.
COMMAND_DIRS = ("/usr/local/sbin", "/usr/local/bin", "/usr/sbin", "/usr/bin", "/sbin", "/bin")


def _refresh_caches():
    subprocess.run(["update-desktop-database", APPS_DIR], capture_output=True, timeout=15)
    subprocess.run(
        ["gtk-update-icon-cache", "-f", "-t", "/usr/share/icons/hicolor"],
        capture_output=True, timeout=15,
    )


def _registry_path(pkg_id):
    return os.path.join(REGISTRY_DIR, f"{pkg_id}.json")


def _registered_paths(pkg_id):
    try:
        with open(_registry_path(pkg_id)) as f:
            paths = json.load(f)["paths"]
    except (OSError, json.JSONDecodeError, KeyError, TypeError):
        return None
    return [p for p in paths if isinstance(p, str)]


def _owned_paths(pkg_id):
    """Every path an install of pkg_id may write."""
    return {
        "bin": os.path.join(BIN_DIR, pkg_id),
        "lib": os.path.join(LIB_ROOT, f"nexa-{pkg_id}"),
        "desktop": os.path.join(APPS_DIR, f"nexa-{pkg_id}.desktop"),
        "icons": [os.path.join(ICON_DIR, f"nexa-{pkg_id}{ext}") for ext in (".svg", ".png")],
    }


def _check_no_collision(pkg_id, owned):
    """An id that isn't already this package's must not touch anything that
    exists: no root helper (/usr/local/bin/nexa-*-helper), no system
    component's code (/usr/local/lib/nexa-<name>), no system command."""
    if _registered_paths(pkg_id) is not None:
        return
    for path in [owned["bin"], owned["lib"], owned["desktop"], *owned["icons"]]:
        if os.path.lexists(path):
            raise packaging.PackageError(
                f"'{pkg_id}' conflicts with something already on the system ({path})")
    for d in COMMAND_DIRS:
        if os.path.lexists(os.path.join(d, pkg_id)):
            raise packaging.PackageError(f"'{pkg_id}' is the name of an existing system command")


def _resolve_inside(root, rel):
    """rel as a regular file inside root, or None. Blocks absolute paths, ..
    and symlinks, so a manifest can't make root read e.g. /etc/shadow."""
    if not rel or os.path.isabs(rel):
        return None
    root = os.path.realpath(root)
    path = os.path.realpath(os.path.join(root, rel))
    if not path.startswith(root + os.sep) or not os.path.isfile(path):
        return None
    return path


def _desktop_value(value):
    # One line only: a newline in the manifest would add arbitrary keys.
    return " ".join(str(value).split())


def _remove(path):
    if os.path.isdir(path) and not os.path.islink(path):
        shutil.rmtree(path)
    elif os.path.lexists(path):
        os.remove(path)


def install(args):
    nexa_path = args["path"]
    if not os.path.isfile(nexa_path):
        raise packaging.PackageError("package file not found")

    with tempfile.TemporaryDirectory(prefix="nexa-install-") as tmp:
        # Work on a private copy: the original is user-writable and could be
        # swapped between the checks below and the extraction.
        pkg_copy = os.path.join(tmp, "package.nexa")
        shutil.copyfile(nexa_path, pkg_copy)
        payload = os.path.join(tmp, "payload")
        os.mkdir(payload)
        manifest = packaging.extract_payload(pkg_copy, payload)
        pkg_id = manifest["id"]
        owned = _owned_paths(pkg_id)
        _check_no_collision(pkg_id, owned)

        # Upgrading: clear out what the previous version installed.
        for path in _registered_paths(pkg_id) or []:
            _remove(path)
        installed = []
        try:
            bin_src = _resolve_inside(payload, os.path.join("bin", pkg_id))
            if bin_src is None:
                raise packaging.PackageError(f"Package is missing its entry point: bin/{pkg_id}")
            shutil.copyfile(bin_src, owned["bin"])
            os.chmod(owned["bin"], 0o755)
            installed.append(owned["bin"])

            lib_src = os.path.join(payload, "lib")
            if os.path.isdir(lib_src) and not os.path.islink(lib_src):
                shutil.copytree(lib_src, owned["lib"], symlinks=True)
                installed.append(owned["lib"])

            icon_name = f"nexa-{pkg_id}"
            icon_src = _resolve_inside(payload, manifest.get("icon", ""))
            if icon_src:
                ext = os.path.splitext(icon_src)[1].lower()
                ext = ext if ext in (".svg", ".png") else ".svg"
                os.makedirs(ICON_DIR, exist_ok=True)
                icon_dest = os.path.join(ICON_DIR, icon_name + ext)
                shutil.copyfile(icon_src, icon_dest)
                installed.append(icon_dest)

            with open(owned["desktop"], "w") as f:
                f.write(
                    "[Desktop Entry]\n"
                    "Type=Application\n"
                    f"Name={_desktop_value(manifest['name'])}\n"
                    f"Comment={_desktop_value(manifest.get('description', ''))}\n"
                    f"Exec={owned['bin']}\n"
                    f"Icon={icon_name}\n"
                    "Terminal=false\n"
                    f"Categories={_desktop_value(manifest.get('categories', 'Utility;'))}\n"
                )
            installed.append(owned["desktop"])

            os.makedirs(REGISTRY_DIR, exist_ok=True)
            with open(_registry_path(pkg_id), "w") as f:
                json.dump({"id": pkg_id, "version": str(manifest["version"]), "paths": installed}, f)
        except BaseException:
            # Never leave unregistered files behind: they would block this id
            # from installing again. Every owned path is safe to remove here:
            # either nothing existed there (_check_no_collision) or it belonged
            # to this same package's previous version.
            for path in [owned["bin"], owned["lib"], owned["desktop"], *owned["icons"]]:
                _remove(path)
            if os.path.lexists(_registry_path(pkg_id)):
                os.remove(_registry_path(pkg_id))
            raise

        _refresh_caches()

    return f"Installed {_desktop_value(manifest['name'])} {_desktop_value(manifest['version'])}"


def uninstall(args):
    pkg_id = args["id"]
    if not isinstance(pkg_id, str) or not packaging.ID_RE.match(pkg_id):
        raise packaging.PackageError("invalid package id")
    paths = _registered_paths(pkg_id)
    if paths is None:
        raise packaging.PackageError(f"'{pkg_id}' is not an installed package")
    # Only ever the paths this package itself could have created.
    allowed = _owned_paths(pkg_id)
    allowed = {allowed["bin"], allowed["lib"], allowed["desktop"], *allowed["icons"]}
    for path in paths:
        if path in allowed:
            _remove(path)
    os.remove(_registry_path(pkg_id))
    _refresh_caches()
    return f"Removed {pkg_id}"


ACTIONS = {"install": install, "uninstall": uninstall}


def main():
    try:
        request = json.loads(sys.stdin.read())
        action = request["action"]
        args = request.get("args") or {}
    except (json.JSONDecodeError, KeyError, TypeError):
        print(json.dumps({"ok": False, "error": "malformed request"}))
        sys.exit(1)

    handler = ACTIONS.get(action)
    if handler is None:
        print(json.dumps({"ok": False, "error": f"'{action}' is not a known action"}))
        sys.exit(1)

    try:
        output = handler(args)
        print(json.dumps({"ok": True, "output": output}))
    except (packaging.PackageError, KeyError, OSError) as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
