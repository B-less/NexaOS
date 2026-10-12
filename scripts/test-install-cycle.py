#!/usr/bin/env python3
"""One-shot, fully automated test of the install-then-reboot cycle: boots the
live system, installs it onto a blank virtual disk, shuts down, then boots
THAT disk alone (no -kernel/-initrd at all) to confirm GRUB + the installed
kernel bring NexaOS up for real. No interaction needed -- run it and read the
PASS/FAIL at the end. If it gets interrupted (e.g. the host restarts), just
run it again; it always starts from a clean scratch dir.

Usage: python3 scripts/test-install-cycle.py [--uefi] [--size-gb N] [--keep PATH]
Run from the repo root. Needs: qemu-system-x86_64, and for --uefi, OVMF
firmware (Debian/Ubuntu: apt install ovmf; Alpine: apk add ovmf).

--size-gb N: size the target disk at N GiB instead of the default 8 (e.g. to
match a real USB stick you intend to `dd`/Rufus this onto afterward).
--keep PATH: on PASS, copy the installed disk image to PATH so it survives
after the scratch dir would normally just be left for inspection.
"""
import os
import re
import select
import shutil
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
UEFI = "--uefi" in sys.argv


def _argval(flag, default):
    if flag in sys.argv:
        return sys.argv[sys.argv.index(flag) + 1]
    return default


SIZE_GB = int(_argval("--size-gb", "8"))
KEEP_PATH = _argval("--keep", None)
# (code, vars) pairs; vars=None marks a unified image loaded with plain -bios.
# Unified images come first: on this host, Ubuntu's split OVMF_CODE_4M.fd
# printed nothing past its first escape codes either via -bios or as pflash
# with a VARS copy, while the unified OVMF.fd booted the installed disk to a
# login prompt. Split images are a fallback for hosts that only ship those.
OVMF_CANDIDATES = [
    ("/usr/share/qemu/OVMF.fd", None),
    ("/usr/share/ovmf/OVMF.fd", None),
    ("/usr/share/OVMF/OVMF_CODE_4M.fd", "/usr/share/OVMF/OVMF_VARS_4M.fd"),
    ("/usr/share/OVMF/OVMF_CODE.fd", "/usr/share/OVMF/OVMF_VARS.fd"),
    ("/usr/share/edk2/x64/OVMF_CODE.fd", "/usr/share/edk2/x64/OVMF_VARS.fd"),
]
TEST_PASSWORD = "nexa-test-pw"
TEST_USER = "nexa"   # installs under a non-default name, so the rename is tested too
PTS_RE = re.compile(r"char device redirected to (/dev/pts/\d+)")
# Strips ANSI/terminal-query escape sequences (e.g. ESC[6n cursor-position
# requests some subprocess emits) before we print captured output to the
# REAL host terminal -- otherwise the host terminal answers the query with
# an ESC[row;colR response, which lands in the buffer we're reading next and
# renders as garbage like "^[[28;9R" obscuring the actual error text.
ANSI_RE = re.compile(rb"\x1b\[[0-9;?]*[A-Za-z]")


def clean(raw_bytes):
    return ANSI_RE.sub(b"", raw_bytes).decode(errors="replace")


