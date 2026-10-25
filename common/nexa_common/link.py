"""Nexa Link from the UI (Settings, the pop-ups, Files): a thin client for
nexa-linkd (Rust), which does the work. Requests go over its socket,
$XDG_RUNTIME_DIR/nexa-link.sock, one JSON line each way; pairing and
sending run the nexa-linkd command, which reports progress as JSON lines.
"""
import json
import os
import shutil
import socket
import subprocess

BIN = os.environ.get("NEXA_LINKD", "nexa-linkd")


class LinkError(Exception):
    pass


def sock_path():
    run = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    return os.path.join(run, "nexa-link.sock")


def available():
    return bool(shutil.which(BIN) or os.path.isfile(BIN))


def request(cmd, timeout=5, **fields):
    """One request to the service; its reply. LinkError if it says no."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect(sock_path())
        s.sendall((json.dumps({"cmd": cmd, **fields}) + "\n").encode())
        data = b""
        while not data.endswith(b"\n"):
            chunk = s.recv(65536)
            if not chunk:
                break
            data += chunk
    except OSError:
        raise LinkError("Nexa Link isn't running.") from None
    finally:
        s.close()
    try:
        reply = json.loads(data.decode() or "{}")
    except ValueError:
        raise LinkError("Nexa Link gave an answer that makes no sense.") from None
    if reply.get("ok") is False:
        raise LinkError(reply.get("error") or "That didn't work.")
    return reply


def _cli(*args, timeout=10):
    try:
        r = subprocess.run([BIN, *args], capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.SubprocessError) as e:
        raise LinkError(f"Nexa Link isn't installed or didn't answer ({e}).") from None
    if r.returncode != 0:
        raise LinkError((r.stderr or r.stdout).strip().removeprefix("nexa-linkd: ") or "That didn't work.")
    return r.stdout


def identity():
    return json.loads(_cli("id"))


def rename(name):
    _cli("name", name)


def devices():
    return request("devices").get("devices", [])


def nearby():
    return request("nearby").get("nearby", [])


def status():
    return request("status")


def set_drop(minutes):
    return request("drop", minutes=int(minutes)).get("drop_open", False)


def answer(offer, accept):
    request("answer", offer=int(offer), accept=bool(accept))


def allow(device, files):
    request("allow", id=device, files=files)


def forget(device):
    request("forget", id=device)


def ring(device):
    _cli("ring", device, timeout=15)


def send_text(target, text):
    _cli("text", target, text, timeout=140)


def qr_rows(text):
    """A QR code for `text`: a list of strings of 0/1, 1 = dark."""
    return json.loads(_cli("qr", text)).get("rows", [])


def start(*args):
    """A long nexa-linkd command (pair-offer, pair-accept, send) reporting
    JSON lines on stdout; its stdin takes the pairing confirmation."""
    return subprocess.Popen([BIN, *args, "--json"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True, bufsize=1)


def subscribe():
    """A socket that receives the service's events as JSON lines (offers,
    received files, rings, messages). Answers can go back on it."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        s.connect(sock_path())
        s.sendall(b'{"cmd": "subscribe"}\n')
    except OSError:
        s.close()
        raise LinkError("Nexa Link isn't running.") from None
    return s


def human_size(n):
    n = float(n or 0)
    for unit in ("bytes", "KB", "MB", "GB"):
        if n < 1000 or unit == "GB":
            return f"{n:.0f} {unit}" if unit == "bytes" else f"{n:.1f} {unit}"
        n /= 1000
    return f"{n:.1f} GB"
