"""Run with: python3 common/tests/test_diskinstall.py"""
import io, os, subprocess, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from nexa_common import diskinstall as di

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)
def raises(fn, *a):
    try: fn(*a)
    except di.InstallError as e: return str(e)
    return None

tmp = tempfile.mkdtemp()
sysroot, dev, procm = (os.path.join(tmp, x) for x in ("sys", "dev", "mounts"))
os.makedirs(os.path.join(sysroot, "class", "block")); os.makedirs(dev)

def add_disk(name, size_sectors=20*1024*1024, model="Test Disk", parted=False):
    d = os.path.join(sysroot, "devices", name, "block", name)
    os.makedirs(os.path.join(d, "device"))
    open(os.path.join(d, "size"), "w").write(str(size_sectors))
    open(os.path.join(d, "device", "model"), "w").write(model)
    os.symlink(d, os.path.join(sysroot, "class", "block", name))
    open(os.path.join(dev, name), "w").close()
    if parted:
        pd = os.path.join(d, name + "1"); os.makedirs(pd)
        open(os.path.join(pd, "partition"), "w").write("1")
        os.symlink(pd, os.path.join(sysroot, "class", "block", name + "1"))

add_disk("vda", size_sectors=4*1024*1024, model="QEMU HARDDISK")   # 2GB, too small, and the boot disk
add_disk("sdb", size_sectors=20*1024*1024, model="Install target") # 10GB, real target
add_disk("sdc", size_sectors=2*1024*1024, model="Tiny stick")      # 1GB, too small
add_disk("loop0", size_sectors=100, model="")
os.symlink(os.path.join(sysroot, "class", "block", "vda"), os.path.join(sysroot, "class", "block", "sr0"))  # not disk-shaped name, ignored anyway
open(procm, "w").write(f"{dev}/vda1 / ext4 rw 0 0\nproc /proc proc rw 0 0\n")

di.SYS, di.DEV, di.PROC_MOUNTS = sysroot, dev, procm
disks = {d["name"]: d for d in di.list_target_disks()}
check("boot disk (vda, backing /) excluded", "vda" not in disks, str(list(disks)))
check("loop devices excluded", "loop0" not in disks and "sr0" not in disks, str(list(disks)))
check("real target disk listed with size and model", disks.get("sdb") == {"name": "sdb", "size_bytes": 20*1024*1024*512, "model": "Install target", "large_enough": True}, str(disks.get("sdb")))
check("too-small disk flagged, not silently hidden", disks.get("sdc", {}).get("large_enough") is False)

check("validate accepts the real target", di.validate_disk("sdb")["name"] == "sdb")
check("validate rejects the boot disk", raises(di.validate_disk, "vda") is not None)
check("validate rejects a too-small disk", "too small" in (raises(di.validate_disk, "sdc") or ""))
check("validate rejects an unknown/hostile name", raises(di.validate_disk, "../../etc/passwd") is not None)

# Live session from a USB stick: "/" is an overlay, the stick is only visible
# at the live-medium mount (Rufus ISO mode: a partition; DD mode: whole disk).
for src, label in ((f"{dev}/sdb1", "ISO mode, partition"), (f"{dev}/sdb", "DD mode, whole disk")):
    open(procm, "w").write(f"overlay / overlay rw 0 0\n{src} {di.LIVE_MEDIUM_MOUNT} vfat ro 0 0\n")
    live = {d["name"] for d in di.list_target_disks()}
    check(f"live USB medium never offered ({label})", "sdb" not in live and "vda" in live, str(live))
# DD mode with persistence: the ISO is mounted through a loop device on the
# stick (so the stick's NEXA_PERSIST partition can be mounted as well).
os.makedirs(os.path.join(sysroot, "devices", "loop0", "block", "loop0", "loop"), exist_ok=True)
open(os.path.join(sysroot, "devices", "loop0", "block", "loop0", "loop", "backing_file"), "w").write("/dev/sdb\n")
open(procm, "w").write(f"overlay / overlay rw 0 0\n{dev}/loop0 {di.LIVE_MEDIUM_MOUNT} iso9660 ro 0 0\n")
live = {d["name"] for d in di.list_target_disks()}
check("live USB medium never offered (mounted through a loop device)", "sdb" not in live and "vda" in live, str(live))
open(procm, "w").write(f"{dev}/vda1 / ext4 rw 0 0\nproc /proc proc rw 0 0\n")

