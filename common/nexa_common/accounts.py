"""The people on this computer, for Settings > Users: read straight from
/etc/passwd and /etc/group (no root needed). Changing anything goes
through the system broker (add_user, remove_user, change_password)."""
import os

ADMIN_GROUP = "wheel"


def list_people(etc="/etc", my_uid=None):
    """Accounts of people (uid 1000-59999), you first: name, full name,
    uid, whether they're an administrator, whether it's you."""
    my_uid = os.getuid() if my_uid is None else my_uid
    admins = set()
    try:
        with open(os.path.join(etc, "group")) as f:
            for line in f:
                parts = line.rstrip("\n").split(":")
                if len(parts) >= 4 and parts[0] == ADMIN_GROUP:
                    admins = {m for m in parts[3].split(",") if m}
    except OSError:
        pass
    people = []
    try:
        with open(os.path.join(etc, "passwd")) as f:
            for line in f:
                parts = line.rstrip("\n").split(":")
                if len(parts) < 7 or not parts[2].isdigit():
                    continue
                uid = int(parts[2])
                if not 1000 <= uid < 60000:
                    continue
                full = parts[4].split(",")[0].strip()
                people.append({"name": parts[0], "full_name": full or parts[0], "uid": uid,
                               "admin": parts[0] in admins, "you": uid == my_uid})
    except OSError:
        pass
    return sorted(people, key=lambda p: (not p["you"], p["full_name"].lower()))


def check_new_account(name, password, confirm):
    """What's wrong with a new account, in plain words, or None."""
    import re
    if not re.fullmatch(r"[a-z][a-z0-9_-]{0,30}", name or ""):
        return "User names start with a small letter and use only a-z, 0-9, - and _."
    if len(password or "") < 4:
        return "The password needs at least 4 characters."
    if password != confirm:
        return "The two passwords aren't the same."
    return None
