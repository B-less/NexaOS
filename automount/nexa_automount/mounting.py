"""Mount/unmount/eject logic for removable (USB) drives. Runs as root, from
udev (auto/cleanup) or through `doas nexa-mount-helper` (list/mount/unmount/
eject). It never trusts its caller: every device name is re-validated against
the kernel's own sysfs view, only removable USB/SD drives are touched, and
mount points are confined to /media. All system paths are module attributes
so the logic can be tested against a fake tree."""
import os
import pwd
import re
import subprocess
import time

SYS = "/sys"
DEV = "/dev"
MEDIA = "/media"
PROC_MOUNTS = "/proc/mounts"

FS_TYPES = ("vfat", "exfat", "ntfs3", "ext4", "udf", "iso9660")
_KNAME_RE = re.compile(r"^(sd[a-z]+[0-9]*|mmcblk[0-9]+(p[0-9]+)?)$")
_DISK_RE = re.compile(r"^(sd[a-z]+|mmcblk[0-9]+)$")
_BLKID_TYPE = {"vfat": "vfat", "exfat": "exfat", "ntfs": "ntfs3", "ext4": "ext4",
               "ext3": "ext4", "ext2": "ext4", "udf": "udf", "iso9660": "iso9660"}

_run = subprocess.run
_sleep = time.sleep


class MountError(Exception):
    pass


