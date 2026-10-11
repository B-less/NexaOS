import json
import sys

from nexa_common import diskinstall

# Runs as root via `doas /usr/local/bin/nexa-setup-helper` (see
# /etc/doas.conf). Unlike every other helper in this project, "install" is
# inherently destructive by design (it wipes a whole disk) rather than a
# narrow allowlisted action -- diskinstall.validate_disk() is what keeps it
# from ever touching the disk the live session itself booted from, or a disk
# too small to hold the system. Streams one {"stage": ...} JSON line per
# install stage to stdout (flushed immediately) so the GUI can show real
# progress on what is normally a multi-minute operation, then one final
# {"ok": ...} line. The request (on stdin, so the password never appears in
# argv) is {"disk", "password", "hostname", "zone", "keyboard"}.


def _report(stage):
    print(json.dumps({"stage": stage}), flush=True)


def main():
    try:
        req = json.loads(sys.stdin.read())
        disk = req["disk"]
        # Required here (not just in the GUI) so no request can produce an
        # installed system with the live image's empty passwords.
        password = req["password"]
        if not isinstance(password, str):
            raise TypeError("password must be a string")
    except (json.JSONDecodeError, KeyError, TypeError):
        print(json.dumps({"ok": False, "error": "malformed request"}))
        return 1
    try:
        result = diskinstall.run_install(
            disk, req.get("hostname"), req.get("zone"), req.get("keyboard"),
            on_stage=_report, password=password,
        )
    except diskinstall.InstallError as e:
        print(json.dumps({"ok": False, "error": str(e)}), flush=True)
        return 1
    print(json.dumps({"ok": True, "result": result}), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
