import re
import subprocess

PACKAGE_NAME_RE = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9+._-]*$")
# apk search/info print "<name>-<version>[-r<release>]" as one token; the
# version always starts right after a dash with a digit, so split there --
# package names themselves may also contain dashes and digits (py3-gobject3,
# gtk-layer-shell), so a plain rsplit("-") would cut them wrong.
_NAME_VERSION_RE = re.compile(r"^(?P<name>.+)-(?P<version>\d[^-]*(?:-r\d+)?)$")


class ApkError(Exception):
    pass


def is_valid_package_name(name):
    return bool(name) and len(name) <= 100 and bool(PACKAGE_NAME_RE.match(name))


def _run(argv, timeout=60):
    try:
        return subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except FileNotFoundError:
        raise ApkError(f"{argv[0]} is not installed")
    except subprocess.TimeoutExpired:
        raise ApkError(f"{argv[0]} timed out")


def _split_name_version(name_version):
    m = _NAME_VERSION_RE.match(name_version)
    if m:
        return m.group("name"), m.group("version")
    return name_version, ""


def update_index():
    result = _run(["apk", "update"], timeout=60)
    if result.returncode != 0:
        raise ApkError(result.stderr.strip() or "apk update failed")
    return result.stdout


def search(query):
    query = query.strip()
    if not query:
        return []
    result = _run(["apk", "search", "-v", query], timeout=30)
    if result.returncode != 0:
        raise ApkError(result.stderr.strip() or "apk search failed")
    packages = []
    for line in result.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        name_version, _, desc = line.partition(" - ")
        name, version = _split_name_version(name_version)
        packages.append({"name": name, "version": version, "description": desc})
    return packages


def list_installed():
    result = _run(["apk", "info"], timeout=15)
    if result.returncode != 0:
        raise ApkError(result.stderr.strip() or "apk info failed")
    return {line.strip() for line in result.stdout.splitlines() if line.strip()}


def install_package(name):
    if not is_valid_package_name(name):
        raise ApkError("invalid package name")
    result = _run(["apk", "add", name], timeout=120)
    if result.returncode != 0:
        raise ApkError(result.stderr.strip() or f"failed to install {name}")
    return result.stdout


def remove_package(name):
    if not is_valid_package_name(name):
        raise ApkError("invalid package name")
    result = _run(["apk", "del", name], timeout=60)
    if result.returncode != 0:
        raise ApkError(result.stderr.strip() or f"failed to remove {name}")
    return result.stdout
