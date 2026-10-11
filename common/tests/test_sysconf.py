"""Run with: python3 common/tests/test_sysconf.py"""
import os
import stat
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.dirname(HERE))
from nexa_common import sysconf

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


tmp = tempfile.mkdtemp()
zi = os.path.join(tmp, "zoneinfo")
for z in ("UTC", "Africa/Accra", "America/Argentina/Buenos_Aires", "posix/UTC", "right/UTC"):
    os.makedirs(os.path.dirname(os.path.join(zi, z)), exist_ok=True)
    open(os.path.join(zi, z), "wb").write(b"TZif2" + b"\0" * 20)
open(os.path.join(zi, "zone.tab"), "w").write("# not a zone\n")
os.makedirs(os.path.join(tmp, "outside"), exist_ok=True)
open(os.path.join(tmp, "outside", "Evil"), "wb").write(b"TZif2")
os.symlink(os.path.join(tmp, "outside", "Evil"), os.path.join(zi, "Escape"))

check("real zone accepted", sysconf.valid_zone("Africa/Accra", zi))
check("three-level zone accepted", sysconf.valid_zone("America/Argentina/Buenos_Aires", zi))
check("no-slash zone accepted", sysconf.valid_zone("UTC", zi))
check("non-zone file (zone.tab) rejected", not sysconf.valid_zone("zone.tab", zi))
check("path traversal rejected", not sysconf.valid_zone("../outside/Evil", zi))
check("symlink escaping the tree rejected", not sysconf.valid_zone("Escape", zi))
check("posix/ and right/ variants rejected", not sysconf.valid_zone("posix/UTC", zi) and not sysconf.valid_zone("right/UTC", zi))
check("garbage and non-strings rejected", not sysconf.valid_zone("Africa/../../etc/passwd", zi) and not sysconf.valid_zone(None, zi) and not sysconf.valid_zone("A B", zi))
check("list_zones finds exactly the real zones",
      sysconf.list_zones(zi) == ["Africa/Accra", "America/Argentina/Buenos_Aires", "UTC"], str(sysconf.list_zones(zi)))

lt = os.path.join(tmp, "localtime")
os.symlink(os.path.join(zi, "Africa/Accra"), lt)
check("current_zone follows the /etc/localtime link", sysconf.current_zone(lt, os.path.join(tmp, "none"), zi) == "Africa/Accra")
os.remove(lt)
tzf = os.path.join(tmp, "timezone"); open(tzf, "w").write("America/Argentina/Buenos_Aires\n")
check("current_zone falls back to /etc/timezone", sysconf.current_zone(lt, tzf, zi) == "America/Argentina/Buenos_Aires")
check("current_zone defaults to UTC", sysconf.current_zone(lt, os.path.join(tmp, "none"), zi) == "UTC")

lst = os.path.join(tmp, "base.lst")
open(lst, "w").write("! model\n  pc105    Generic 105-key\n\n! layout\n  us              English (US)\n  gh              English (Ghana)\n  de              German\n\n! variant\n  euro     us: English (US, euro on 5)\n")
layouts = sysconf.list_layouts(lst)
check("layouts parsed from only the layout section", layouts == [("us", "English (US)"), ("gh", "English (Ghana)"), ("de", "German")], str(layouts))
check("missing list falls back to us", sysconf.list_layouts(os.path.join(tmp, "nope")) == [("us", "English (US)")])
check("valid_layout allows listed codes only", sysconf.valid_layout("gh", layouts) and not sysconf.valid_layout("zz", layouts) and not sysconf.valid_layout('us"; exec x', layouts))

kb = os.path.join(tmp, "cfg", "sway", "config.d", "10-nexa-keyboard.conf")
check("default layout is us when unset", sysconf.read_keyboard_layout(kb) == "us")
sysconf.write_keyboard_layout("gh", kb)
check("layout round-trips through the sway include file", sysconf.read_keyboard_layout(kb) == "gh")
check("sway include text is valid", open(kb).read() == 'input type:keyboard {\n    xkb_layout "gh"\n}\n', open(kb).read())
try:
    sysconf.write_keyboard_layout('us"\nexec bad', kb); check("injection attempt rejected", False)
except ValueError:
    check("layout injection attempt rejected", True)

idle = os.path.join(tmp, "cfg", "nexa", "idle.conf")
check("idle defaults", sysconf.read_idle(idle) == {"lock_minutes": 5, "blank_minutes": 10})
check("idle round-trip + clamp", sysconf.write_idle(7, 9999, idle) == {"lock_minutes": 7, "blank_minutes": 240} and sysconf.read_idle(idle) == {"lock_minutes": 7, "blank_minutes": 240})

# nexa-idle wrapper, with a fake swayidle that records its argv
bindir = os.path.join(tmp, "bin"); os.makedirs(bindir)
fake = os.path.join(bindir, "swayidle")
open(fake, "w").write('#!/bin/sh\nfor a in "$@"; do printf "[%s]" "$a"; done; echo\n')
os.chmod(fake, os.stat(fake).st_mode | stat.S_IEXEC)


def run_idle(conf_text):
    home = tempfile.mkdtemp(); conf = os.path.join(home, ".config", "nexa", "idle.conf")
    if conf_text is not None:
        os.makedirs(os.path.dirname(conf)); open(conf, "w").write(conf_text)
    r = subprocess.run(["sh", os.path.join(ROOT, "power", "nexa-idle")], capture_output=True, text=True,
                       env={"PATH": bindir + ":/usr/bin:/bin", "HOME": home})
    return r.returncode, r.stdout.strip()


rc, out = run_idle(None)
check("idle wrapper defaults: lock 300s, screen off 600s",
      out == '[-w][timeout][300][nexa-lock][timeout][600]["swaymsg "output * power off""][resume]["swaymsg "output * power on""]'.replace('"swaymsg "output * power off""', 'swaymsg "output * power off"').replace('"swaymsg "output * power on""', 'swaymsg "output * power on"'), out)
rc, out = run_idle("lock_minutes=08\nblank_minutes=0\n")
check("leading zeros handled, blank disabled", out == "[-w][timeout][480][nexa-lock]", out)
rc, out = run_idle("lock_minutes=0\nblank_minutes=0\n")
check("both disabled -> swayidle not started, clean exit", rc == 0 and out == "", f"{rc} {out}")
rc, out = run_idle("lock_minutes=abc\nblank_minutes=3;rm -rf x\n")
check("garbage values ignored, defaults used", "[300]" in out and "[600]" in out, out)

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
