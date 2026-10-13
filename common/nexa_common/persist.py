"""Persistent live USB: keep files and settings on the stick between boots.

The hybrid ISO, written to a USB stick in DD mode, uses only the start of
the stick. create() adds partition 3, labelled NEXA_PERSIST, in the empty
space after it. On the next boot the initramfs finds that partition on the
same stick and keeps system changes and /home there instead of in RAM
(see scripts/build-initramfs.sh). Booting with `nexa.nopersist` ignores it.

Layout of the partition:
    home/              your files; kept when a newer NexaOS is written
    system-<id>/upper  changes to the system (apps, settings, Wi-Fi...);
    system-<id>/work   <id> identifies the NexaOS image, so a newer ISO
                       starts with a clean system layer
status() reads only /proc and /sys, so it works without root; create()
needs root (Settings helper).
"""
import os
import subprocess

LABEL = "NEXA_PERSIST"
LIVE_MOUNT = "/mnt/nexa-live"       # the boot medium (set up by the initramfs)
PERSIST_MOUNT = "/mnt/nexa-persist"  # the persistence partition, when in use
SECTOR = 512
ALIGN = 2048                         # 1 MiB, the usual partition alignment
MIN_BYTES = 1024 ** 3                # not worth it below 1 GiB
SYS = "/sys/class/block"
PROC_MOUNTS = "/proc/mounts"


class PersistError(Exception):
    pass


def _mounts():
    try:
        with open(PROC_MOUNTS) as f:
            return [line.split()[:3] for line in f if len(line.split()) >= 3]
    except OSError:
        return []


def _read_int(path):
    with open(path) as f:
        return int(f.read().strip())


def live_device():
    """The device the live system booted from ('sdb' or 'sdb1'), or None.
    A loop device (the initramfs mounts a whole-disk ISO through one) is
    followed to the stick behind it."""
    for src, mp, _fs in _mounts():
        if mp == LIVE_MOUNT and src.startswith("/dev/"):
            dev = os.path.basename(src)
            if dev.startswith("loop"):
                try:
                    with open(os.path.join(SYS, dev, "loop", "backing_file")) as f:
                        backing = f.read().strip()
                except OSError:
                    return None
                return os.path.basename(backing) if backing.startswith("/dev/") else None
            return dev
    return None


def disk_of(dev):
    """'sdb1' -> 'sdb', 'nvme0n1p1' -> 'nvme0n1'; a whole disk maps to itself."""
    if os.path.exists(os.path.join(SYS, dev, "partition")):
        return os.path.basename(os.path.dirname(os.path.realpath(os.path.join(SYS, dev))))
    return dev


def partitions(disk):
    """[(number, start_sector, size_sectors)] of the disk's partitions."""
    out = []
    base = os.path.join(SYS, disk)
    for name in sorted(os.listdir(base)) if os.path.isdir(base) else []:
        p = os.path.join(base, name)
        if os.path.exists(os.path.join(p, "partition")):
            out.append((_read_int(os.path.join(p, "partition")), _read_int(os.path.join(p, "start")),
                        _read_int(os.path.join(p, "size"))))
    return out


