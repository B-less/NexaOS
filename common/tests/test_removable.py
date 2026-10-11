"""Run with: python3 common/tests/test_removable.py"""
import os, stat, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from nexa_common import removable

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

bindir = tempfile.mkdtemp()
def fake_doas(body):
    path = os.path.join(bindir, "doas")
    open(path, "w").write("#!/bin/sh\n" + body + "\n"); os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
os.environ["PATH"] = bindir + ":" + os.environ["PATH"]

fake_doas('cat >/dev/null; echo \'{"ok": true, "result": [{"name": "sdb1", "label": "PHOTOS", "size_bytes": 8000000000, "mountpoint": null, "fstype": "vfat"}]}\'')
vols = removable.list_volumes()
check("list returns the helper's volumes", vols[0]["name"] == "sdb1" and vols[0]["label"] == "PHOTOS")
check("label is the display name", removable.display_name(vols[0]) == "PHOTOS")
check("unlabelled drive shows its size", removable.display_name({"label": "", "size_bytes": 8 * 1024**3}) == "Drive (8.0 GB)")

fake_doas('cat >/dev/null; echo \'{"ok": false, "error": "The drive is still in use - close anything using it and try again."}\'')
try: removable.unmount("sdb1"); check("helper error becomes RemovableError", False)
except removable.RemovableError as e: check("helper error becomes RemovableError with its message", "still in use" in str(e), str(e))

fake_doas('cat >/dev/null; echo "not json"; exit 1')
try: removable.eject("sdb1"); check("garbage output handled", False)
except removable.RemovableError: check("garbage helper output handled", True)

fake_doas('cat > "$0.request"; echo \'{"ok": true, "result": {"mountpoint": "/media/X", "fstype": "vfat"}}\'')
removable.mount("sdb1")
import json
req = json.load(open(os.path.join(bindir, "doas.request")))
check("request carries only the action and device name", req == {"action": "mount", "name": "sdb1"}, str(req))

os.environ["PATH"] = "/nonexistent"
try: removable.list_volumes(); check("missing doas handled", False)
except removable.RemovableError as e: check("missing doas handled", "doas" in str(e), str(e))
print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
