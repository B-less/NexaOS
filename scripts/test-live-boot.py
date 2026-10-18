#!/usr/bin/env python3
"""Boot the live ISO in QEMU (UEFI, as a USB stick), wait for the desktop,
save a screenshot, log in on the serial console and print a health report.

Usage: python3 scripts/test-live-boot.py [ISO] [--shot desktop.png] [--run 'shell command']
--run also runs that command as root on the booted system and prints its output.
--no-checks skips the health report (only --run / --fetch / the screenshot).
--fetch REMOTE LOCAL copies a file off the booted system after 85 s of uptime
(used by `make preload-map` for the boot readahead map).
--disk FILE boots a writable USB-stick image (e.g. the ISO written into a bigger
file) instead of the read-only ISO; used by scripts/test-persistence.py.
Defaults: dist/NexaOS-<PROJECT_VERSION>.iso, screenshot to dist/live-desktop.png.
Needs qemu-system-x86_64 with KVM and OVMF (/usr/share/qemu/OVMF.fd).
A VM has no real GPU, so this exercises the Sway session.
"""
import base64
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
    'echo PTRACE_SCOPE=$(sysctl -n kernel.yama.ptrace_scope 2>&1)',
    'echo SSHD=$(rc-service sshd status 2>&1 | grep -o "started\\|stopped")',
    'echo FW_ERRORS=$(dmesg | grep -ciE "firmware.*(fail|error)")',
    # [x]-bracketed patterns can't match this command line itself.
    'for p in "[s]way -c" "[H]yprland" "[w]aybar" "[m]ako" "[s]waybg" "[n]exa_shell dock"; do '
    'pgrep -f "$p" >/dev/null && echo "RUN:$p" || echo "NOT:$p"; done',
    'echo ICON_THEME=$(test -f /usr/share/icons/hicolor/index.theme && echo ok || echo missing)',
    # The live account has no password: Enter (empty) must unlock the lock screen.
    ': > /tmp/empty; chmod 644 /tmp/empty; echo UNLOCK_EMPTY=$(su user -s /bin/sh -c "/usr/sbin/unix_chkpwd user nullok < /tmp/empty"; echo $?)',
    'echo BROWSER_SANDBOX=$(grep -c "NEXA_BROWSER_SANDBOX:-1" /usr/local/bin/nexa-browser)',
    'echo BLUETOOTHD=$(rc-service bluetooth status 2>&1 | grep -o "started\\|stopped\\|crashed")',
    'echo WIREGUARD=$(ip link add nexatest type wireguard 2>&1 && ip link del nexatest && echo ok)',
    'echo POLICY_ROUTING=$(ip rule show | grep -c .)',
    # The assistant's hands: a sandboxed (Landlock) write as the desktop user.
    """printf '%s' '{"tool":"write_text","args":{"path":"~/.cache/nexa-tools-check","content":"hi","mode":"overwrite"}}' > /tmp/hreq; """
    'chmod 644 /tmp/hreq; echo HANDS=$(su user -s /bin/sh -c "nexa-tools < /tmp/hreq" 2>&1 | grep -o "\\"ok\\":[a-z]*")',
    # A signed .nexa from ~/Downloads installs through the system broker.
    """P=$(ls /home/user/Downloads/*.nexa 2>/dev/null | head -1); [ -n "$P" ] && printf '{"path":"%s"}' "$P" > /tmp/ireq && """
    """echo NEXA_INSTALL=$(nexa-brokerd call install_app "$(cat /tmp/ireq)" | grep -o '"ok":[a-z]*' | tr -d '"')$(command -v $(basename ${P%%-[0-9]*}) >/dev/null && echo ',command')$(test -f /usr/share/applications/nexa-$(basename ${P%%-[0-9]*}).desktop && echo ',menu')$(test "$(readlink /usr/local/bin/$(basename ${P%%-[0-9]*}))" = nexa-run && echo ',confined')""",
    # The system broker: the desktop user changes a setting through it; a
    # non-administrator account is refused.
    """printf '%s' '{"zone":"Africa/Accra"}' > /tmp/breq; chmod 644 /tmp/breq; """
    r'echo BROKER=$(su user -s /bin/sh -c "nexa-brokerd call set_timezone \"\$(cat /tmp/breq)\"" | grep -o "\"ok\":[a-z]*")$(grep -q Africa/Accra /etc/timezone && echo ,applied)',
    'echo BROKER_DENY=$(su nexa-mind -s /bin/sh -c "nexa-brokerd call set_brightness {}" | grep -o "only administrators")',
    # The Files app's engine (Rust, Landlock) as the desktop user: Delete = Trash.
    r"""su user -s /bin/sh -c 'echo hi > $HOME/fo-check; printf "{\"op\":\"trash\",\"paths\":[\"%s/fo-check\"]}" $HOME | nexa-fileops' > /tmp/fo; """
    r"""echo FILEOPS=$(grep -o '"ok":[a-z]*' /tmp/fo | tr -d '"')$(test -f /home/user/.local/share/Trash/info/fo-check.trashinfo && echo ,trashed)""",
    # Nexa Mind's kernel link: which kernel interfaces it got (PSI triggers,
    # process connector, OOM log, RAPL, uclamp, the AI's cgroup).
    r"""echo KERNELD=$(nexa-kerneld status 2>&1 | tr -d ' \n' | grep -o '"can":{[^}]*}')""",
    # The stick NexaOS runs from is never auto-mounted, offered or ejected
    # (ejecting it pulled the system out from under itself).
    r'L=$(basename $(cat /sys/block/loop1/loop/backing_file)); echo LIVE_STICK=listed:$(nexa-brokerd call list_drives {} | grep -c "\"name\":\"$L"),'
    r'mounted:$(grep -c "^/dev/$L.* /media/" /proc/mounts)$(nexa-brokerd call eject_drive "{\"name\":\"${L}1\"}" | grep -q "using this drive itself" && echo ,eject_refused)',
    # No root shell on the screen: tty1 belongs to the desktop, tty2 asks for a login.
    'echo CONSOLE=tty1_root:$(ps -o tty,user,comm | grep -c "^tty1 *root")$(pgrep -f "getty 38400 tty2" >/dev/null && echo ,tty2_login)',
    'echo APPS=$(for c in nexa-pulse nexadark tor wg-quick nexa-pack nexa-mindd nexa-tools nexa-brokerd llama-server; do command -v $c >/dev/null && printf "%s," $c; done)',
    # Nexa Mind runs at boot and the desktop user can reach it (group nexa-mind).
    'echo MIND=$(su user -s /bin/sh -c "nexa-mindd status" 2>&1 | grep -o "\\"state\\": *\\"[a-z]*\\"" | tr -d " \\"")',
)


