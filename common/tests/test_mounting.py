"""Run with: python3 common/tests/test_mounting.py"""
import io
import json
import os
import subprocess
import sys
import tempfile
from contextlib import redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "automount"))
from nexa_automount import mounting, helper

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


def raises(fn, *a):
    try:
        fn(*a)
    except mounting.MountError as e:
        return str(e)
    return None


tmp = tempfile.mkdtemp()
sysroot, dev, media, procm = (os.path.join(tmp, x) for x in ("sys", "dev", "media", "mounts"))
for d in (dev, media, os.path.join(sysroot, "class", "block")):
    os.makedirs(d)
open(procm, "w").close()


def add_disk(name, path, removable, parts=(), size=2048):
    d = os.path.join(sysroot, "devices", path, "block", name)
    os.makedirs(os.path.join(d, "device"))
    open(os.path.join(d, "removable"), "w").write("1" if removable else "0")
    open(os.path.join(d, "size"), "w").write(str(size))
    open(os.path.join(d, "device", "delete"), "w").write("")
    os.symlink(d, os.path.join(sysroot, "class", "block", name))
    open(os.path.join(dev, name), "w").close()
    for p in parts:
        pd = os.path.join(d, p)
        os.makedirs(pd)
        open(os.path.join(pd, "partition"), "w").write("1")
        open(os.path.join(pd, "size"), "w").write(str(size // 2))
        os.symlink(pd, os.path.join(sysroot, "class", "block", p))
        open(os.path.join(dev, p), "w").close()


add_disk("sdb", "pci0/usb1/1-1", True, parts=("sdb1",))     # USB stick with a partition
add_disk("sdc", "pci0/usb1/1-2", False)                     # USB disk, no partition table, removable=0
add_disk("sda", "pci0/ata1", False, parts=("sda1",))        # internal SATA disk
add_disk("vda", "pci0/virtio0", False, parts=("vda1",))     # virtio root disk

mounting.SYS, mounting.DEV, mounting.MEDIA, mounting.PROC_MOUNTS = sysroot, dev, media, procm
mounting._sleep = lambda s: None
calls = []
state = {"fail": set(), "blkid": 'LABEL="MY DRIVE" TYPE="vfat"'}


def fake_run(argv, **kw):
    calls.append(argv)
    if argv[0] == "blkid":
        return subprocess.CompletedProcess(argv, 0, stdout=state["blkid"] + "\n", stderr="")
    if argv[0] == "mount":
        fs, target, source = argv[2], argv[-1], argv[-1 - 1]
        if fs in state["fail"]:
            return subprocess.CompletedProcess(argv, 32, stdout="", stderr="bad fs")
        open(procm, "a").write(f"{source} {target} {fs} rw 0 0\n")
        return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")
    if argv[0] == "umount":
        target = argv[-1]
        lines = [l for l in open(procm) if f" {target} " not in l]
        open(procm, "w").writelines(lines)
        return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")
    return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")


mounting._run = fake_run

check("USB partition and disk are eligible", mounting.eligible("sdb1") and mounting.eligible("sdb"))
check("USB disk with removable=0 still eligible (path is under /usb)", mounting.eligible("sdc"))
check("internal SATA and virtio disks are not", not mounting.eligible("sda") and not mounting.eligible("sda1") and not mounting.eligible("vda1"))
check("hostile names rejected", not any(mounting.eligible(n) for n in ("../sdb1", "sdb1; rm -rf /", "sdb1/../sda1", "nvme0n1p1", "", None)))

r = mounting.mount("sdb1")
check("mounts at /media/<label>, using the detected fs first", r == {"mountpoint": os.path.join(media, "MY_DRIVE"), "fstype": "vfat"}, str(r))
mcall = next(c for c in calls if c[0] == "mount")
check("mount options are nosuid,nodev,noexec and give the desktop user ownership",
      all(o in mcall[4] for o in ("nosuid", "nodev", "noexec", "uid=", "gid=", "umask=022")), mcall[4])
check("mount point folder was created", os.path.isdir(os.path.join(media, "MY_DRIVE")))
check("mounting an already-mounted drive is idempotent", mounting.mount("sdb1")["mountpoint"] == r["mountpoint"])

msg = raises(mounting.mount, "sdb")
check("partitioned disk refused (mount a partition instead)", msg and "partition" in msg, str(msg))
check("internal disk refused", raises(mounting.mount, "sda1") == "That isn't a removable drive.")

open(procm, "a").write(f"{dev}/sdc /mnt/cdrom iso9660 ro 0 0\n")
check("drive mounted outside /media is refused", "in use by the system" in (raises(mounting.mount, "sdc") or ""))
check("unmount refuses mounts outside /media", raises(mounting.unmount, "sdc") == "That drive isn't one of the removable drives.")
kept = open(procm).read().replace(f"{dev}/sdc /mnt/cdrom iso9660 ro 0 0\n", "")
open(procm, "w").write(kept)

state["blkid"] = ""; state["fail"] = {"vfat", "exfat"}; calls.clear()
r2 = mounting.mount("sdc")
tried = [c[2] for c in calls if c[0] == "mount"]
check("unknown fs: tries types in order until one works", tried == ["vfat", "exfat", "ntfs3"] and r2["fstype"] == "ntfs3", str(tried))
check("label-less drive is mounted under its device name", r2["mountpoint"] == os.path.join(media, "sdc"), r2["mountpoint"])

state["blkid"] = 'LABEL="MY DRIVE" TYPE="vfat"'; state["fail"] = set()
add_disk("sdd", "pci0/usb1/1-3", True, parts=("sdd1",))
r3 = mounting.mount("sdd1")
check("same label on a second drive gets a unique folder", r3["mountpoint"] == os.path.join(media, "MY_DRIVE-sdd1"), r3["mountpoint"])

state["fail"] = set(mounting.FS_TYPES)
add_disk("sde", "pci0/usb1/1-4", True, parts=("sde1",))
msg = raises(mounting.mount, "sde1")
check("nothing mounts -> clear error and no leftover folder", msg and "unsupported or damaged" in msg and not os.path.exists(os.path.join(media, "MY_DRIVE-sde1")), str(msg))
state["fail"] = set()

vols = mounting.list_volumes()
names = [v["name"] for v in vols]
check("list shows removable volumes only, never a disk that has partitions",
      names == ["sdb1", "sdc", "sdd1", "sde1"], str(names))
sdb1 = next(v for v in vols if v["name"] == "sdb1")
check("list reports label, size and mount point", sdb1["label"] == "MY DRIVE" and sdb1["size_bytes"] == 1024 * 512 and sdb1["mountpoint"] == r["mountpoint"], str(sdb1))

u = mounting.unmount("sdb1")
check("unmount works and removes the folder", u == {"unmounted": r["mountpoint"]} and not os.path.exists(r["mountpoint"]), str(u))
check("unmounting a drive that isn't mounted fails cleanly", "isn't mounted" in (raises(mounting.unmount, "sdb1") or ""))

mounting.mount("sdd1")
e = mounting.eject("sdd1")
check("eject unmounts every volume and powers the USB device off",
      e == {"ejected": "sdd"} and not mounting.mount_of("sdd1") and open(os.path.join(sysroot, "class", "block", "sdd", "device", "delete")).read() == "1", str(e))

mounting.mount("sdc")
calls.clear()
mounting.cleanup("sdc")   # simulates udev "remove" while the mount is still listed
check("cleanup lazily unmounts a yanked drive's leftover mount", any(c[:2] == ["umount", "-l"] for c in calls), str(calls))
check("cleanup ignores hostile names", mounting.cleanup("../../etc") is None)

check("auto: partition of a USB drive gets mounted", "mountpoint" in mounting.auto("sdb1"))
check("auto: disk that has partitions is skipped", mounting.auto("sdb") == {"skipped": "has partitions"})
check("auto: internal disk is skipped", mounting.auto("sda1") == {"skipped": "not removable"})
state["fail"] = set(mounting.FS_TYPES)
check("auto: unreadable filesystem never raises (udev must not fail)", "skipped" in mounting.auto("sde1"))
state["fail"] = set()

buf = io.StringIO()
with redirect_stdout(buf):
    rc = helper.main(["nexa-mount-helper", "auto", "sdb1"])
check("helper argv mode (udev) exits 0", rc == 0)
sys.stdin = io.StringIO("not json")
buf = io.StringIO()
with redirect_stdout(buf):
    rc = helper.main(["nexa-mount-helper"])
check("helper rejects malformed JSON", rc == 1 and json.loads(buf.getvalue())["ok"] is False)
sys.stdin = io.StringIO(json.dumps({"action": "rm -rf", "name": "sdb1"}))
buf = io.StringIO()
with redirect_stdout(buf):
    rc = helper.main(["nexa-mount-helper"])
check("helper rejects unknown actions", rc == 1 and json.loads(buf.getvalue())["ok"] is False)
sys.stdin = io.StringIO(json.dumps({"action": "mount", "name": "vda1"}))
buf = io.StringIO()
with redirect_stdout(buf):
    rc = helper.main(["nexa-mount-helper"])
out = json.loads(buf.getvalue())
check("helper refuses to mount a non-removable device", rc == 1 and not out["ok"] and "removable" in out["error"], buf.getvalue())

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
