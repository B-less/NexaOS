"""Removable drives (USB sticks, SD cards) through the system broker
(nexa-brokerd, Rust): it re-checks every device against the kernel's view,
only touches removable drives, and mounts them under /media."""
from nexa_common import broker


class RemovableError(Exception):
    pass


def _call(action, name=None):
    try:
        return broker.request(action, {"name": name} if name is not None else {}, timeout=60)
    except broker.BrokerError as e:
        raise RemovableError(str(e))


def list_volumes():
    return _call("list_drives")["drives"]


def mount(name):
    return _call("mount_drive", name)


def unmount(name):
    return _call("unmount_drive", name)


def eject(name):
    return _call("eject_drive", name)


def human_size(n):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024


def display_name(volume):
    return volume["label"] or f"Drive ({human_size(volume['size_bytes'])})"
