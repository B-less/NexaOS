"""Run with: python3 common/tests/test_persist.py

Persistent live USB planning (nexa_common.persist) against a fake /sys and
/proc/mounts. The real partitioning is checked end to end in QEMU by
scripts/test-persistence.py."""
import os, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from nexa_common import persist

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

GB = 1024 ** 3 // 512          # sectors per GiB
ISO = 4316928                  # the 3.2.0 ISO, in sectors

# ---- plan()
n, start, size = persist.plan(8 * GB, ISO, [(2, 156, 4056)])
check("plan: partition 3 right after the ISO, 1 MiB aligned", n == 3 and start == 4317184 and start % 2048 == 0, str(start))
check("plan: runs to the end of the stick", start + size == 8 * GB)
for label, args, msg in (("stick too small", (ISO + GB // 2, ISO, []), "at least 1 GB"),
                         ("slot 3 already used", (8 * GB, ISO, [(3, ISO, 100)]), "no free partition slot"),
                         ("table full", (8 * GB, ISO, [(1, 0, 1), (2, 1, 1), (4, 9, 1), (5, 99, 1)]), "no free partition slot")):
    try:
        persist.plan(*args)
        check(f"plan refuses: {label}", False)
    except persist.PersistError as e:
        check(f"plan refuses: {label}", msg in str(e), str(e))
n, start, size = persist.plan(8 * GB, ISO, [(2, ISO + 10, 4000)])
check("plan: starts after a partition that ends past the ISO", start >= ISO + 4010)

# ---- status() on a fake system
root = tempfile.mkdtemp()
persist.SYS = os.path.join(root, "sys")
persist.PROC_MOUNTS = os.path.join(root, "mounts")
def fake_disk(name, size, parts):
    d = os.path.join(persist.SYS, name)
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, "size"), "w").write(str(size))
    for num, s, l in parts:
        p = os.path.join(d, f"{name}{num}")
        os.makedirs(p, exist_ok=True)
        for f, v in (("partition", num), ("start", s), ("size", l)):
            open(os.path.join(p, f), "w").write(str(v))
        link = os.path.join(persist.SYS, f"{name}{num}")
        if not os.path.exists(link):
            os.symlink(p, link)
def mounts(*lines):
    open(persist.PROC_MOUNTS, "w").write("".join(l + " 0 0\n" for l in lines))
persist.iso_sectors = lambda disk: ISO  # (reads the iso9660 header on a real stick)

mounts("overlay / overlay rw")
check("status: installed system is not live", persist.status()["live"] is False)

fake_disk("sdb", 8 * GB, [(2, 156, 4056)])
mounts("overlay / overlay rw", "/dev/sdb /mnt/nexa-live iso9660 ro")
st = persist.status()
check("status: DD-mode stick with free space can be made persistent",
      st["live"] and st["possible"] and not st["active"] and 5.5 < st["free_gb"] < 6.0, str(st))

mounts("overlay / overlay rw", "/dev/sdb /mnt/nexa-live iso9660 ro", "/dev/sdb3 /mnt/nexa-persist ext4 rw")
st = persist.status()
check("status: already persistent", st["active"] and not st["possible"], str(st))

fake_disk("sdc", 16 * GB, [(1, 2048, 16 * GB - 2048)])
mounts("overlay / overlay rw", "/dev/sdc1 /mnt/nexa-live vfat ro")
st = persist.status()
check("status: Rufus ISO-mode stick explains DD mode", not st["possible"] and "DD Image mode" in st["reason"], str(st))

fake_disk("sdd", ISO + GB // 4, [])
mounts("overlay / overlay rw", "/dev/sdd /mnt/nexa-live iso9660 ro")
st = persist.status()
check("status: too-small stick says why", not st["possible"] and "1 GB" in st["reason"], str(st))

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
