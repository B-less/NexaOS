#!/usr/bin/env python3
"""End-to-end test of the persistent live USB, in QEMU.

Writes the ISO to the start of an 8 GB sparse "USB stick" (as Rufus does in
DD mode) and boots it three times:
  1. save a file in the home folder, then run Settings' "Keep my files on
     this USB" (nexa-settings-helper persist_create)
  2. the file must still be there and persistence active; change the system
  3. the system change must have survived as well

Usage: python3 scripts/test-persistence.py [ISO]   (default: dist/NexaOS-<version>.iso)
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIVE_BOOT = os.path.join(REPO, "scripts", "test-live-boot.py")
STICK_BYTES = 8 * 1024 ** 3


def version():
    with open(os.path.join(REPO, "build.conf")) as f:
        return next(l.split("=", 1)[1].strip() for l in f if l.startswith("PROJECT_VERSION="))


def boot(stick, command, shot):
    r = subprocess.run([sys.executable, LIVE_BOOT, "--disk", stick, "--shot", shot, "--run", command],
                       capture_output=True, text=True, timeout=900)
    out = r.stdout + r.stderr
    if "--- --run output" not in out:
        print(out[-3000:])
        raise SystemExit("FAIL: no output from the booted system")
    return out.split("--- --run output", 1)[1]


def value(out, key):
    m = re.findall(rf"{key}=([^\r\n]*)", out)
    return m[-1].strip() if m else ""


def main(argv):
    iso = argv[0] if argv else os.path.join(REPO, "dist", f"NexaOS-{version()}.iso")
    work = tempfile.mkdtemp(prefix="nexa-persist-", dir="/var/tmp")
    stick = os.path.join(work, "stick.img")
    try:
        with open(stick, "wb") as f:
            f.truncate(STICK_BYTES)
        subprocess.run(["dd", f"if={iso}", f"of={stick}", "bs=4M", "conv=notrunc,sparse", "status=none"], check=True)
        print(f"[*] {STICK_BYTES // 1024 ** 3} GB stick with {os.path.basename(iso)} written to its start")

        out = boot(stick,
                   "echo kept-across-reboots > /home/user/persist-test.txt; chown 1000:1000 /home/user/persist-test.txt; "
                   "echo PERSIST_RESULT=$(echo '{\"action\": \"persist_create\"}' | /usr/local/bin/nexa-settings-helper); "
                   "sync",
                   os.path.join(work, "boot1.png"))
        result = value(out, "PERSIST_RESULT")
        if '"ok": true' not in result:
            raise SystemExit(f"FAIL: making the stick persistent failed: {result or out[-1500:]}")
        print(f"[ok] boot 1: persistence set up: {result}")

        out = boot(stick,
                   "echo HOME_FILE=$(cat /home/user/persist-test.txt 2>&1); "
                   "echo PERSIST_MOUNT=$(grep -c ' /mnt/nexa-persist ' /proc/mounts); "
                   "echo HOME_MOUNT=$(grep ' /home ' /proc/mounts | cut -d' ' -f1); "
                   "echo nexa-system-change > /etc/nexa-persist-marker; sync",
                   os.path.join(work, "boot2.png"))
        if value(out, "HOME_FILE") != "kept-across-reboots":
            raise SystemExit(f"FAIL: the home folder file did not survive the reboot: {value(out, 'HOME_FILE')!r}")
        if value(out, "PERSIST_MOUNT") != "1":
            raise SystemExit("FAIL: the persistence partition is not mounted on the second boot")
        print(f"[ok] boot 2: file from boot 1 still there; persistence active (/home from {value(out, 'HOME_MOUNT')})")

        out = boot(stick, "echo MARKER=$(cat /etc/nexa-persist-marker 2>&1)", os.path.join(work, "boot3.png"))
        if value(out, "MARKER") != "nexa-system-change":
            raise SystemExit(f"FAIL: the system change did not survive: {value(out, 'MARKER')!r}")
        print("[ok] boot 3: system change from boot 2 still there")
        print("\nPASS: the live USB keeps files and system changes between boots.")
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main(sys.argv[1:])
