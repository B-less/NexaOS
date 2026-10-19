"""Run with: python3 common/tests/test_removable.py

The Python side of the system broker (nexa_common/broker.py) and its users
(removable drives), against a fake broker on a test socket: answers,
errors, asking for the password, retrying after a wrong one, cancelling,
the lockout, and untrusted app packages. The broker itself is tested in
Rust (tools/nexa-brokerd)."""
import json, os, socket, sys, tempfile, threading
sock_path = os.path.join(tempfile.mkdtemp(), "broker.sock")
os.environ["NEXA_BROKER_SOCKET"] = sock_path
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from nexa_common import broker, removable

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

seen = []
answer = {"fn": lambda req: {"ok": True, "result": {}}}

# Listening before the thread starts: the socket file appears at bind(), a
# moment before listen(), and a connect in between was refused (flaky test).
srv = socket.socket(socket.AF_UNIX); srv.bind(sock_path); srv.listen(8)

def serve():
    while True:
        c, _ = srv.accept()
        req = json.loads(c.makefile().readline())
        seen.append(req)
        c.sendall((json.dumps(answer["fn"](req)) + "\n").encode()); c.close()
threading.Thread(target=serve, daemon=True).start()

# ---- drives
answer["fn"] = lambda req: {"ok": True, "result": {"drives": [{"name": "sdb1", "label": "PHOTOS", "size_bytes": 8000000000, "mountpoint": None, "fstype": "vfat"}]}}
vols = removable.list_volumes()
check("drives: the broker's list, asked as list_drives", vols[0]["label"] == "PHOTOS" and seen[-1]["action"] == "list_drives")
check("label is the display name", removable.display_name(vols[0]) == "PHOTOS")
check("unlabelled drive shows its size", removable.display_name({"label": "", "size_bytes": 8 * 1024**3}) == "Drive (8.0 GB)")
answer["fn"] = lambda req: {"ok": False, "error": "The drive is still in use - close anything using it and try again."}
try: removable.unmount("sdb1"); check("broker error becomes RemovableError", False)
except removable.RemovableError as e: check("broker error becomes RemovableError with its message", "still in use" in str(e) and seen[-1] == {"action": "unmount_drive", "args": {"name": "sdb1"}}, str(seen[-1]))

# ---- passwords
def needs_secret(req):
    if req.get("password") == "secret": return {"ok": True, "result": {"done": "Renamed"}}
    return {"ok": False, "needs": "password", "what": "Rename this computer to studio",
            "error": "wrong password" if "password" in req else "needs your password"}
answer["fn"] = needs_secret
asked = []
def ask(what, error):
    asked.append((what, error)); return ["guess", "secret"][len(asked) - 1]
r = broker.run("set_hostname", {"name": "studio"}, ask)
check("asks for the password with what it's for, retries after a wrong one", r == {"done": "Renamed"}
      and asked == [("Rename this computer to studio", None), ("Rename this computer to studio", "wrong password")], str(asked))
check("the password is sent only after asking", "password" not in seen[-3] and seen[-1]["password"] == "secret")
try: broker.run("set_hostname", {"name": "studio"}, lambda w, e: None); check("cancel", False)
except broker.BrokerError as e: check("cancelling the password prompt stops", str(e) == "cancelled")
try: broker.run("set_hostname", {"name": "studio"}); check("no prompt", False)
except broker.NeedsPassword: check("without a prompt, NeedsPassword is raised to the caller", True)
answer["fn"] = lambda req: {"ok": False, "needs": "password", "error": "too many wrong passwords; try again in 60 s"} if "password" in req else needs_secret(req)
try: broker.run("set_hostname", {"name": "x"}, lambda w, e: "guess"); check("lockout", False)
except broker.BrokerError as e: check("a lockout ends the retries with its message", "too many" in str(e), str(e))

# ---- apps
answer["fn"] = lambda req: {"ok": False, "unverified": True, "error": "Thing isn't signed."}
try: broker.request("install_app", {"path": "/x.nexa"}); check("unverified", False)
except broker.Unverified as e: check("an untrusted package is reported as Unverified", "isn't signed" in str(e))

# ---- no broker
os.environ["NEXA_BROKER_SOCKET"] = "/nonexistent/sock"
broker.SOCKET = "/nonexistent/sock"
try: broker.request("list_drives"); check("down", False)
except broker.BrokerError as e: check("a missing broker says so plainly", "isn't running" in str(e))

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
