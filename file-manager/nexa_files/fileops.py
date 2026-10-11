import os
import shutil
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


def make_dir(parent, name):
    path = os.path.join(parent, name)
    try:
        os.mkdir(path)
    except OSError as e:
        raise FileOpError(str(e))
    return path


def rename(path, new_name):
    new_path = os.path.join(os.path.dirname(path), new_name)
    try:
        os.rename(path, new_path)
    except OSError as e:
        raise FileOpError(str(e))
    return new_path


def delete(path):
    try:
        if os.path.isdir(path) and not os.path.islink(path):
            shutil.rmtree(path)
        else:
            os.remove(path)
    except OSError as e:
        raise FileOpError(str(e))


def copy_into(src, dest_dir):
    name = os.path.basename(src.rstrip("/"))
    dest = os.path.join(dest_dir, name)
    src_abs = os.path.abspath(src)
    dest_dir_abs = os.path.abspath(dest_dir)
    if dest_dir_abs == src_abs or dest_dir_abs.startswith(src_abs + os.sep):
        raise FileOpError("can't copy a folder into itself")
    if os.path.abspath(dest) == src_abs:
        raise FileOpError("source and destination are the same")
    try:
        if os.path.isdir(src) and not os.path.islink(src):
            shutil.copytree(src, dest)
        else:
            shutil.copy2(src, dest)
    except (OSError, shutil.Error) as e:
        raise FileOpError(str(e))
    return dest