check("sdX partition naming", di.partition_path("sdb", 3) == f"{dev}/sdb3")
check("nvme partition naming gets the p separator", di.partition_path("nvme0n1", 2) == f"{dev}/nvme0n12p2".replace("12p2", "1p2"))

open(procm, "w").write(
    f"{dev}/vda1 / ext4 rw 0 0\n"
    "proc /proc proc rw 0 0\nsysfs /sys sysfs rw 0 0\ndevtmpfs /dev devtmpfs rw 0 0\n"
    "tmpfs /run tmpfs rw 0 0\ntmpfs /tmp tmpfs rw 0 0\n"
    f"{dev}/sdd1 /media/USBSTICK vfat rw 0 0\n"
    "iso9660 /mnt/cdrom iso9660 ro 0 0\n"
    f"{dev}/sdb3 /mnt/nexa-install ext4 rw 0 0\n"
    f"{dev}/sdb2 /mnt/nexa-install/boot/efi vfat rw 0 0\n"
)
patterns = di._clone_exclude_patterns("/mnt/nexa-install")
check("clone excludes pick up the live target mount and its own sub-mount (ESP)",
      "mnt" in patterns, str(patterns))
check("clone excludes standard pseudo-filesystems", {"proc", "sys", "dev", "run", "tmp"} <= set(patterns), str(patterns))
check("clone excludes other live media (USB stick under /media)", "media" in patterns)
check("no attempt to exclude / itself (that would exclude everything)", "" not in patterns and "." not in patterns)

grub_dir = tempfile.mkdtemp()
di.write_grub_cfg(grub_dir, "/dev/sdb3")
cfg = open(os.path.join(grub_dir, "boot", "grub", "grub.cfg")).read()
check("grub.cfg points at the real root partition and the installed kernel/initrd",
      "root=/dev/sdb3" in cfg and "linux /boot/vmlinuz" in cfg and "initrd /boot/initramfs.cpio.gz" in cfg, cfg)

idroot = tempfile.mkdtemp()
os.makedirs(os.path.join(idroot, "etc"))
os.makedirs(os.path.join(idroot, "home", "user"))
real_zi = tempfile.mkdtemp()
os.makedirs(os.path.join(real_zi, "Africa"))
open(os.path.join(real_zi, "Africa", "Accra"), "wb").write(b"TZif2" + b"\0"*20)
from nexa_common import sysconf
sysconf.ZONEINFO = real_zi
di.apply_identity(idroot, "my-laptop", "Africa/Accra", "gh")
check("hostname written", open(os.path.join(idroot, "etc", "hostname")).read() == "my-laptop\n")
check("timezone symlink points into the real zoneinfo tree", os.readlink(os.path.join(idroot, "etc", "localtime")) == os.path.join(real_zi, "Africa/Accra"))
check("timezone file written", open(os.path.join(idroot, "etc", "timezone")).read() == "Africa/Accra\n")
kb = os.path.join(idroot, "home", "user", ".config", "sway", "config.d", "10-nexa-keyboard.conf")
check("keyboard layout written for the target's own user", 'xkb_layout "gh"' in open(kb).read())

idroot2 = tempfile.mkdtemp(); os.makedirs(os.path.join(idroot2, "etc"))
di.apply_identity(idroot2, None, "Not/AZone", None)
check("garbage timezone is silently ignored, not written", not os.path.exists(os.path.join(idroot2, "etc", "localtime")))
check("no hostname given -> file not touched", not os.path.exists(os.path.join(idroot2, "etc", "hostname")))

calls = []
inputs = {}
def fake_run(argv, **kw):
    calls.append(argv)
    if kw.get("input") is not None:
        inputs[argv[0]] = kw["input"]
    if argv[0] == "wipefs" and "fail-partition" in os.environ.get("NEXA_TEST", ""):
        return subprocess.CompletedProcess(argv, 1, stdout="", stderr="boom")
    if argv[0] == "parted":
        # simulate the kernel creating partition device nodes, like the real
        # loopback test showed happens after parted + partprobe
        disk = argv[2]
        open(disk + "2", "w").close()
        open(disk + "3", "w").close()
    return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")
