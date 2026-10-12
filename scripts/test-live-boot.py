#!/usr/bin/env python3
"""Boot the live ISO in QEMU (UEFI, as a USB stick), wait for the desktop,
save a screenshot, log in on the serial console and print a health report.

Usage: python3 scripts/test-live-boot.py [ISO] [--shot desktop.png]
Defaults: dist/NexaOS-3.1.0.iso, screenshot to dist/live-desktop.png.
Needs qemu-system-x86_64 with KVM and OVMF (/usr/share/qemu/OVMF.fd).
A VM has no real GPU, so this exercises the Sway session.
"""
import os
import socket
import struct
import subprocess
import sys
import tempfile
import time
import zlib

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OVMF = "/usr/share/qemu/OVMF.fd"
CHECKS = (
    'echo ALPINE=$(cat /etc/alpine-release)',
    'echo FIREWALL=$(nft list ruleset 2>/dev/null | grep -c "policy drop")',
    'echo KPTR=$(sysctl -n kernel.kptr_restrict)',
    'echo SSHD=$(rc-service sshd status 2>&1 | grep -o "started\\|stopped")',
    'echo FW_ERRORS=$(dmesg | grep -ciE "firmware.*(fail|error)")',
    # [x]-bracketed patterns can't match this command line itself.
    'for p in "[s]way -c" "[H]yprland" "[w]aybar" "[m]ako" "[s]waybg" "[n]exa_shell dock"; do '
    'pgrep -f "$p" >/dev/null && echo "RUN:$p" || echo "NOT:$p"; done',
    'echo ICON_THEME=$(test -f /usr/share/icons/hicolor/index.theme && echo ok || echo missing)',
    'echo BROWSER_SANDBOX=$(grep -c "NEXA_BROWSER_SANDBOX:-1" /usr/local/bin/nexa-browser)',
)


def ppm_to_png(ppm_path, png_path, half=True):
    data = open(ppm_path, "rb").read()
    header = data.split(b"\n", 3)
    w, h = map(int, header[1].split())
    px = header[3]
    step = 2 if half else 1
    W, H = w // step, h // step
    rows = []
    for y in range(H):
        row = px[(y * step) * w * 3:(y * step + 1) * w * 3]
        rows.append(b"\x00" + b"".join(row[x * step * 3:x * step * 3 + 3] for x in range(W)))

    def chunk(tag, body):
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body) & 0xffffffff)

    open(png_path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
                               + chunk(b"IDAT", zlib.compress(b"".join(rows), 6)) + chunk(b"IEND", b""))


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    iso = args[0] if args else os.path.join(REPO, "dist", "NexaOS-3.1.0.iso")
    shot = argv[argv.index("--shot") + 1] if "--shot" in argv else os.path.join(REPO, "dist", "live-desktop.png")
    work = tempfile.mkdtemp(prefix="nexa-live-")
    mon, ser = os.path.join(work, "mon.sock"), os.path.join(work, "ser.sock")
    qemu = subprocess.Popen(
        ["qemu-system-x86_64", "-enable-kvm", "-cpu", "host", "-m", "4G", "-smp", "2", "-bios", OVMF,
         "-drive", f"file={iso},format=raw,if=none,id=u,readonly=on", "-device", "qemu-xhci",
         "-device", "usb-storage,drive=u", "-nic", "user,model=virtio", "-display", "none",
         "-serial", f"unix:{ser},server,nowait", "-monitor", f"unix:{mon},server,nowait"],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(100):
            if os.path.exists(mon) and os.path.exists(ser):
                break
            time.sleep(0.1)
        m = socket.socket(socket.AF_UNIX)
        m.connect(mon)
        s = socket.socket(socket.AF_UNIX)
        s.connect(ser)
        s.settimeout(0.5)
        buf = b""

        def pump(seconds):
            nonlocal buf
            end = time.time() + seconds
            while time.time() < end:
                try:
                    buf += s.recv(65536)
                except socket.timeout:
                    pass

        started = time.time()
        while b"login:" not in buf and time.time() - started < 180:
            pump(1)
        if b"login:" not in buf:
            print("FAIL: no login prompt within 3 minutes")
            return 1
        print(f"login prompt after {time.time() - started:.0f}s")
        pump(20)  # let the desktop settle
        ppm = os.path.join(work, "desktop.ppm")
        m.sendall(f"screendump {ppm}\n".encode())
        time.sleep(1)
        ppm_to_png(ppm, shot)
        print(f"screenshot: {shot}")
        s.sendall(b"root\n")
        pump(3)
        marker = "NEXA_CHECKS_DONE"
        s.sendall(("; ".join(CHECKS) + f"; echo {marker}\n").encode())
        end = time.time() + 20
        while time.time() < end and buf.count(marker.encode()) < 2:
            pump(1)
        text = buf.decode(errors="replace").replace("\r", "")
        report = [l for l in text.split(marker)[-2].splitlines()
                  if l.startswith(("ALPINE=", "FIREWALL=", "KPTR=", "SSHD=", "FW_ERRORS=", "RUN:", "NOT:",
                                   "ICON_THEME=", "BROWSER_SANDBOX="))] if text.count(marker) >= 2 else []
        print("\n".join(report) if report else "no report (serial login failed?)")
        return 0
    finally:
        qemu.terminate()
        qemu.wait()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
