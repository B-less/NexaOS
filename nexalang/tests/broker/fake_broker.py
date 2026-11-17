"""A stand-in for nexa-brokerd used by the tests: answers each JSON line
with a JSON line, speaking the real broker's protocol
(tools/nexa-brokerd/src/main.rs):
  {"ok": true, "result": ...}
  {"ok": false, "error": "...", "needs": "password", "what": "..."}
  {"ok": false, "error": "...", "unverified": true}
  {"ok": false, "error": "..."}
Three wrong passwords lock the asker out, like the real one. Not part of NexaOS."""
import json, os, socket, sys

PASSWORD = "open sesame"
path = sys.argv[1]
if os.path.exists(path):
    os.unlink(path)
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(path)
srv.listen(8)
print("ready", flush=True)
failures = 0
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
        args = req.get("args")
        if not isinstance(args, dict):
            raise ValueError("args must be an object")
        if action == "stop":
            conn.sendall(b'{"ok": true, "result": null}\n'); conn.close(); break
        if action == "set_brightness":
            p = args["percent"]
            reply = {"ok": True, "result": {"undo": {"action": "set_brightness", "args": {"percent": 70}}, "got": p}}
        elif action == "set_hostname":
            what = f"Rename this computer to {args.get('name')}"
            if failures >= 3:
                reply = {"ok": False, "error": "too many wrong passwords; try again in 60 s", "needs": "password"}
            elif "password" not in req:
                reply = {"ok": False, "error": "needs your password", "needs": "password", "what": what}
            elif req["password"] != PASSWORD:
                failures += 1
                reply = {"ok": False, "error": "wrong password", "needs": "password", "what": what}
            else:
                failures = 0
                reply = {"ok": True, "result": {"undo": {"action": "set_hostname", "args": {"name": "nexa"}}}}
        elif action == "install_app":
            reply = {"ok": False, "error": "signed by an unknown publisher", "unverified": True}
        elif action == "locked":
            reply = {"ok": False, "error": "this account's password is locked"}
        elif action == "no_result":
            reply = {"ok": True}
        elif action == "not_object":
            conn.sendall(b'[1, 2]\n'); conn.close(); continue
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
