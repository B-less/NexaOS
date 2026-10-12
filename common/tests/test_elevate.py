"""Run with: python3 common/tests/test_elevate.py

elevate.py against a fake doas that, like OpenDoas, refuses -n when a
password is needed and reads the password from the controlling terminal."""
import json, os, stat, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from nexa_common import elevate

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

d = tempfile.mkdtemp()
def script(name, body):
    path = os.path.join(d, name)
    open(path, "w").write("#!" + sys.executable + "\n" + body)
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
    return path

# The "helper": echoes back the request it got on stdin, as root helpers do.
helper = script("helper", "import sys, json\nreq = json.load(sys.stdin)\nprint(json.dumps({'ok': True, 'got': req}))\n")
secure_doas = script("doas", """import os, sys
args = sys.argv[1:]
if args[0] == '-n':
    sys.stderr.write('doas: Authentication required\\n'); sys.exit(1)
fd = os.open('/dev/tty', os.O_RDWR)
os.write(fd, b'doas (user@nexa) password: ')
line = b''
while not line.endswith(b'\\n'):
    chunk = os.read(fd, 64)
    if not chunk: break
    line += chunk
if line.strip() != b'hunter22':
    sys.stderr.write('doas: Authentication failed\\n'); sys.exit(1)
os.execv(args[0], args)
""")
open_doas = script("doas-open", """import os, sys
args = [a for a in sys.argv[1:] if a != '-n']
os.execv(args[0], args)
""")
request = json.dumps({"action": "install", "args": {"package": "htop"}})

elevate.DOAS = secure_doas
try:
    elevate.run(helper, request); raised = None
except elevate.PasswordRequired:
    raised = "required"
check("password-protected helper: no password -> PasswordRequired", raised == "required")

p = elevate.run(helper, request, password="hunter22")
check("right password over the terminal runs the helper with the request on stdin",
      p.returncode == 0 and json.loads(p.stdout)["got"]["args"]["package"] == "htop", p.stdout + p.stderr)
check("password never reaches the helper's input or output", "hunter22" not in p.stdout)

try:
    elevate.run(helper, request, password="nope"); raised = None
except elevate.WrongPassword:
    raised = "wrong"
check("wrong password -> WrongPassword", raised == "wrong")

asked = []
def ask(retry):
    asked.append(retry)
    return "nope" if len(asked) == 1 else "hunter22"
p = elevate.run_interactive(helper, request, ask)
check("interactive: asks, retries after a wrong password, then succeeds",
      asked == [False, True] and p.returncode == 0, str(asked))
check("interactive: cancel returns None", elevate.run_interactive(helper, request, lambda retry: None) is None)
try:
    elevate.run_interactive(helper, request, lambda retry: "nope"); raised = None
except elevate.WrongPassword:
    raised = "gave up"
check(f"interactive: gives up after {elevate.MAX_ATTEMPTS} wrong passwords", raised == "gave up")

elevate.DOAS = open_doas
asked.clear()
p = elevate.run_interactive(helper, request, ask)
check("passwordless rule (live USB): runs straight away, never asks", p.returncode == 0 and asked == [])

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
