"""Run with: python3 common/tests/test_accounts.py

Settings > Users reads people straight from /etc/passwd and /etc/group."""
import os, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from nexa_common import accounts

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

etc = tempfile.mkdtemp()
open(os.path.join(etc, "passwd"), "w").write(
    "root:x:0:0:root:/root:/bin/sh\nnexa-mind:x:110:110:Nexa Mind:/var/lib/nexa-mind:/sbin/nologin\n"
    "kofi:x:1001:1001:Kofi Mensah,,,:/home/kofi:/bin/sh\nama:x:1000:1000::/home/ama:/bin/sh\nnobody:x:65534:65534::/:/sbin/nologin\n")
open(os.path.join(etc, "group"), "w").write("wheel:x:10:ama\nvideo:x:27:ama,kofi\n")
people = accounts.list_people(etc, my_uid=1001)
check("only people (uid 1000-59999), you first", [p["name"] for p in people] == ["kofi", "ama"], str(people))
check("full name from GECOS, else the user name", people[0]["full_name"] == "Kofi Mensah" and people[1]["full_name"] == "ama")
check("administrators come from the wheel group", people[1]["admin"] and not people[0]["admin"])
check("you are marked", people[0]["you"] and not people[1]["you"])
check("missing files: nobody, no crash", accounts.list_people(os.path.join(etc, "nope")) == [])
for name, pw, again, want in [("ama2", "secret1", "secret1", None), ("Ama", "secret1", "secret1", "small letter"),
                              ("ama", "abc", "abc", "at least 4"), ("ama", "secret1", "secret2", "aren't the same"),
                              ("a b", "secret1", "secret1", "small letter")]:
    got = accounts.check_new_account(name, pw, again)
    check(f"new account {name!r}: {want or 'ok'}", (got is None) if want is None else (got is not None and want in got), str(got))
print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