def project_version():
    with open(os.path.join(REPO, "build.conf")) as f:
        for line in f:
            if line.startswith("PROJECT_VERSION="):
                return line.split("=", 1)[1].strip()
    return "unknown"


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
    extra = argv[argv.index("--run") + 1] if "--run" in argv else None
    disk = argv[argv.index("--disk") + 1] if "--disk" in argv else None
    shot = argv[argv.index("--shot") + 1] if "--shot" in argv else os.path.join(REPO, "dist", "live-desktop.png")
    fetch = argv[argv.index("--fetch") + 1:argv.index("--fetch") + 3] if "--fetch" in argv else None
    no_checks = "--no-checks" in argv
    argv = [a for i, a in enumerate(argv)
            if not (a in ("--run", "--disk", "--shot", "--fetch") or (i and argv[i - 1] in ("--run", "--disk", "--shot", "--fetch"))
                    or (i > 1 and argv[i - 2] == "--fetch"))]
    args = [a for a in argv if not a.startswith("--")]
    iso = args[0] if args else os.path.join(REPO, "dist", f"NexaOS-{project_version()}.iso")
    work = tempfile.mkdtemp(prefix="nexa-live-")
    mon, ser = os.path.join(work, "mon.sock"), os.path.join(work, "ser.sock")
    qemu = subprocess.Popen(
        ["qemu-system-x86_64", "-enable-kvm", "-cpu", "host", "-m", "4G", "-smp", "2", "-bios", OVMF,
         "-drive", (f"file={disk},format=raw,if=none,id=u" if disk
                    else f"file={iso},format=raw,if=none,id=u,readonly=on"), "-device", "qemu-xhci",
         "-device", "usb-storage,drive=u", "-nic", "user,model=virtio", "-display", "none",
         "-serial", f"unix:{ser},server,nowait", "-monitor", f"unix:{mon},server,nowait"],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=open(os.path.join(work, "qemu.err"), "w"))
    try:
        for _ in range(100):
            if os.path.exists(mon) and os.path.exists(ser):
                break
            time.sleep(0.1)
        def connect(path):
            # The socket file appears a moment before QEMU listens on it.
            for _ in range(50):
                sock = socket.socket(socket.AF_UNIX)
                try:
                    sock.connect(path)
                    return sock
                except (ConnectionRefusedError, FileNotFoundError):
                    sock.close()
                    if qemu.poll() is not None:
                        raise SystemExit(f"QEMU exited with code {qemu.returncode}: "
                                         + open(os.path.join(work, "qemu.err")).read().strip())
                    time.sleep(0.1)
            raise SystemExit(f"QEMU never accepted connections on {path}")

        m = connect(mon)
        s = connect(ser)
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
        s.sendall(b"root\n")
        pump(3)
        marker = "NEXA_CHECKS_DONE"
        checks = ([] if no_checks else list(CHECKS)) + ([f"echo EXTRA_BEGIN; {extra}; echo EXTRA_END"] if extra else [])
        # One command per line: BusyBox's line editor stops at ~2 KB.
        s.sendall(("\n".join(checks) + f"\necho {marker}\n").encode())
        end = time.time() + (120 if extra else 20)
        while time.time() < end and buf.count(marker.encode()) < 2:
            pump(1)
        text = buf.decode(errors="replace").replace("\r", "")
        report = [l for l in text.rsplit(marker, 1)[0].splitlines()
                  if l.startswith(("ALPINE=", "FIREWALL=", "KPTR=", "PTRACE_SCOPE=", "SSHD=", "FW_ERRORS=", "RUN:", "NOT:",
                                   "ICON_THEME=", "UNLOCK_EMPTY=", "BROWSER_SANDBOX=", "BLUETOOTHD=",
                                   "WIREGUARD=", "POLICY_ROUTING=", "APPS=", "MIND=", "HANDS=", "NEXA_INSTALL=", "BROKER=", "BROKER_DENY=", "LIVE_STICK=", "FILEOPS=", "KERNELD=", "CONSOLE="))] if text.count(marker) >= 2 else []
        if not no_checks:
            print("\n".join(report) if report else "no report (serial login failed?)\n--- serial tail\n" + text[-1500:])
        if extra and text.count("EXTRA_END") >= 2:
            print("--- --run output\n" + text.split("EXTRA_BEGIN")[-1].split("EXTRA_END")[0].strip())

        if fetch:
            remote, local = fetch
            buf = b""
            s.sendall(f'while [ $(cut -d. -f1 /proc/uptime) -lt 85 ]; do sleep 1; done; echo FETCH_BEGIN; '
                      f'base64 {remote} | tr -d "\\n"; echo; echo FETCH_END\n'.encode())
            end = time.time() + 150
            while time.time() < end and buf.count(b"FETCH_END") < 2:
                pump(1)
            text = buf.decode(errors="replace").replace("\r", "")
            data = "".join(text.split("FETCH_BEGIN")[-1].split("FETCH_END")[0].split()) if text.count("FETCH_END") >= 2 else ""
            if not data:
                print(f"FAIL: couldn't fetch {remote}")
                return 1
            with open(local, "wb") as f:
                f.write(base64.b64decode(data + "=" * (-len(data) % 4)))
            print(f"fetched {remote} -> {local} ({os.path.getsize(local)} bytes)")

        # Screenshot from inside the session with grim: QEMU's own screendump
        # comes out black under Sway here. Without a GPU the bar and Dock
        # appear ~45 s after the login prompt, so wait for 80 s of uptime.
        buf = b""
        s.sendall(b'while [ $(cut -d. -f1 /proc/uptime) -lt 80 ]; do sleep 1; done; R=/run/user/1000; '
                  b'W=$(basename $(ls $R/wayland-? | head -1)); su user -s /bin/sh -c '
                  b'"XDG_RUNTIME_DIR=$R WAYLAND_DISPLAY=$W grim -t png /tmp/shot.png"; '
                  b'echo SHOT_BEGIN; base64 /tmp/shot.png | tr -d "\\n"; echo; echo SHOT_END\n')
        end = time.time() + 120
        while time.time() < end and buf.count(b"SHOT_END") < 2:
            pump(1)
        text = buf.decode(errors="replace").replace("\r", "")
        if text.count("SHOT_END") >= 2:
            data = "".join(text.split("SHOT_BEGIN")[-1].split("SHOT_END")[0].split())
            with open(shot, "wb") as f:
                f.write(base64.b64decode(data + "=" * (-len(data) % 4)))
            print(f"screenshot: {shot}")
        else:
            ppm = os.path.join(work, "desktop.ppm")
            m.sendall(f"screendump {ppm}\n".encode())
            time.sleep(1)
            ppm_to_png(ppm, shot)
            print(f"screenshot (QEMU screendump; grim failed): {shot}")
        return 0
    finally:
        qemu.terminate()
        qemu.wait()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