def _log(message):
    try:
        _run(["logger", "-t", "nexa-mount", message], capture_output=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        pass


def _read(path):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return None


def _block_dir(kname):
    return os.path.join(SYS, "class", "block", kname)


def is_partition(kname):
    return os.path.exists(os.path.join(_block_dir(kname), "partition"))


def disk_of(kname):
    if is_partition(kname):
        return os.path.basename(os.path.dirname(os.path.realpath(_block_dir(kname))))
    return kname


def partitions_of(disk):
    d = os.path.realpath(_block_dir(disk))
    try:
        return sorted(e for e in os.listdir(d) if os.path.exists(os.path.join(d, e, "partition")))
    except OSError:
        return []


def eligible(kname):
    """Only removable disks and USB-attached ones -- never internal drives,
    virtio/NVMe/optical devices, or anything that isn't a plain disk name."""
    if not isinstance(kname, str) or not _KNAME_RE.match(kname):
        return False
    disk = disk_of(kname)
    if not _DISK_RE.match(disk):
        return False
    disk_dir = os.path.realpath(_block_dir(disk))
    if not os.path.isdir(disk_dir):
        return False
    return _read(os.path.join(disk_dir, "removable")) == "1" or "/usb" in disk_dir


def _mounts():
    entries = []
    try:
        with open(PROC_MOUNTS) as f:
            for line in f:
                parts = line.split()
                if len(parts) >= 3:
                    entries.append((parts[0], parts[1].replace("\\040", " "), parts[2]))
    except OSError:
        pass
    return entries


def mount_of(kname):
    dev = os.path.join(DEV, kname)
    for source, mountpoint, fstype in _mounts():
        if source == dev:
            return mountpoint, fstype
    return None


def _under_media(path):
    return path.startswith(MEDIA.rstrip("/") + "/")


def _blkid(kname):
    result = _run(["blkid", os.path.join(DEV, kname)], capture_output=True, text=True, timeout=10)
    label = re.search(r'LABEL="([^"]*)"', result.stdout)
    fstype = re.search(r'\bTYPE="([^"]*)"', result.stdout)
    return (label.group(1) if label else None), (fstype.group(1) if fstype else None)


def _safe_name(label, kname):
    cleaned = re.sub(r"[^A-Za-z0-9._-]", "_", label or "")[:32].strip("._")
    return cleaned or kname


def _user_ids():
    try:
        u = pwd.getpwuid(1000)  # the desktop account, whatever the installer named it
        return u.pw_uid, u.pw_gid
    except KeyError:
        return 1000, 1000


def _options(fstype):
    base = "nosuid,nodev,noexec,noatime"
    if fstype in ("vfat", "exfat", "ntfs3"):
        uid, gid = _user_ids()
        opts = f"{base},uid={uid},gid={gid},umask=022"
        return opts + ",flush" if fstype == "vfat" else opts
    return base


def _pick_mountpoint(label, kname):
    base = os.path.join(MEDIA, _safe_name(label, kname))
    taken = {mp for _s, mp, _t in _mounts()}
    for candidate in (base, f"{base}-{kname}"):
        if candidate not in taken:
            return candidate
    raise MountError("Couldn't pick a folder to mount this drive in.")


def mount(kname):
    if not eligible(kname):
        raise MountError("That isn't a removable drive.")
    if not is_partition(kname) and partitions_of(kname):
        raise MountError("This disk is partitioned - open one of its partitions instead.")
    dev = os.path.join(DEV, kname)
    existing = mount_of(kname)
    if existing:
        if _under_media(existing[0]):
            return {"mountpoint": existing[0], "fstype": existing[1]}
        raise MountError("This drive is already in use by the system.")

    label, detected = _blkid(kname)
    mountpoint = _pick_mountpoint(label, kname)
    os.makedirs(mountpoint, mode=0o755, exist_ok=True)
    first = _BLKID_TYPE.get(detected or "")
    order = ([first] if first else []) + [t for t in FS_TYPES if t != first]
    for fstype in order:
        result = _run(["mount", "-t", fstype, "-o", _options(fstype), dev, mountpoint],
                      capture_output=True, text=True, timeout=30)
        if result.returncode == 0:
            _log(f"mounted {dev} ({fstype}) at {mountpoint}")
            return {"mountpoint": mountpoint, "fstype": fstype}
    try:
        os.rmdir(mountpoint)
    except OSError:
        pass
    raise MountError("Couldn't open this drive (unsupported or damaged filesystem).")


def unmount(kname):
    if not isinstance(kname, str) or not _KNAME_RE.match(kname):
        raise MountError("That isn't a removable drive.")
    existing = mount_of(kname)
    if not existing:
        raise MountError("That drive isn't mounted.")
    mountpoint = existing[0]
    if not _under_media(mountpoint):
        raise MountError("That drive isn't one of the removable drives.")
    result = _run(["umount", mountpoint], capture_output=True, text=True, timeout=30)
    if result.returncode != 0:
        raise MountError("The drive is still in use - close anything using it and try again.")
    try:
        os.rmdir(mountpoint)
    except OSError:
        pass
    return {"unmounted": mountpoint}


def eject(kname):
    if not eligible(kname):
        raise MountError("That isn't a removable drive.")
    disk = disk_of(kname)
    for name in partitions_of(disk) + [disk]:
        if mount_of(name):
            unmount(name)
    _run(["sync"], timeout=30)
    delete = os.path.join(os.path.realpath(_block_dir(disk)), "device", "delete")
    if "/usb" in os.path.realpath(_block_dir(disk)) and os.path.exists(delete):
        with open(delete, "w") as f:
            f.write("1")
    return {"ejected": disk}


def cleanup(kname):
    """Called when a drive is unplugged: drop any leftover mount and folder."""
    if not isinstance(kname, str) or not _KNAME_RE.match(kname):
        return
    dev = os.path.join(DEV, kname)
    for source, mountpoint, _t in _mounts():
        if source == dev and _under_media(mountpoint):
            _run(["umount", "-l", mountpoint], capture_output=True, timeout=30)
            try:
                os.rmdir(mountpoint)
            except OSError:
                pass


def auto(kname, settle=1.0):
    """udev hot-plug entry point: never raises, never blocks udev for long."""
    try:
        if not eligible(kname):
            return {"skipped": "not removable"}
        if not is_partition(kname):
            # The kernel announces a disk before its partitions; give the
            # partition scan a moment so we don't grab a partitioned disk.
            _sleep(settle)
            if partitions_of(kname):
                return {"skipped": "has partitions"}
        return mount(kname)
    except MountError as e:
        _log(f"auto-mount of {kname} skipped: {e}")
        return {"skipped": str(e)}
    except (OSError, subprocess.SubprocessError) as e:
        _log(f"auto-mount of {kname} failed: {e}")
        return {"skipped": str(e)}


def list_volumes():
    volumes = []
    try:
        names = sorted(os.listdir(os.path.join(SYS, "class", "block")))
    except OSError:
        return volumes
    for kname in names:
        if not eligible(kname):
            continue
        if not is_partition(kname) and partitions_of(kname):
            continue
        try:
            label, fstype = _blkid(kname)
        except (OSError, subprocess.SubprocessError):
            label = fstype = None
        size = _read(os.path.join(os.path.realpath(_block_dir(kname)), "size"))
        mounted = mount_of(kname)
        volumes.append({
            "name": kname,
            "label": label or "",
            "size_bytes": int(size) * 512 if size and size.isdigit() else 0,
            "mountpoint": mounted[0] if mounted and _under_media(mounted[0]) else None,
            "fstype": fstype or "",
        })
    return volumes