di._run = fake_run

class FakePopen:
    """Stands in for clone_system()'s two real tar processes so the test
    never shells out to an actual tar (or needs real files to copy)."""
    def __init__(self, argv, **kw):
        calls.append(argv)
        self.returncode = 0
        self.stdout = io.BytesIO()
    def wait(self, timeout=None):
        return self.returncode
di._popen = FakePopen
di.validate_disk = lambda name: {"name": name, "size_bytes": 99, "model": "x", "large_enough": True}

stages = []
os.environ["NEXA_TEST"] = "fail-partition"
try:
    di.run_install("sdb", on_stage=stages.append)
    check("failing partition step still raises", False)
except di.InstallError:
    check("failing partition step raises InstallError", True)
check("unmount still ran even though partitioning failed (cleanup on error)",
      any(c[:2] == ["umount", "-R"] for c in calls), str(calls))
check("stages reported before the failure are exactly partition (nothing later)", stages == ["partition", "unmount"], str(stages))

os.environ["NEXA_TEST"] = ""
calls.clear(); stages.clear()
di.list_target_disks = lambda: [{"name": "sdb", "size_bytes": 99, "model": "x", "large_enough": True}]
di.TARGET_ROOT = tempfile.mkdtemp()
di.BOOT_SRC = tempfile.mkdtemp()
os.makedirs(os.path.join(di.TARGET_ROOT, "etc"))  # what a real clone_system() would have populated
open(os.path.join(di.BOOT_SRC, "vmlinuz"), "w").write("fake kernel")
open(os.path.join(di.BOOT_SRC, "initramfs.cpio.gz"), "w").write("fake initrd")
result = di.run_install("sdb", hostname="h", zone=None, keyboard_layout=None, on_stage=stages.append)
check("happy path visits every stage in order",
      stages == list(di.STAGES), str(stages))
check("happy path result names the installed disk", result == {"installed_to": f"{di.DEV}/sdb"}, str(result))

check("no password given -> chpasswd never runs", not any(c[0] == "chpasswd" for c in calls), str(calls))

for bad, why in (("", "empty"), ("abc", "too short"), ("good\npass", "newline")):
    check(f"validate_password rejects {why}", raises(di.validate_password, bad) is not None)
check("validate_password accepts a normal password", raises(di.validate_password, "s3cret pass:x") is None)

calls.clear(); stages.clear(); inputs.clear()
result = di.run_install("sdb", hostname="h", on_stage=stages.append, password="hunter22")
chp = [c for c in calls if c[0] == "chpasswd"]
check("password given -> chpasswd runs once, SHA-512, chrooted into the target",
      chp == [["chpasswd", "-c", "sha512", "-R", di.TARGET_ROOT]], str(chp))
check("password goes to user and root via stdin",
      inputs.get("chpasswd") == "user:hunter22\nroot:hunter22\n", repr(inputs.get("chpasswd")))
check("password never appears in any argv", not any("hunter22" in a for c in calls for a in c))
check("password is set after the clone (else the clone would overwrite shadow)",
      calls.index(chp[0]) > max(i for i, c in enumerate(calls) if c[0] == "tar"), str(calls))
check("password set inside the identity stage, before unmount", stages[-2:] == ["identity", "unmount"], str(stages))

conf_root = tempfile.mkdtemp(); os.makedirs(os.path.join(conf_root, "etc"))
open(os.path.join(conf_root, "etc", "doas.conf"), "w").write(
    "permit nopass user cmd /usr/local/bin/nexa-power-helper\n"
    "permit nopass user cmd /usr/local/bin/nexa-software-helper\n"
    "permit nopass user cmd /usr/local/bin/nexa-install-helper\n"
    "permit nopass user cmd /usr/local/bin/nexa-ai-control-helper\n"
    "permit nopass user cmd /usr/local/bin/nexa-setup-helper\n")
real_chown = os.chown; os.chown = lambda *a: None
di.harden_privileges(conf_root)
os.chown = real_chown
rules = open(os.path.join(conf_root, "etc", "doas.conf")).read().splitlines()
check("installed system: software/package/assistant helpers need the password",
      "permit user cmd /usr/local/bin/nexa-software-helper" in rules
      and "permit user cmd /usr/local/bin/nexa-install-helper" in rules
      and "permit user cmd /usr/local/bin/nexa-ai-control-helper" in rules, str(rules))
