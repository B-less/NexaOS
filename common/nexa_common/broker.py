"""Client for nexa-brokerd, NexaOS's system broker (Rust, tools/nexa-brokerd).

Every change to the system as root goes through it: one JSON line to
/run/nexa-broker.sock, one back. The broker checks who is asking, the
action and its arguments, and the password when the action needs one;
the work happens in a Landlock-confined worker and every request is logged
(/var/log/nexa-broker.log). Python never runs as root for these.
"""
import json
import os
import socket

SOCKET = os.environ.get("NEXA_BROKER_SOCKET", "/run/nexa-broker.sock")


class BrokerError(Exception):
    pass


class Unverified(BrokerError):
    """An app package isn't signed by a trusted publisher; the installer may
    ask the user and retry with allow_unverified."""


class NeedsPassword(BrokerError):
    """The action needs the user's password (or the one given was wrong)."""
    def __init__(self, message, what=None):
        super().__init__(message)
        self.what = what


def request(action, args=None, password=None, timeout=1900):
    req = {"action": action, "args": args or {}}
    if password is not None:
        req["password"] = password
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(timeout)
            s.connect(SOCKET)
            s.sendall((json.dumps(req) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = s.recv(65536)
                if not chunk:
                    break
                data += chunk
    except (FileNotFoundError, ConnectionRefusedError):
        raise BrokerError("NexaOS's system service isn't running, so this can't be done right now. Restarting the computer usually fixes it.")
    except socket.timeout:
        raise BrokerError("the system broker didn't answer in time")
    try:
        resp = json.loads(data)
    except json.JSONDecodeError:
        raise BrokerError("the system broker gave no answer")
    if resp.get("ok"):
        return resp.get("result")
    if resp.get("needs") == "password":
        raise NeedsPassword(resp.get("error", "needs your password"), resp.get("what"))
    if resp.get("unverified"):
        raise Unverified(resp.get("error"))
    raise BrokerError(resp.get("error") or "the system broker refused")


def run(action, args=None, ask_password=None):
    """request(), asking for the password when the broker needs one.
    ask_password(what, error) returns the password or None (cancelled)."""
    try:
        return request(action, args)
    except NeedsPassword as e:
        if ask_password is None:
            raise
        what, error = e.what, None
        for _ in range(3):
            pw = ask_password(what, error)
            if pw is None:
                raise BrokerError("cancelled")
            try:
                return request(action, args, password=pw)
            except NeedsPassword as again:
                error = str(again)
                if "too many" in error:
                    raise BrokerError(error)
        raise BrokerError(error or "wrong password")
