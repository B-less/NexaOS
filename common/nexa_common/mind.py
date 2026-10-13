"""Client for nexa-mindd, the on-device AI service (tools/nexa-mindd, Rust),
over its Unix socket. Used by the assistant (nexa_ai.local_client) and by
Settings (model download, status). The socket's group (nexa-mind) and the
service's SO_PEERCRED check decide who may use it; no keys involved."""
import http.client
import json
import os
import socket

SOCKET = os.environ.get("NEXA_MIND_SOCKET", "/run/nexa-mind/mind.sock")


class MindError(Exception):
    """The service answered with an error (status, message)."""
    def __init__(self, status, message):
        super().__init__(message)
        self.status = status


class MindUnreachable(Exception):
    pass


class _UnixHTTPConnection(http.client.HTTPConnection):
    def __init__(self, path, timeout):
        super().__init__("localhost", timeout=timeout)
        self._path = path

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(self._path)


def request(method, path, body=None, timeout=600):
    conn = _UnixHTTPConnection(SOCKET, timeout)
    try:
        conn.request(method, path, body=json.dumps(body) if body is not None else None,
                     headers={"Content-Type": "application/json"})
        resp = conn.getresponse()
        data = resp.read()
    except (OSError, http.client.HTTPException) as e:
        raise MindUnreachable(f"Nexa Mind isn't reachable: {e}")
    finally:
        conn.close()
    try:
        payload = json.loads(data or b"null")
    except ValueError:
        raise MindError(resp.status, f"Nexa Mind sent something unreadable (HTTP {resp.status})")
    if resp.status not in (200, 202):
        detail = payload.get("error") if isinstance(payload, dict) else payload
        raise MindError(resp.status, f"Nexa Mind: {detail}")
    return payload


def status():
    """The service's status dict, or None if it isn't running."""
    try:
        return request("GET", "/v1/status", timeout=5)
    except (MindUnreachable, MindError):
        return None


def models():
    try:
        return request("GET", "/v1/models", timeout=5)
    except (MindUnreachable, MindError):
        return []


def download(name):
    return request("POST", "/v1/models/download", {"name": name}, timeout=10)


def available():
    """True when the service runs and its default chat model is installed."""
    st = status()
    return bool(st and st.get("default_model") in (st.get("installed") or []))