def plan(disk_sectors, iso_sectors, parts):
    """Where the new partition goes: (number, start, size) in sectors.
    It starts after the ISO and every existing partition, 1 MiB aligned,
    and runs to the end of the stick. Raises PersistError if it can't."""
    used = {n for n, _s, _l in parts}
    if 3 in used or len(used) >= 4:
        raise PersistError("This USB stick has no free partition slot.")
    end = max([iso_sectors] + [s + l for _n, s, l in parts])
    start = -(-end // ALIGN) * ALIGN
    size = disk_sectors - start
    if size * SECTOR < MIN_BYTES:
        free_gb = max(size, 0) * SECTOR / 1024 ** 3
        raise PersistError(f"Only {free_gb:.1f} GB is free after NexaOS on this stick; "
                           "at least 1 GB is needed. Use a bigger stick.")
    return 3, start, size


def status():
    """What Settings shows: {live, active, possible, reason, free_gb}."""
    dev = live_device()
    if dev is None:
        return {"live": False, "active": False, "possible": False, "reason": "Not running from a live USB.",
                "free_gb": 0}
    active = any(mp == PERSIST_MOUNT for _s, mp, _f in _mounts())
    if active:
        return {"live": True, "active": True, "possible": False, "reason": "", "free_gb": 0}
    disk = disk_of(dev)
    if disk != dev:
        return {"live": True, "active": False, "possible": False, "free_gb": 0,
                "reason": "This stick was written in Rufus's ISO mode, which uses the whole stick. "
                          "Write NexaOS again in DD Image mode to keep files on it."}
    if disk.startswith("sr"):
        return {"live": True, "active": False, "possible": False, "free_gb": 0,
                "reason": "Running from a DVD, which can't store files."}
    try:
        _n, _start, size = plan(_read_int(os.path.join(SYS, disk, "size")), iso_sectors(disk), partitions(disk))
    except (PersistError, OSError) as e:
        return {"live": True, "active": False, "possible": False, "reason": str(e), "free_gb": 0}
    return {"live": True, "active": False, "possible": True, "reason": "",
            "free_gb": round(size * SECTOR / 1024 ** 3, 1)}


def iso_sectors(disk):
    """Size of the ISO at the start of the stick, from its iso9660 header
    (volume space size, little-endian at offset 0x8050, in 2048-byte blocks).
    Needs read access to the disk; falls back to the medium's mount size."""
    try:
        with open(f"/dev/{disk}", "rb") as f:
            f.seek(0x8050)
            blocks = int.from_bytes(f.read(4), "little")
        if blocks:
            return blocks * 4
    except OSError:
        pass
    st = os.statvfs(LIVE_MOUNT)
    return st.f_blocks * st.f_frsize // SECTOR


def _wait_for_partition(pname, start, seconds=5):
    """True once the kernel lists the partition at `start` and /dev has it."""
    import time
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            if _read_int(os.path.join(SYS, pname, "start")) == start and os.path.exists(f"/dev/{pname}"):
                return True
        except OSError:
            pass
        time.sleep(0.25)
    return False


def _run(argv, **kw):
    r = subprocess.run(argv, capture_output=True, text=True, **kw)
    if r.returncode != 0:
        raise PersistError(f"{os.path.basename(argv[0])} failed: {(r.stderr or r.stdout).strip()[-300:]}")
    return r.stdout


def create(copy_home="/home"):
    """Make the persistence partition (root only). Copies the current
    session's /home onto it, so nothing done so far is lost."""
    st = status()
    if not st["possible"]:
        raise PersistError(st["reason"] or "Already keeping files on this USB.")
    disk = live_device()
    number, start, size = plan(_read_int(os.path.join(SYS, disk, "size")), iso_sectors(disk), partitions(disk))
    dev = f"/dev/{disk}"
    # The stick is in use, so the kernel can't re-read the whole table:
    # write the entry, then tell the kernel about just this partition.
    # -N pins the slot: the ISO's own entry 1 has type 0, which a plain
    # --append could otherwise take for a free slot (checked: it doesn't).
    _run(["sfdisk", "-N", str(number), "--no-reread", "--no-tell-kernel", dev],
         input=f"start={start}, size={size}, type=83\n")
    pname = f"{disk}{number}" if not disk[-1].isdigit() else f"{disk}p{number}"
    part = f"/dev/{pname}"
    # udev often re-reads the table by itself once sfdisk closes the disk
    # (it can, now that the medium is mounted through a loop device), and
    # then the kernel already has the partition: only add it if it doesn't.
    if not _wait_for_partition(pname, start):
        _run(["addpart", dev, str(number), str(start), str(size)])
        if not _wait_for_partition(pname, start):
            raise PersistError(f"The new partition {part} didn't appear.")
    _run(["mkfs.ext4", "-q", "-F", "-L", LABEL, "-m", "0", part])
    target = "/run/nexa-persist-setup"
    os.makedirs(target, exist_ok=True)
    _run(["mount", "-t", "ext4", part, target])
    try:
        os.makedirs(os.path.join(target, "home"), exist_ok=True)
        if copy_home and os.path.isdir(copy_home):
            _run(["cp", "-a", copy_home.rstrip("/") + "/.", os.path.join(target, "home")])
    finally:
        subprocess.run(["sync"])
        subprocess.run(["umount", target])
    return {"device": part, "size_gb": round(size * SECTOR / 1024 ** 3, 1)}
