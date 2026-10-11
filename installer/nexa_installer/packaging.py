import json
import os
import re
import zipfile

ID_RE = re.compile(r"^[a-z][a-z0-9_-]*$")
REQUIRED_FIELDS = ("id", "name", "version")


class PackageError(Exception):
    pass


def read_manifest(nexa_path):
    try:
        with zipfile.ZipFile(nexa_path) as zf:
            with zf.open("manifest.json") as f:
                manifest = json.load(f)
    except (zipfile.BadZipFile, KeyError, json.JSONDecodeError, OSError) as e:
        raise PackageError(f"Not a valid .nexa package: {e}")

    for field in REQUIRED_FIELDS:
        if not manifest.get(field):
            raise PackageError(f"manifest.json is missing required field '{field}'")
    if not ID_RE.match(manifest["id"]):
        raise PackageError(
            f"Invalid package id '{manifest['id']}' (must be lowercase letters, "
            "digits, '-', '_', starting with a letter)"
        )
    return manifest


def extract_icon(nexa_path, manifest, dest_path):
    """Extracts the icon named in the manifest (if any) to dest_path.
    Returns True if an icon was found and written."""
    icon_name = manifest.get("icon")
    if not icon_name:
        return False
    try:
        with zipfile.ZipFile(nexa_path) as zf:
            data = zf.read(icon_name)
    except (KeyError, zipfile.BadZipFile, OSError):
        return False
    with open(dest_path, "wb") as f:
        f.write(data)
    return True


def extract_payload(nexa_path, dest_dir):
    """Extracts the whole archive to dest_dir (a fresh temp dir the caller
    owns) and returns the manifest. Raises PackageError if bin/<id> --
    the one file every package must have -- is missing."""
    manifest = read_manifest(nexa_path)
    with zipfile.ZipFile(nexa_path) as zf:
        for info in zf.infolist():
            # Reject absolute paths and ".." components before extracting
            # anything -- zipfile.extractall() on an untrusted archive could
            # otherwise write outside dest_dir ("zip slip").
            normalized = os.path.normpath(info.filename)
            if normalized.startswith("..") or os.path.isabs(normalized):
                raise PackageError(f"Unsafe path in package: {info.filename}")
        zf.extractall(dest_dir)

    bin_path = os.path.join(dest_dir, "bin", manifest["id"])
    if not os.path.isfile(bin_path):
        raise PackageError(f"Package is missing its entry point: bin/{manifest['id']}")
    return manifest
