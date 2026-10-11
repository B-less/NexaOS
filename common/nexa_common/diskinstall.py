"""Installs the running live system onto a real disk with its own GRUB
bootloader (BIOS + UEFI, GPT layout). Runs as root, from nexa-setup-helper.

Every command here was validated by hand against a real loopback disk image
before being written (parted GPT layout, mkfs, grub-install --target=i386-pc
and x86_64-efi --removable, and the tar-based clone's exclude list) -- this
performs a destructive operation on a real disk, so nothing here is guessed.

All system paths/commands are module attributes so this is testable with a
fake sysfs tree and a fake `_run`, the same pattern as automount/mounting.py.
"""
import os
import re
import subprocess
import time

SYS = "/sys"
DEV = "/dev"
PROC_MOUNTS = "/proc/mounts"
TARGET_ROOT = "/mnt/nexa-install"
# Where the live initramfs leaves the ISO/USB medium mounted (see
# build-initramfs.sh). In a live session "/" is an overlay, not a disk, so
# this is the only way to know which disk the session is running from.
LIVE_MEDIUM_MOUNT = "/mnt/nexa-live"
BOOT_SRC = "/boot"  # the running system's own kernel/initramfs, see build-disk-image.sh

MIN_DISK_BYTES = 6 * 1024**3  # headroom over the ~1GB rootfs for a real install
MIN_PASSWORD_LEN = 4
# The live image ships these with empty passwords; an install sets both to
# the password chosen in the installer, so the installed disk never has a
# passwordless login.
PASSWORD_ACCOUNTS = ("user", "root")
BIOS_GRUB_END_MIB = 2
ESP_END_MIB = 322  # 320MiB ESP -- generous for holding more than one kernel later
ESP_LABEL = "NEXABOOT"
ROOT_LABEL = "nexaos"

_DISK_RE = re.compile(r"^(sd[a-z]+|nvme[0-9]+n[0-9]+|vd[a-z]+)$")
_ALWAYS_EXCLUDE = ("proc", "sys", "dev", "run", "tmp", "mnt", "media")

_run = subprocess.run
_popen = subprocess.Popen


class InstallError(Exception):
    pass


def _read(path):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return None


def _sh(argv, timeout=60, check=True, input=None):
    result = _run(argv, capture_output=True, text=True, timeout=timeout, input=input)
    if check and result.returncode != 0:
        raise InstallError(f"`{' '.join(argv)}` failed: {result.stderr.strip() or result.stdout.strip()}")
    return result


def _mounts():
    entries = []
    try:
        with open(PROC_MOUNTS) as f:
            for line in f:
                parts = line.split()
                if len(parts) >= 2:
                    entries.append((parts[0], parts[1].replace("\\040", " ")))
    except OSError:
        pass
    return entries


def _disk_of(source):
    if not source or not source.startswith(f"{DEV}/"):
        return None
    kname = os.path.basename(source)
    return re.sub(r"p?\d+$", "", kname) if re.match(r"^nvme\d+n\d+p\d+$", kname) else re.sub(r"\d+$", "", kname)


def _boot_disks():
    """The disks the running session depends on -- the one backing "/" and,
    in a live session, the ISO/USB medium -- so they're never offered/wiped."""
    disks = set()
    for src, mp in _mounts():
        if mp in ("/", LIVE_MEDIUM_MOUNT):
            disk = _disk_of(src)
            if disk:
                disks.add(disk)
    return disks