class Serial:
    """Launches QEMU with a pty-backed serial console, discovers the pty
    device QEMU actually allocated (from its own stdout), and drives it like
    a human typing at a terminal: send a line, wait for a pattern to appear
    in everything received since the process started."""

    def __init__(self, qemu_argv):
        self.proc = subprocess.Popen(
            qemu_argv, cwd=REPO, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        )
        self.buf = b""
        pts_path = self._discover_pts(timeout=10)
        if pts_path is None:
            self.close()
            raise RuntimeError("QEMU never reported a pty device (check qemu-system-x86_64 is installed)")
        # O_NOCTTY: this process must not become the pty's controlling
        # terminal, or signals/job-control could interfere with QEMU's use
        # of it.
        self.fd = os.open(pts_path, os.O_RDWR | os.O_NOCTTY)

    def _discover_pts(self, timeout):
        deadline = time.time() + timeout
        header = b""
        while time.time() < deadline:
            r, _, _ = select.select([self.proc.stdout], [], [], 0.2)
            if self.proc.stdout in r:
                chunk = os.read(self.proc.stdout.fileno(), 4096)
                if not chunk:
                    break
                header += chunk
                m = PTS_RE.search(header.decode(errors="replace"))
                if m:
                    return m.group(1)
        return None

    def _pump(self, until):
        while time.time() < until:
            r, _, _ = select.select([self.fd], [], [], 0.3)
            if self.fd in r:
                try:
                    chunk = os.read(self.fd, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                self.buf += chunk

    def wait_for(self, pattern, timeout):
        deadline = time.time() + timeout
        needle = pattern.encode()
        while time.time() < deadline:
            self._pump(min(time.time() + 1, deadline))
            if needle in self.buf:
                return True
        return False

    def send(self, line):
        os.write(self.fd, (line + "\n").encode())

    def close(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        try:
            os.close(self.fd)
        except (OSError, AttributeError):
            pass


def find_ovmf():
    for code, vars_ in OVMF_CANDIDATES:
        if os.path.isfile(code) and (vars_ is None or os.path.isfile(vars_)):
            return code, vars_
    return None


def phase(msg):
    print(f"\n=== {msg} ===", flush=True)


def fail(msg):
    print(f"\nFAIL: {msg}")
    sys.exit(1)


def kvm_usable():
    return os.path.exists("/dev/kvm") and os.access("/dev/kvm", os.R_OK | os.W_OK)


def main():
    if not shutil.which("qemu-system-x86_64"):
        fail("qemu-system-x86_64 not found on PATH")
    if not os.path.isfile(os.path.join(REPO, "dist", "nexa.img")):
        fail("dist/nexa.img not found -- build it first")
    # Without KVM, QEMU silently falls back to TCG (pure software emulation)
    # -- 10-50x slower. The installer (partition+format+full-rootfs-clone+
    # GRUB x2) does not reliably finish inside sane timeouts under TCG, and
    # this failure mode gives no error, just a mysteriously slow/hanging
    # test. Fail fast with a clear reason instead.
    if not kvm_usable():
        fail("/dev/kvm not accessible -- without it QEMU runs 10-50x slower "
             "under software emulation and this test will likely time out. "
             "Run: sudo usermod -aG kvm $USER, then open a NEW terminal "
             "(or `newgrp kvm`) before re-running this script.")
    accel = ["-enable-kvm"]

    # Not tempfile's default /tmp: on this host (and plausibly others) /tmp is
    # a small RAM-backed tmpfs (~2G) -- nowhere near enough for a 2G rootfs
    # copy plus an 8G target disk image. /var/tmp is real disk space.
    tmp = tempfile.mkdtemp(prefix="nexa-install-test-", dir="/var/tmp")
    print(f"[*] scratch dir: {tmp}")
    nexa_scratch = os.path.join(tmp, "nexa-scratch.img")
    target = os.path.join(tmp, "target.img")
    shutil.copyfile(os.path.join(REPO, "dist", "nexa.img"), nexa_scratch)
    with open(target, "wb") as f:
        f.truncate(SIZE_GB * 1024**3)

    phase(f"Booting the live system with a blank {SIZE_GB}G target disk attached")
    qemu = Serial([
        "qemu-system-x86_64", *accel, "-cpu", "host", "-m", "3G", "-smp", "2",
        "-kernel", "dist/bzImage", "-initrd", "dist/initramfs.cpio.gz",
        "-drive", f"if=none,id=nexadisk,file={nexa_scratch},format=raw",
        "-device", "virtio-blk-pci,drive=nexadisk",
        "-drive", f"if=none,id=target,file={target},format=raw",
        "-device", "virtio-blk-pci,drive=target",
        "-append", "root=/dev/vda console=ttyS0 rootwait",
        "-nographic", "-chardev", "pty,id=ser0", "-serial", "chardev:ser0",
        "-monitor", "none",
        "-net", "nic,model=virtio", "-net", "user", "-display", "none",
    ])

    if not qemu.wait_for("login:", 120):
        qemu.close()
        fail("live system never reached the login prompt")
    print("[ok] live system booted")

    qemu.send("")
    time.sleep(1)
    qemu.send("root")
    if not qemu.wait_for("~#", 15):
        qemu.close()
        fail("root login didn't return a prompt")
    # The VM's tty echoes back everything we send, so any later wait_for()
    # on a marker we ourselves typed (e.g. "echo MARKER_$?") matches
    # instantly against that echo -- before the command has even run, let
    # alone finished -- making completion checks fire immediately and
    # falsely. Turning off echo means the buffer only ever contains genuine
    # output from here on.
    qemu.send("stty -echo")
    time.sleep(1)
    qemu.send("su user -c sh")
    qemu.wait_for("$", 10)
    qemu.send(
        "cat > /tmp/req.json << 'EOF'\n"
        '{"disk": "vdb", "hostname": "nexa-test", "zone": null, "keyboard": null, '
        f'"password": "{TEST_PASSWORD}", "username": "{TEST_USER}"}}\n'
        "EOF"
    )
    time.sleep(1)

    phase("Running the installer (partition, format, clone, GRUB x2) -- this takes a few minutes")
    marker = "NEXA_INSTALL_RESULT"
    qemu.send(f"doas /usr/local/bin/nexa-setup-helper < /tmp/req.json > /tmp/install.out 2>&1; echo {marker}_$?")
    if not qemu.wait_for(f"{marker}_", 600):
        qemu.close()
        fail("installer never finished within 10 minutes")
    ok = f"{marker}_0".encode() in qemu.buf[-4000:]
    before_cat = len(qemu.buf)
    cat_marker = "NEXA_CAT_DONE"
    # A reused prompt string like "/root $" already exists earlier in the
    # cumulative buffer (from the shell prompt right after login), so
    # wait_for() would match instantly against stale data instead of
    # actually waiting for this command's output. A marker that has never
    # appeared before forces a genuine wait.
    qemu.send(f"cat /tmp/install.out; echo {cat_marker}")
    qemu.wait_for(cat_marker, 10)
    raw_out = qemu.buf[before_cat:]
    raw_path = os.path.join(tmp, "install.out.raw")
    with open(raw_path, "wb") as f:
        f.write(raw_out)
    print(clean(raw_out))
    print(f"[*] raw (unfiltered) bytes also saved to {raw_path}")
    if not ok:
        qemu.close()
        fail("nexa-setup-helper exited non-zero -- see output above")
    print("[ok] installer reported success")
    qemu.close()
    time.sleep(2)

    phase(f"Booting the installed disk ALONE (no -kernel/-initrd) -- {'UEFI' if UEFI else 'BIOS'}")
    argv = ["qemu-system-x86_64", *accel, "-cpu", "host", "-m", "2G",
            "-drive", f"file={target},format=raw",
            "-nographic", "-chardev", "pty,id=ser0", "-serial", "chardev:ser0",
            "-monitor", "none", "-net", "none", "-display", "none"]
    if UEFI:
        ovmf = find_ovmf()
        if not ovmf:
            fail("--uefi requested but no OVMF firmware found (tried: "
                 + ", ".join(code for code, _ in OVMF_CANDIDATES) + ")")
        code, vars_ = ovmf
        if vars_ is None:
            argv[1:1] = ["-bios", code]
        else:
            vars_copy = os.path.join(tmp, "OVMF_VARS.fd")
            shutil.copyfile(vars_, vars_copy)
            argv[1:1] = ["-drive", f"if=pflash,format=raw,readonly=on,file={code}",
                         "-drive", f"if=pflash,format=raw,file={vars_copy}"]

    boot2 = Serial(argv)
    # BIOS mode is slow here only because QEMU's SeaBIOS reads disks through
    # emulated int13 calls: GRUB takes ~70 s to load the 37 MB initramfs
    # (firmware for i915/amdgpu lives in it), then the OS boots in ~25 s.
    reached = boot2.wait_for("login:", 240)
    if not reached:
        tail = boot2.buf[-2000:].decode(errors="replace")
        boot2.close()
        fail(f"installed disk did NOT boot to a login prompt on its own ({'UEFI' if UEFI else 'BIOS'} mode) -- "
             "GRUB/kernel handoff failed. Last output:\n" + tail)
    print("[ok] installed disk reached the login prompt")

    # The live image has empty passwords; the installer must have replaced
    # them. Waiting for "Password:" before typing it matters: with an empty
    # password the shell starts at once, the password line would just run as
    # a failed command, and the marker below would still print -- a false PASS.
    phase("Logging in to the installed system with the password set by the installer")
    time.sleep(2)
    boot2.send(TEST_USER)
    if not boot2.wait_for("Password:", 15):
        tail = boot2.buf[-1000:].decode(errors="replace")
        boot2.close()
        fail("login never asked for a password -- the installer did not set one. Last output:\n" + tail)
    boot2.send(TEST_PASSWORD)
    # Typed text is echoed as "$(id -un)", so only real command output
    # contains "NEXA_LOGIN_user".
    time.sleep(2)
    boot2.send("echo NEXA_LOGIN_$(id -un)")
    logged_in = boot2.wait_for(f"NEXA_LOGIN_{TEST_USER}", 20)
    if not logged_in:
        tail = boot2.buf[-1000:].decode(errors="replace")
        boot2.close()
        fail(f"logging in as {TEST_USER} with the installer-set password failed. Last output:\n" + tail)
    print(f"[ok] logged in as {TEST_USER} with the password chosen at install time")

    # The installed system's privilege setup (no root needed to read these).
    checks = {
        "HOME": f"test -d /home/{TEST_USER} && echo yes",
        "SOFTWARE_NEEDS_PW": f"grep -cx 'permit {TEST_USER} cmd /usr/local/bin/nexa-software-helper' /etc/doas.conf 2>/dev/null || ls -l /etc/doas.conf",
        "LOGIN_LOCK": "test -f /etc/nexa/login-required && echo yes",
    }
    for key, cmd in checks.items():
        boot2.send(f"echo NEXA_{key}=$({cmd})")
    boot2.wait_for("NEXA_LOGIN_LOCK=", 15)
    time.sleep(1)
    boot2._pump(time.time() + 2)
    out = boot2.buf.decode(errors="replace")
    boot2.close()
    import re as _re
    found = {k: (_re.findall(rf"NEXA_{k}=([^\r\n]*)", out) or [""])[-1].strip() for k in checks}
    if found["HOME"] != "yes":
        fail(f"home directory /home/{TEST_USER} missing after the rename ({found})")
    if found["LOGIN_LOCK"] != "yes":
        fail(f"/etc/nexa/login-required missing: the desktop would start unlocked ({found})")
    print(f"[ok] account renamed, starts locked; doas.conf check: {found['SOFTWARE_NEEDS_PW']}")

    print(f"\nPASS: the installed disk booted on its own via GRUB ({'UEFI' if UEFI else 'BIOS'} mode), "
          "no -kernel/-initrd needed, and the installer-set password works.")
    if KEEP_PATH:
        shutil.copyfile(target, KEEP_PATH)
        print(f"[*] installed disk image copied to {KEEP_PATH}")
    print(f"(scratch dir left at {tmp} for inspection; delete it whenever)")


if __name__ == "__main__":
    main()
