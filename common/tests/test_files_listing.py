"""Files hides folders this account can't open (/root, lost+found): they
only led to a permission error. Ctrl+H (hidden items) still shows them;
opening one directly says why it can't. Run with: python3 common/tests/test_files_listing.py"""
import os
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "file-manager"))
from nexa_files import fileops  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


if os.geteuid() == 0:
    print("SKIP running as root: every folder can be opened")
    sys.exit(0)
d = tempfile.mkdtemp()
os.mkdir(os.path.join(d, "open"))
os.mkdir(os.path.join(d, "private"))
open(os.path.join(d, "note.txt"), "w").close()
os.chmod(os.path.join(d, "private"), 0)
try:
    names = [e["name"] for e in fileops.list_dir(d)]
    check("a folder the account can't open isn't listed", names == ["open", "note.txt"], str(names))
    names = [e["name"] for e in fileops.list_dir(d, show_hidden=True)]
    check("...but is with hidden items shown", "private" in names, str(names))
    try:
        fileops.list_dir(os.path.join(d, "private"))
        check("opening it says why it can't", False)
    except fileops.FileOpError as e:
        check("opening it says why it can't", "belongs to the system" in str(e), str(e))
finally:
    os.chmod(os.path.join(d, "private"), 0o755)

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