def list_target_disks():
    """Real disks that are safe to offer as an install target: not the disk
    the live session booted from, not loop/optical/zram devices."""
    disks = []
    boot_disks = _boot_disks()
    try:
        names = sorted(os.listdir(os.path.join(SYS, "class", "block")))
    except OSError:
        return disks
    for name in names:
        if not _DISK_RE.match(name) or name in boot_disks:
            continue
        block_dir = os.path.realpath(os.path.join(SYS, "class", "block", name))
        if os.path.exists(os.path.join(block_dir, "partition")):
            continue  # a partition, not a whole disk
        size_sectors = _read(os.path.join(block_dir, "size"))
        if not size_sectors or not size_sectors.isdigit():
            continue
        size_bytes = int(size_sectors) * 512
        model = _read(os.path.join(block_dir, "device", "model")) or ""
        disks.append({
            "name": name, "size_bytes": size_bytes,
            "model": model.strip(), "large_enough": size_bytes >= MIN_DISK_BYTES,
        })
    return disks


def validate_disk(name):
    disks = {d["name"]: d for d in list_target_disks()}
    disk = disks.get(name)
    if disk is None:
        raise InstallError(f"'{name}' is not a valid install target.")
    if not disk["large_enough"]:
        raise InstallError(
            f"That disk is too small (need at least {MIN_DISK_BYTES // 1024**3}GB).")
    return disk


def partition_path(disk_name, index):
    # Kernel convention: a disk name ending in a digit (nvme0n1, loop0, ...)
    # needs a "p" separator before the partition number to stay unambiguous;
    # a letter-ending name (sda, vda, ...) does not.
    sep = "p" if disk_name and disk_name[-1].isdigit() else ""
    return f"{DEV}/{disk_name}{sep}{index}"


def partition_disk(disk_name):
    """Wipes `disk_name` and lays down GPT: a BIOS boot partition (for GRUB's
    core.img under BIOS+GPT), an EFI System Partition, and the root partition
    filling the rest. Irreversibly destroys all data on the disk."""
    validate_disk(disk_name)
    disk_path = f"{DEV}/{disk_name}"
    _sh(["wipefs", "-a", disk_path])
    _sh(["parted", "-s", disk_path,
         "mklabel", "gpt",
         "mkpart", "bios_grub", "1MiB", f"{BIOS_GRUB_END_MIB}MiB",
         "set", "1", "bios_grub", "on",
         "mkpart", "ESP", "fat32", f"{BIOS_GRUB_END_MIB}MiB", f"{ESP_END_MIB}MiB",
         "set", "2", "esp", "on",
         "mkpart", "root", "ext4", f"{ESP_END_MIB}MiB", "100%"])
    _sh(["partprobe", disk_path], check=False)
    time.sleep(1)
    esp, root = partition_path(disk_name, 2), partition_path(disk_name, 3)
    for _ in range(20):
        if os.path.exists(esp) and os.path.exists(root):
            break
        time.sleep(0.5)
    else:
        raise InstallError("The kernel didn't pick up the new partitions in time.")
    return {"esp": esp, "root": root}


def format_partitions(esp, root):
    _sh(["mkfs.vfat", "-F32", "-n", ESP_LABEL, esp])
    _sh(["mkfs.ext4", "-q", "-L", ROOT_LABEL, root])


def mount_target(root, esp, mountpoint=None):
    mountpoint = mountpoint or TARGET_ROOT
    os.makedirs(mountpoint, exist_ok=True)
    _sh(["mount", root, mountpoint])
    esp_mount = os.path.join(mountpoint, "boot", "efi")
    os.makedirs(esp_mount, exist_ok=True)
    _sh(["mount", esp, esp_mount])
    return esp_mount


def unmount_target(mountpoint=None):
    mountpoint = mountpoint or TARGET_ROOT
    _sh(["umount", "-R", mountpoint], check=False)


def _clone_exclude_patterns(mountpoint):
    """Every currently-mounted path under "/" (this mountpoint included) plus
    the standard pseudo-filesystems, as patterns relative to `tar -C /`.
    Computed fresh from /proc/mounts rather than a fixed list, so a mounted
    USB drive or the live ISO's own CD-ROM never gets walked into either --
    proved not to self-recurse by hand against a loopback target."""
    mp_norm = mountpoint.rstrip("/")
    patterns = set(_ALWAYS_EXCLUDE)
    for _src, mp in _mounts():
        if mp == "/" or not mp.startswith("/"):
            continue
        if mp == mp_norm or mp.startswith(mp_norm + "/"):
            patterns.add(mp.lstrip("/").split("/")[0])
            continue
        patterns.add(mp.lstrip("/"))
    return sorted(p for p in patterns if p)