check("installed system: power stays passwordless", "permit nopass user cmd /usr/local/bin/nexa-power-helper" in rules)
check("installed system: the disk-wiping installer helper is gone", not any("nexa-setup-helper" in r for r in rules))
check("doas.conf stays root-only (0400)", oct(os.stat(os.path.join(conf_root, "etc", "doas.conf")).st_mode & 0o777) == "0o400")

# ---- username
for bad, why in (("", "empty"), ("Kwame", "uppercase"), ("1abc", "starts with a digit"),
                 ("root", "reserved"), ("a b", "space"), ("x" * 40, "too long")):
    check(f"username rejected: {why}", raises(di.validate_username, bad) is not None)
check("username accepted: kwame-2", raises(di.validate_username, "kwame-2") is None)
r = tempfile.mkdtemp(); os.makedirs(os.path.join(r, "etc")); os.makedirs(os.path.join(r, "home", "user", ".config"))
open(os.path.join(r, "etc", "passwd"), "w").write("root:x:0:0:root:/root:/bin/sh\nuser:x:1000:1000:user:/home/user:/bin/sh\n")
open(os.path.join(r, "etc", "shadow"), "w").write("root::0:0:99999:7:::\nuser::0:0:99999:7:::\n")
os.chmod(os.path.join(r, "etc", "shadow"), 0o600)
open(os.path.join(r, "etc", "group"), "w").write("root:x:0:\nuser:x:1000:\nwheel:x:10:user\nvideo:x:27:user,other\n")
open(os.path.join(r, "etc", "doas.conf"), "w").write("permit nopass user cmd /usr/local/bin/nexa-power-helper\npermit user cmd /usr/local/bin/nexa-software-helper\n")
di.rename_user(r, "kwame")
passwd = open(os.path.join(r, "etc", "passwd")).read()
check("passwd: name, gecos and home renamed, uid stays 1000", "kwame:x:1000:1000:kwame:/home/kwame:/bin/sh" in passwd and "user:" not in passwd, passwd)
check("shadow renamed and still private", open(os.path.join(r, "etc", "shadow")).read().startswith("root::") and "\nkwame::" in open(os.path.join(r, "etc", "shadow")).read()
      and oct(os.stat(os.path.join(r, "etc", "shadow")).st_mode & 0o777) == "0o600")
group = open(os.path.join(r, "etc", "group")).read()
check("own group and memberships renamed", "kwame:x:1000:" in group and "wheel:x:10:kwame" in group and "video:x:27:kwame,other" in group, group)
doas = open(os.path.join(r, "etc", "doas.conf")).read()
check("doas rules follow the new name", "permit nopass kwame cmd /usr/local/bin/nexa-power-helper" in doas
      and "permit kwame cmd /usr/local/bin/nexa-software-helper" in doas, doas)
check("home directory moved", os.path.isdir(os.path.join(r, "home", "kwame", ".config")) and not os.path.exists(os.path.join(r, "home", "user")))
check("keeping the name 'user' changes nothing", di.rename_user(r, "user") is None)

calls.clear(); stages.clear(); inputs.clear()
di.run_install("sdb", hostname="h", on_stage=stages.append, password="hunter22", username="kwame")
check("install with a username sets that account's password (and root's)",
      inputs.get("chpasswd") == "kwame:hunter22\nroot:hunter22\n", repr(inputs.get("chpasswd")))
calls.clear(); stages.clear()
check("bad username is rejected before the disk is touched",
      raises(lambda: di.run_install("sdb", password="hunter22", username="Root!", on_stage=stages.append)) is not None
      and stages == [] and not any(c[0] in ("wipefs", "parted") for c in calls), f"{stages} {calls}")

calls.clear(); stages.clear()
check("bad password is rejected before the disk is touched",
      raises(lambda: di.run_install("sdb", password="ab", on_stage=stages.append)) is not None
      and stages == [] and not any(c[0] in ("wipefs", "parted") for c in calls), f"{stages} {calls}")

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
