"""A stand-in for nexa-brokerd used by the tests: answers each JSON line
with a JSON line, the way the real broker does. Not part of NexaOS."""
import json, os, socket, sys

path = sys.argv[1]
if os.path.exists(path):
    os.unlink(path)
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(path)
srv.listen(8)
print("ready", flush=True)
while True:
    conn, _ = srv.accept()
    data = b""
    while not data.endswith(b"\n"):
        chunk = conn.recv(4096)
        if not chunk:
            break
        data += chunk
    try:
        req = json.loads(data)
        action = req.get("action")
        if action == "stop":
            conn.sendall(b'{"ok": true}\n'); conn.close(); break
        if action == "set_brightness":
            p = req["args"]["percent"]
            reply = {"ok": True, "undo": {"action": "set_brightness", "args": {"percent": 70}}, "got": p}
        elif action == "garbage":
            conn.sendall(b"not json at all\n"); conn.close(); continue
        elif action == "silent":
            conn.close(); continue
        else:
            reply = {"ok": False, "error": f"unknown action {action}"}
    except Exception as e:
        reply = {"ok": False, "error": f"bad request: {e}"}
    conn.sendall((json.dumps(reply) + "\n").encode())
    conn.close()