def clone_system(mountpoint=None, exclude_file="/tmp/nexa-setup-exclude.txt"):
    mountpoint = mountpoint or TARGET_ROOT
    """Copies the running system's own filesystem onto the target. Safe by
    construction: every other currently-mounted filesystem (pseudo-fs, the
    target itself, any USB/CD media) is excluded, so this can never copy
    the target into itself or walk /proc's live (and enormous) content."""
    with open(exclude_file, "w") as f:
        f.write("\n".join(_clone_exclude_patterns(mountpoint)) + "\n")
    # A real kernel pipe between the two tar processes, not
    # _run(..., input=tar_out.stdout) -- that form (subprocess.run with
    # input=) fully buffers the *entire* source archive as a Python bytes
    # object before the extractor even starts, rather than letting the two
    # tars run concurrently through a pipe like the shell equivalent
    # `tar -cf - . | tar -xf -` this was meant to match. Found by hand: on
    # an install-test VM this made a ~2GB clone take 10+ minutes at
    # 2-3MB/s (and stall under memory pressure) instead of the tens of
    # MB/s a real streaming copy achieves.
    src = _popen(["tar", "-C", "/", "-X", exclude_file, "-cf", "-", "."],
                 stdout=subprocess.PIPE)
    dst = _popen(["tar", "-C", mountpoint, "-xf", "-"], stdin=src.stdout)
    src.stdout.close()  # this process's copy of the read end; dst now owns it
    dst.wait(timeout=1800)
    src.wait(timeout=1800)
    if src.returncode != 0 or dst.returncode != 0:
        raise InstallError("Copying the system files failed.")
    # tar's exclude patterns skip these paths entirely, contents AND the
    # directory entry itself -- correct, since they're live pseudo-
    # filesystems that must never be walked into, but it leaves the target
    # with no /proc, /dev, /run, /tmp, /mnt, /media directories at all, not
    # even empty ones. OpenRC's sysinit can't mount anything onto a
    # mountpoint that doesn't exist (confirmed by hand on an install-test
    # VM: "mount: /proc: mount point does not exist", "can't create
    # /dev/null: nonexistent directory", boot never got past sysinit).
    # Recreate them as plain empty dirs; the real filesystems mount over
    # them at boot.
    for name in _ALWAYS_EXCLUDE:
        os.makedirs(os.path.join(mountpoint, name), exist_ok=True)


def install_kernel(mountpoint=None):
    mountpoint = mountpoint or TARGET_ROOT
    os.makedirs(os.path.join(mountpoint, "boot"), exist_ok=True)
    for name in ("vmlinuz", "initramfs.cpio.gz"):
        src = os.path.join(BOOT_SRC, name)
        if not os.path.isfile(src):
            raise InstallError(f"{src} is missing from the running system -- can't install a bootable copy.")
        _sh(["cp", src, os.path.join(mountpoint, "boot", name)])


def write_grub_cfg(mountpoint, root_partition):
    grub_dir = os.path.join(mountpoint, "boot", "grub")
    os.makedirs(grub_dir, exist_ok=True)
    with open(os.path.join(grub_dir, "grub.cfg"), "w") as f:
        f.write(
            'set timeout=2\n'
            'set default=0\n\n'
            'menuentry "NexaOS" {\n'
            '    insmod part_gpt\n'
            '    insmod ext2\n'
            f'    linux /boot/vmlinuz root={root_partition} rootwait mac80211_hwsim.radios=0 console=tty0 console=ttyS0\n'
            '    initrd /boot/initramfs.cpio.gz\n'
            '}\n'
        )


