"""Run with: python3 common/tests/test_package_helper.py

Security tests for nexa-install-helper (runs as root via passwordless doas):
a .nexa package must never overwrite, shadow or delete anything it didn't
install itself, or make root copy out a file it shouldn't."""
import json, os, sys, tempfile, zipfile
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO, "installer"))
from nexa_installer import control_helper as ch, packaging

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)
def error_of(fn, *a):
    try: fn(*a)
    except packaging.PackageError as e: return str(e)
    return None

root = tempfile.mkdtemp()
def d(*parts):
    path = os.path.join(root, *parts); os.makedirs(path, exist_ok=True); return path
ch.BIN_DIR, ch.LIB_ROOT = d("usr/local/bin"), d("usr/local/lib")
ch.APPS_DIR, ch.ICON_DIR = d("usr/share/applications"), d("icons")
ch.REGISTRY_DIR = os.path.join(root, "var/lib/nexa-packages")
ch.COMMAND_DIRS = (ch.BIN_DIR, d("usr/bin"), d("bin"))
ch._refresh_caches = lambda: None

# What a real system has: a passwordless root helper, a system component's
# code and launcher, a system command, and a secret only root can read.
helper = os.path.join(ch.BIN_DIR, "nexa-setup-helper"); open(helper, "w").write("REAL HELPER")
os.makedirs(os.path.join(ch.LIB_ROOT, "nexa-setup")); open(os.path.join(ch.LIB_ROOT, "nexa-setup", "x.py"), "w").write("REAL")
open(os.path.join(ch.APPS_DIR, "nexa-setup.desktop"), "w").write("REAL")
open(os.path.join(root, "bin", "su"), "w").write("REAL SU")
secret = os.path.join(root, "shadow"); open(secret, "w").write("root:$6$SECRET")

def make_pkg(pkg_id, name="App", icon=None, lib=True, extra=None, manifest_extra=None):
    path = os.path.join(tempfile.mkdtemp(), f"{pkg_id}.nexa")
    manifest = {"id": pkg_id, "name": name, "version": "1.0", **(manifest_extra or {})}
    if icon is not None: manifest["icon"] = icon
    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr("manifest.json", json.dumps(manifest))
        zf.writestr(f"bin/{pkg_id}", "#!/bin/sh\necho hi\n")
        if lib: zf.writestr("lib/data.txt", "data")
        zf.writestr("icon.svg", "<svg/>")
        for name_, data in (extra or {}).items(): zf.writestr(name_, data)
    return path

def install(path): return ch.install({"path": path})

# Normal install
out = install(make_pkg("hello", icon="icon.svg"))
reg = json.load(open(os.path.join(ch.REGISTRY_DIR, "hello.json")))
check("normal package installs", out.startswith("Installed") and os.access(os.path.join(ch.BIN_DIR, "hello"), os.X_OK))
check("install records exactly what it wrote", set(reg["paths"]) == {
    os.path.join(ch.BIN_DIR, "hello"), os.path.join(ch.LIB_ROOT, "nexa-hello"),
    os.path.join(ch.ICON_DIR, "nexa-hello.svg"), os.path.join(ch.APPS_DIR, "nexa-hello.desktop")}, str(reg))

# Overwriting system files
check("id matching a root helper is refused", error_of(install, make_pkg("nexa-setup-helper")) is not None)
check("...and the helper is untouched", open(helper).read() == "REAL HELPER")
check("id matching a system component's code dir is refused", error_of(install, make_pkg("setup")) is not None)
check("...and its code is untouched", open(os.path.join(ch.LIB_ROOT, "nexa-setup", "x.py")).read() == "REAL")
check("id shadowing a system command is refused", "system command" in (error_of(install, make_pkg("su")) or ""))
check("refused installs leave nothing behind",
      not os.path.exists(os.path.join(ch.BIN_DIR, "su")) and not os.path.exists(os.path.join(ch.REGISTRY_DIR, "su.json")))

# Reading files through the icon field
for bad, why in ((secret, "absolute path"), ("../../../shadow", "relative escape")):
    install(make_pkg("leak", icon=bad))
    leaked = [f for f in os.listdir(ch.ICON_DIR) if f.startswith("nexa-leak")]
    check(f"icon outside the package is ignored ({why})", leaked == [], str(leaked))
    ch.uninstall({"id": "leak"})

# Desktop entry injection
install(make_pkg("inj", name="Nice\nExec=/tmp/evil\nX", manifest_extra={"categories": "Utility;\nExec=/tmp/evil2"}))
entry = open(os.path.join(ch.APPS_DIR, "nexa-inj.desktop")).read()
exec_lines = [l for l in entry.splitlines() if l.startswith("Exec=")]
check("manifest text can't add desktop-entry lines",
      exec_lines == [f"Exec={os.path.join(ch.BIN_DIR, 'inj')}"], str(exec_lines))

# Upgrade
install(make_pkg("hello", lib=False))
check("upgrading the same package is allowed and drops files it no longer ships",
      not os.path.exists(os.path.join(ch.LIB_ROOT, "nexa-hello")) and not os.path.exists(os.path.join(ch.ICON_DIR, "nexa-hello.svg")))

# Uninstall
check("uninstalling a system component is refused", error_of(ch.uninstall, {"id": "setup"}) is not None)
check("...and it is untouched", os.path.exists(os.path.join(ch.LIB_ROOT, "nexa-setup", "x.py"))
      and os.path.exists(os.path.join(ch.APPS_DIR, "nexa-setup.desktop")))
check("uninstalling a root helper's name is refused", error_of(ch.uninstall, {"id": "nexa-setup-helper"}) is not None
      and os.path.exists(helper))
ch.uninstall({"id": "hello"})
check("uninstall removes the package and its record",
      not os.path.exists(os.path.join(ch.BIN_DIR, "hello")) and not os.path.exists(os.path.join(ch.APPS_DIR, "nexa-hello.desktop"))
      and not os.path.exists(os.path.join(ch.REGISTRY_DIR, "hello.json")))
json.dump({"paths": [helper]}, open(os.path.join(ch.REGISTRY_DIR, "odd.json"), "w"))
ch.uninstall({"id": "odd"})
check("uninstall only deletes paths the id could own, even if the record lists more", os.path.exists(helper))

# Rollback on failure
icon_dir = ch.ICON_DIR
ch.ICON_DIR = os.path.join(root, "not-a-dir"); open(ch.ICON_DIR, "w").close()
try:
    install(make_pkg("half", icon="icon.svg")); raised = False
except OSError:
    raised = True
check("a failing install raises", raised)
check("a failed install leaves no files and no record",
      not os.path.exists(os.path.join(ch.BIN_DIR, "half")) and not os.path.exists(os.path.join(ch.LIB_ROOT, "nexa-half"))
      and not os.path.exists(os.path.join(ch.REGISTRY_DIR, "half.json")))
ch.ICON_DIR = icon_dir
check("...so the same package can be installed afterwards", install(make_pkg("half", icon="icon.svg")).startswith("Installed"))

# Zip slip is still rejected
check("paths escaping the archive are rejected",
      error_of(install, make_pkg("slip", extra={"../../escape.txt": "x"})) is not None)

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
