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
# /usr/share/applications, /usr/share/icons/hicolor) -- it never executes
# the installed payload itself, so installing a package here carries the
# same trust model as `apk add`: root places the files, but they only ever
# run later as the unprivileged "user" account, same as every other app.

BIN_DIR = "/usr/local/bin"
LIB_ROOT = "/usr/local/lib"
APPS_DIR = "/usr/share/applications"
ICON_DIR = "/usr/share/icons/hicolor/scalable/apps"


def _refresh_caches():
    subprocess.run(["update-desktop-database", APPS_DIR], capture_output=True, timeout=15)
    subprocess.run(
        ["gtk-update-icon-cache", "-f", "-t", "/usr/share/icons/hicolor"],
        capture_output=True, timeout=15,
    )


def install(args):
    nexa_path = args["path"]
    if not os.path.isfile(nexa_path):
        raise packaging.PackageError("package file not found")

    with tempfile.TemporaryDirectory(prefix="nexa-install-") as tmp:
        manifest = packaging.extract_payload(nexa_path, tmp)
        pkg_id = manifest["id"]

        bin_dest = os.path.join(BIN_DIR, pkg_id)
        shutil.copy(os.path.join(tmp, "bin", pkg_id), bin_dest)
        os.chmod(bin_dest, 0o755)

        lib_src = os.path.join(tmp, "lib")
        if os.path.isdir(lib_src):
            lib_dest = os.path.join(LIB_ROOT, f"nexa-{pkg_id}")
            if os.path.isdir(lib_dest):
                shutil.rmtree(lib_dest)
            shutil.copytree(lib_src, lib_dest)

        icon_name = f"nexa-{pkg_id}"
        icon_rel = manifest.get("icon", "")
        icon_src = os.path.join(tmp, icon_rel) if icon_rel else ""
        if icon_rel and os.path.isfile(icon_src):
            os.makedirs(ICON_DIR, exist_ok=True)
            ext = os.path.splitext(icon_rel)[1] or ".svg"
            shutil.copy(icon_src, os.path.join(ICON_DIR, icon_name + ext))

        desktop_path = os.path.join(APPS_DIR, f"nexa-{pkg_id}.desktop")
        with open(desktop_path, "w") as f:
            f.write(
                "[Desktop Entry]\n"
                "Type=Application\n"
                f"Name={manifest['name']}\n"
                f"Comment={manifest.get('description', '')}\n"
                f"Exec={bin_dest}\n"
                f"Icon={icon_name}\n"
                "Terminal=false\n"
                f"Categories={manifest.get('categories', 'Utility;')}\n"
            )

        _refresh_caches()

    return f"Installed {manifest['name']} {manifest['version']}"


def uninstall(args):
    pkg_id = args["id"]
    if not packaging.ID_RE.match(pkg_id):
        raise packaging.PackageError("invalid package id")

    removed = []

    bin_path = os.path.join(BIN_DIR, pkg_id)
    if os.path.isfile(bin_path):
        os.remove(bin_path)
        removed.append(bin_path)

    lib_path = os.path.join(LIB_ROOT, f"nexa-{pkg_id}")
    if os.path.isdir(lib_path):
        shutil.rmtree(lib_path)
        removed.append(lib_path)

    desktop_path = os.path.join(APPS_DIR, f"nexa-{pkg_id}.desktop")
    if os.path.isfile(desktop_path):
        os.remove(desktop_path)
        removed.append(desktop_path)

    for ext in (".svg", ".png"):
        icon_path = os.path.join(ICON_DIR, f"nexa-{pkg_id}{ext}")
        if os.path.isfile(icon_path):
            os.remove(icon_path)
            removed.append(icon_path)

    if not removed:
        raise packaging.PackageError(f"'{pkg_id}' is not installed (nothing to remove)")

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
