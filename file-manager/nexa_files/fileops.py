"""Listing and display helpers. Changing files is nexa-fileops' job (engine.py)."""
import os
import stat as statmod
import time

TEXT_EXTS = {
    ".txt", ".md", ".log", ".conf", ".cfg", ".ini", ".c", ".h", ".py", ".sh",
    ".json", ".xml", ".js", ".css", ".yml", ".yaml", ".toml", ".rs", ".go",
}
IMAGE_EXTS = {".png", ".jpg", ".jpeg", ".gif", ".bmp", ".svg", ".webp"}
AUDIO_EXTS = {".mp3", ".wav", ".ogg", ".flac"}
VIDEO_EXTS = {".mp4", ".mkv", ".avi", ".webm"}
ARCHIVE_EXTS = {".tar", ".gz", ".bz2", ".xz", ".zip", ".7z", ".tgz"}
PDF_EXTS = {".pdf"}


class FileOpError(Exception):
    pass


def list_dir(path, show_hidden=False):
    try:
        names = os.listdir(path)
    except PermissionError:
        raise FileOpError("This folder belongs to the system; your account can't open it.")
    except OSError as e:
        raise FileOpError(str(e))
    entries = []
    for name in names:
        if not show_hidden and name.startswith("."):
            continue
        full = os.path.join(path, name)
        try:
            st = os.lstat(full)
        except OSError:
            continue
        is_dir = statmod.S_ISDIR(st.st_mode)
        # Folders this account can't open (/root, lost+found, ...) only led to
        # a permission error: shown only with hidden items (Ctrl+H).
        if is_dir and not show_hidden and not os.access(full, os.R_OK | os.X_OK):
            continue
        is_exec = bool(st.st_mode & statmod.S_IXUSR) and not is_dir
        entries.append({
            "name": name, "path": full, "is_dir": is_dir, "is_exec": is_exec,
            "size": st.st_size, "mtime": st.st_mtime,
        })
    entries.sort(key=lambda e: (not e["is_dir"], e["name"].lower()))
    return entries


def guess_icon(name, is_dir, is_exec):
    if is_dir:
        return "folder"
    ext = os.path.splitext(name)[1].lower()
    if ext in IMAGE_EXTS:
        return "image-x-generic"
    if ext in AUDIO_EXTS:
        return "audio-x-generic"
    if ext in VIDEO_EXTS:
        return "video-x-generic"
    if ext in ARCHIVE_EXTS:
        return "package-x-generic"
    if ext in PDF_EXTS:
        return "x-office-document"
    if ext in TEXT_EXTS:
        return "text-x-generic"
    if is_exec:
        return "application-x-executable"
    return "text-x-generic"


def human_size(n, is_dir):
    if is_dir:
        return ""
    size = float(n)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if size < 1024 or unit == "TB":
            return f"{size:.0f} {unit}" if unit == "B" else f"{size:.1f} {unit}"
        size /= 1024
    return f"{size:.1f} TB"


def human_time(ts):
    return time.strftime("%Y-%m-%d %H:%M", time.localtime(ts))
