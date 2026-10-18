"""Client for nexa-kerneld, Nexa Mind's link to the kernel (Rust,
tools/nexa-kerneld): pressure stalls, crashes and out-of-memory kills as
they happen, power drawn and by which programs, and the AI's own CPU share.

One JSON line to /run/nexa-kerneld.sock, one back. You see your own
programs' events and energy; administrators see everyone's.
"""
import json
import os
import socket

SOCKET = os.environ.get("NEXA_KERNELD_SOCKET", "/run/nexa-kerneld.sock")


class KernelError(Exception):
    pass


def request(op, timeout=5, **args):
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(timeout)
            s.connect(SOCKET)
            s.sendall((json.dumps(dict(op=op, **args)) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = s.recv(65536)
                if not chunk:
                    break
                data += chunk
    except OSError as e:
        raise KernelError(f"the kernel link (nexa-kerneld) isn't running: {e}")
    try:
        reply = json.loads(data)
    except ValueError:
        raise KernelError("the kernel link gave a broken answer")
    if not reply.get("ok"):
        raise KernelError(reply.get("error") or "the kernel link refused")
    return reply["result"]


def status():
    return request("status")


def events(limit=20):
    return request("events", limit=limit)


def energy():
    return request("energy")
