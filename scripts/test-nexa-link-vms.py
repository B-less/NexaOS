#!/usr/bin/env python3
"""Nexa Link between two real NexaOS systems: two QEMU VMs booted from
dist/nexa.img (writes discarded), each with its own internet network (the
default route, both 10.0.2.15) and one private network between them
(192.168.77.0/24, its addresses set without a broadcast address), as a PC
on Wi-Fi with a second network. Everything runs as the desktop user, in the
real session, through the real firewall and sandbox:

  pairing (the code carries every address; the other side finds the right
  one), each finding the other (signals), a file declined (nothing saved),
  a file accepted (arrives whole), Forget (cut off at once).

Usage: python3 scripts/test-nexa-link-vms.py [INITRD]
INITRD: another initramfs (default dist/initramfs-core.cpio.gz), e.g. one
that copies a freshly built nexa-linkd over the image before it starts.
Needs KVM; about 4 minutes."""
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INITRD = sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "dist", "initramfs-core.cpio.gz")
work = tempfile.mkdtemp(prefix="nl-vms-", dir="/tmp")
failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""), flush=True)
    if not cond:
        failures.append(label)


class VM:
    def __init__(self, n):
        self.n, self.buf = n, b""
        ser = os.path.join(work, f"s{n}")
        self.q = subprocess.Popen(
            ["qemu-system-x86_64", "-enable-kvm", "-cpu", "host", "-m", "2G", "-smp", "2",
             "-kernel", os.path.join(REPO, "dist", "bzImage"), "-initrd", INITRD,
             "-append", "quiet splash loglevel=3 console=ttyS0 console=tty0",
             "-drive", f"file={os.path.join(REPO, 'dist', 'nexa.img')},format=raw,if=virtio,snapshot=on",
             "-nic", "user,model=virtio",
             "-netdev", "socket,id=lan,mcast=230.0.0.1:47999", "-device", f"virtio-net-pci,netdev=lan,mac=52:54:00:77:00:0{n}",
             "-display", "none", "-serial", f"unix:{ser},server,nowait", "-monitor", "none"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=dict(os.environ, TMPDIR=work))
        for _ in range(200):
            try:
                self.s = socket.socket(socket.AF_UNIX)
                self.s.connect(ser)
                break
            except OSError:
                time.sleep(0.05)
        self.s.settimeout(0.1)

    def pump(self, t=0.3):
        end = time.time() + t
        while time.time() < end:
            try:
                d = self.s.recv(65536)
                self.buf += d or b""
            except socket.timeout:
                pass

    def wait(self, pat, t):
        end = time.time() + t
        while time.time() < end:
            self.pump()
            if re.search(pat.encode(), self.buf):
                return True
        return False

    def run(self, cmd, t=30):
        tag = str(time.time_ns())
        b, e = f"BEG{tag}", f"END{tag}"
        start = len(self.buf)
        self.s.sendall(f"echo {b[:3]}''{b[3:]}; {cmd}; echo {e[:3]}''{e[3:]}\n".encode())
        end = time.time() + t
        while time.time() < end and e.encode() not in self.buf[start:]:
            self.pump()
        out = self.buf[start:].decode(errors="replace").replace("\r", "")
        return out.split(b, 1)[-1].split(e, 1)[0].strip("\n")

    def user(self, cmd, t=30):
        return self.run(f"su user -s /bin/sh -c 'export XDG_RUNTIME_DIR=/run/user/1000 HOME=/home/user; {cmd}'", t)


def offer_id(vm, after=0):
    for _ in range(30):
        found = re.findall(r'"offer":(\d+)', vm.run("cat /tmp/events"))
        found = [f for f in found if int(f) > after]
        if found:
            return int(found[0])
        time.sleep(1)
    return None


vms = [VM(1), VM(2)]
a, b = vms
try:
    for v in vms:
        check(f"VM {v.n} boots", v.wait("login:", 180))
        v.s.sendall(b"\nroot\n")
        v.pump(3)
        v.run("stty -echo")
        # No broadcast address on purpose: some network setups leave it out.
        v.run(f"ip addr add 192.168.77.{v.n}/24 dev eth1; ip link set eth1 up")
    for v in vms:
        for _ in range(90):
            if v.run("pgrep -x nexa-linkd"):
                break
            time.sleep(1)
        check(f"VM {v.n}: nexa-linkd runs in the desktop session", bool(v.run("pgrep -x nexa-linkd")))
    ida = re.search(r'"id":"([0-9a-f]+)","name":"([^"]*)"', a.user("nexa-linkd id"))
    idb = re.search(r'"id":"([0-9a-f]+)","name":"([^"]*)"', b.user("nexa-linkd id"))
    check("the two get different names (not both \"nexa\")", ida and idb and ida.group(2) != idb.group(2),
          f"{ida and ida.group(2)} / {idb and idb.group(2)}")
    a.user("nexa-linkd pair-offer --yes --json > /tmp/pair.log 2>&1 &", 5)
    time.sleep(2)
    code = re.search(r'"code":"([^"]+)"', a.run("cat /tmp/pair.log"))
    check("the pairing code lists both networks' addresses", code and "10.0.2.15" in code.group(1)
          and "192.168.77.1" in code.group(1), code and code.group(1))
    out = b.user(f"nexa-linkd pair-accept '{code.group(1) if code else ''}' --yes --json 2>&1", 60)
    time.sleep(2)
    sas_b = re.search(r'"sas":"(\d+)"', out)
    sas_a = re.search(r'"sas":"(\d+)"', a.run("cat /tmp/pair.log"))
    check("paired across the private network (same six digits on both)", sas_a and sas_b and sas_a.group(1) == sas_b.group(1)
          and '"paired"' in out, out[-300:])
    time.sleep(12)
    check("A sees B nearby", '"online":true' in a.user("nexa-linkd devices"), a.user("nexa-linkd devices"))
    check("B sees A nearby", '"online":true' in b.user("nexa-linkd devices"), b.user("nexa-linkd devices"))
    b.run("head -c 300000 /dev/urandom > /tmp/photo.jpg; chmod 644 /tmp/photo.jpg")
    a.user("nexa-linkd watch > /tmp/events 2>&1 &", 5)
    time.sleep(1)
    target = ida.group(1) if ida else ""
    b.user(f"nexa-linkd send {target} /tmp/photo.jpg --json > /tmp/send1.log 2>&1 &", 5)
    first = offer_id(a)
    check("A is asked before anything is saved", first is not None)
    a.user(f"nexa-linkd answer {first} no")
    time.sleep(3)
    check("declined: B is told, nothing saved", '"declined"' in b.run("cat /tmp/send1.log")
          and "photo.jpg" not in a.run("ls /home/user/Downloads"), b.run("cat /tmp/send1.log"))
    b.user(f"nexa-linkd send {target} /tmp/photo.jpg --json > /tmp/send2.log 2>&1 &", 5)
    second = offer_id(a, after=first or 0)
    a.user(f"nexa-linkd answer {second} yes")
    for _ in range(20):
        if '"sent"' in b.run("cat /tmp/send2.log"):
            break
        time.sleep(1)
    sums = (b.run("sha256sum /tmp/photo.jpg | cut -c1-64"), a.run("sha256sum /home/user/Downloads/photo.jpg 2>&1 | cut -c1-64"))
    check("accepted: it arrives whole in Downloads", sums[0] and sums[0] == sums[1], str(sums))
    a.user(f"nexa-linkd forget {idb.group(1) if idb else ''}")
    out = b.user(f"nexa-linkd send {target} /tmp/photo.jpg --json 2>&1", 30)
    check("after Forget on A, B can't send any more", '"sent"' not in out, out[-200:])
finally:
    for v in vms:
        v.q.terminate()
        v.q.wait()
    shutil.rmtree(work, ignore_errors=True)
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
