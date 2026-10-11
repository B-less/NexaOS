import os
import subprocess

TAR_EXTS = (".tar", ".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tar.xz", ".txz")


class ArchiveError(Exception):
    pass


def _detect_kind(path):
    lower = path.lower()
    if lower.endswith(".zip"):
        return "zip"
    if lower.endswith(TAR_EXTS):
        return "tar"
    raise ArchiveError(f"Unsupported archive type: {os.path.basename(path)}")


def list_contents(path):
    kind = _detect_kind(path)
    if kind == "zip":
        return _parse_zip_listing(_run(["unzip", "-l", path]))
    return _parse_tar_listing(_run(["tar", "-tvf", path]))


def extract_all(path, dest_dir):
    kind = _detect_kind(path)
    os.makedirs(dest_dir, exist_ok=True)
    if kind == "zip":
        _run(["unzip", "-o", path, "-d", dest_dir])
    else:
        _run(["tar", "-xf", path, "-C", dest_dir])


def _run(argv):
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=60)
    except FileNotFoundError:
        raise ArchiveError(f"{argv[0]} is not installed")
    except subprocess.TimeoutExpired:
        raise ArchiveError(f"{argv[0]} timed out")
    if result.returncode != 0:
        raise ArchiveError(result.stderr.strip() or f"{argv[0]} failed")
    return result.stdout


def _parse_zip_listing(text):
    # unzip -l output brackets the entries between two "---------" rules:
    #   Length      Date    Time    Name
    # ---------  ---------- -----   ----
    #      1234  01-01-2024 00:00   file.txt
    # ---------                     -------
    entries = []
    in_body = False
    for line in text.splitlines():
        if line.strip().startswith("---"):
            if in_body:
                break
            in_body = True
            continue
        if not in_body:
            continue
        parts = line.split(None, 3)
        if len(parts) < 4 or not parts[0].isdigit():
            continue
        size_str, _date, _time, name = parts
        entries.append({"name": name, "size": int(size_str), "is_dir": name.endswith("/")})
    return entries


def _parse_tar_listing(text):
    # busybox/GNU tar -tv: "-rw-r--r-- user/group  1234 2024-01-01 00:00 path"
    entries = []
    for line in text.splitlines():
        parts = line.split(None, 5)
        if len(parts) < 6 or not parts[2].isdigit():
            continue
        mode, _owner, size_str, _date, _time, name = parts
        entries.append({"name": name, "size": int(size_str), "is_dir": mode.startswith("d")})
    return entries
