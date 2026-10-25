"""nexa_common.sessions against a stand-in nexa-sessiond on a real socket."""
import os
import socket
import sys
import tempfile
import threading

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from nexa_common import sessions  # noqa: E402

failures = 0


def check(what, ok):
    global failures
    print(("PASS " if ok else "FAIL ") + what)
    failures += not ok


d = tempfile.mkdtemp()
path = os.path.join(d, "s.sock")
got = []
REPLY = {"people": "ok\nuser\t1000\tNexa User\t1\t1\nama\t1001\tAma Mensah\t1\t0\nkofi\t1002\tKofi\t0\t0\nbad line\n"}


def serve(srv):
    while True:
        c, _ = srv.accept()
        line = c.makefile().readline().strip()
        got.append(line)
        c.sendall(REPLY.get(line, "error unknown request\n").encode())
        c.close()


srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(path)
srv.listen(4)
threading.Thread(target=serve, args=(srv,), daemon=True).start()
os.environ["NEXA_SESSION_SOCKET"] = path

ppl = sessions.people()
check("people: everyone, malformed lines skipped", [p["name"] for p in ppl] == ["user", "ama", "kofi"])
check("people: who is signed in and on screen",
      ppl[0]["on_screen"] and ppl[1]["signed_in"] and not ppl[1]["on_screen"] and not ppl[2]["signed_in"])
check("people: display names", ppl[1]["display"] == "Ama Mensah")
check("others: without you", [p["name"] for p in sessions.others(my_uid=1000)] == ["ama", "kofi"])
check("the request is one line", got[0] == "people")
REPLY["people"] = "error broken\n"
check("an error reply: nobody", sessions.people() == [])
os.environ["NEXA_SESSION_SOCKET"] = os.path.join(d, "missing.sock")
check("no nexa-sessiond: nobody (Switch person hidden)", sessions.people() == [] and sessions.others() == [])
sys.exit(1 if failures else 0)
