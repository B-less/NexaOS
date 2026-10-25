"""Switch person, for the desktop's own screens (Power, Settings > Users).

nexa-sessiond runs everyone's desktop, one per person, each on its own VT;
it answers one line on /run/nexa-session.sock. people() says who may sign
in and who is; switch_to() hands the screen over through the lock screen
(nexa-lock --switch-to), which locks this desktop first, plays the
hand-over and then asks nexa-sessiond to put the other desktop on screen.
"""
import os
import socket
import subprocess

SOCKET = "/run/nexa-session.sock"


def _path():
    return os.environ.get("NEXA_SESSION_SOCKET") or SOCKET


def request(line, timeout=3.0):
    """One request; the whole reply. OSError if nexa-sessiond isn't there."""
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.settimeout(timeout)
        s.connect(_path())
        s.sendall((line + "\n").encode())
        s.shutdown(socket.SHUT_WR)
        chunks = []
        while True:
            b = s.recv(4096)
            if not b:
                break
            chunks.append(b)
    return b"".join(chunks).decode("utf-8", "replace")


def people():
    """Everyone who may use the desktop: name, uid, display name, signed_in,
    on_screen. Empty when switching isn't available here."""
    try:
        text = request("people")
    except OSError:
        return []
    lines = text.splitlines()
    if not lines or lines[0] != "ok":
        return []
    out = []
    for line in lines[1:]:
        f = line.split("\t")
        if len(f) != 5 or not f[1].isdigit():
            continue
        out.append({"name": f[0], "uid": int(f[1]), "display": f[2],
                    "signed_in": f[3] == "1", "on_screen": f[4] == "1"})
    return out


def others(my_uid=None):
    """people() without you."""
    me = os.getuid() if my_uid is None else my_uid
    return [p for p in people() if p["uid"] != me]


def switch_to(name):
    """Starts the hand-over to `name` (the lock screen takes it from here).
    Returns None, or why it couldn't start."""
    try:
        subprocess.Popen(["nexa-lock", "--switch-to", name], start_new_session=True,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError as e:
        return f"couldn't start the lock screen: {e}"
    return None