def install_bootloader(disk_name, mountpoint, esp_mount):
    boot_dir = os.path.join(mountpoint, "boot")
    _sh(["grub-install",
         "--target=i386-pc", f"--boot-directory={boot_dir}",
         "--modules=part_gpt ext2 fat", f"{DEV}/{disk_name}"], timeout=120)
    _sh(["grub-install",
         "--target=x86_64-efi", f"--efi-directory={esp_mount}",
         f"--boot-directory={boot_dir}", "--removable",
         "--modules=part_gpt ext2 fat"], timeout=120)


def apply_identity(mountpoint, hostname, zone, keyboard_layout):
    from . import sysconf
    if hostname:
        with open(os.path.join(mountpoint, "etc", "hostname"), "w") as f:
            f.write(hostname + "\n")
    if zone and sysconf.valid_zone(zone):
        etc = os.path.join(mountpoint, "etc")
        target = os.path.join(etc, "localtime")
        if os.path.lexists(target):
            os.remove(target)
        os.symlink(os.path.join(sysconf.ZONEINFO, zone), target)
        with open(os.path.join(etc, "timezone"), "w") as f:
            f.write(zone + "\n")
    if keyboard_layout and sysconf.valid_layout(keyboard_layout):
        conf = os.path.join(mountpoint, "home", "user", ".config", "sway", "config.d", "10-nexa-keyboard.conf")
        os.makedirs(os.path.dirname(conf), exist_ok=True)
        with open(conf, "w") as f:
            f.write(f'input type:keyboard {{\n    xkb_layout "{keyboard_layout}"\n}}\n')
        try:
            uid_gid = os.stat(os.path.join(mountpoint, "home", "user")).st_uid
            os.chown(conf, uid_gid, uid_gid)
        except OSError:
            pass


STAGES = (
    "partition", "format", "mount", "kernel", "clone", "bootloader", "identity", "unmount",
)


def validate_password(password):
    if not password:
        raise InstallError("A password is required.")
    if len(password) < MIN_PASSWORD_LEN:
        raise InstallError(f"The password must be at least {MIN_PASSWORD_LEN} characters.")
    # chpasswd reads one "name:password" per line, so a line break would
    # split the entry.
    if any(c in password for c in "\n\r\0"):
        raise InstallError("The password can't contain line breaks.")


def set_password(mountpoint, password):
    validate_password(password)
    # Fed on stdin, never argv, so the password doesn't show up in ps.
    # -R makes chpasswd chroot into the installed system and edit its
    # /etc/shadow (SHA-512 hashes), not the live session's.
    lines = "".join(f"{account}:{password}\n" for account in PASSWORD_ACCOUNTS)
    _sh(["chpasswd", "-c", "sha512", "-R", mountpoint], input=lines)


def run_install(disk_name, hostname=None, zone=None, keyboard_layout=None, on_stage=None,
                password=None):
    """Runs every step in order, always trying to unmount on failure so a
    half-finished install doesn't leave the target disk's filesystems busy.
    password=None leaves the cloned accounts' passwords as they were."""
    def report(stage):
        if on_stage:
            on_stage(stage)

    validate_disk(disk_name)
    if password is not None:
        validate_password(password)  # before anything is wiped
    mountpoint = TARGET_ROOT
    try:
        report("partition")
        parts = partition_disk(disk_name)
        report("format")
        format_partitions(parts["esp"], parts["root"])
        report("mount")
        esp_mount = mount_target(parts["root"], parts["esp"], mountpoint)
        report("kernel")
        install_kernel(mountpoint)
        write_grub_cfg(mountpoint, parts["root"])
        report("clone")
        clone_system(mountpoint)
        report("bootloader")
        install_bootloader(disk_name, mountpoint, esp_mount)
        report("identity")
        apply_identity(mountpoint, hostname, zone, keyboard_layout)
        if password is not None:
            set_password(mountpoint, password)
    finally:
        report("unmount")
        unmount_target(mountpoint)
    return {"installed_to": f"{DEV}/{disk_name}"}
