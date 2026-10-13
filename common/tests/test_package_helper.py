"""Run with: python3 common/tests/test_package_helper.py   (needs build/native/nexa-pack: make native)

Security tests for nexa-install-helper (runs as root): packages are checked
by the native verifier (nexa-pack) before anything is written; a .nexa must
never overwrite, shadow or delete anything it didn't install itself, never
install unverified without the user agreeing, and tampering always fails."""
import hashlib, json, os, subprocess, sys, tempfile, zipfile
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO, "installer"))
from nexa_installer import control_helper as ch, packaging

NEXA_PACK = os.path.join(REPO, "build", "native", "nexa-pack")
if not os.access(NEXA_PACK, os.X_OK):
    print("FAIL build/native/nexa-pack is missing: run `make native` first")
    sys.exit(1)

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
ch.NEXA_PACK, ch.KEYS_DIR = NEXA_PACK, d("etc/nexa/keys")
ch._refresh_caches = lambda: None
installed_deps = []
ch._install_depends = lambda names: installed_deps.extend(names)

# What a real system has: a root helper, a system component's code and
# launcher, a system command.
helper = os.path.join(ch.BIN_DIR, "nexa-setup-helper"); open(helper, "w").write("REAL HELPER")
os.makedirs(os.path.join(ch.LIB_ROOT, "nexa-setup")); open(os.path.join(ch.LIB_ROOT, "nexa-setup", "x.py"), "w").write("REAL")
open(os.path.join(ch.APPS_DIR, "nexa-setup.desktop"), "w").write("REAL")
open(os.path.join(root, "bin", "su"), "w").write("REAL SU")

def make_pkg(pkg_id, name="App", icon=True, lib=True, extra=None, manifest_extra=None, tamper=None):
    """A format-2 package built the way nexa-pack lays them out (unsigned)."""
    files = {f"bin/{pkg_id}": b"#!/bin/sh\necho hi\n"}
    if lib: files["lib/data.txt"] = b"data"
    if icon: files["icon.svg"] = b"<svg/>"
    manifest = {"format": 2, "id": pkg_id, "name": name, "version": "1.0", "description": "", "categories": "Utility;",
                "arch": "x86_64", "abi": "musl", "depends": [], "permissions": [], "terminal": False,
                "files": {p: hashlib.sha256(b).hexdigest() for p, b in sorted(files.items())}}
    if icon: manifest["icon"] = "icon.svg"
    manifest.update(manifest_extra or {})
    path = os.path.join(tempfile.mkdtemp(), f"{pkg_id}.nexa")
    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr("manifest.json", json.dumps(manifest, indent=2))
        for p, b in sorted({**files, **(tamper or {})}.items()):
            zf.writestr(p, b)
        for p, b in (extra or {}).items():
            zf.writestr(p, b)
    return path

def install(path, allow=True): return ch.install({"path": path, "allow_unverified": allow})

# Normal (unsigned, user agreed) install
out = install(make_pkg("hello"))
reg = json.load(open(os.path.join(ch.REGISTRY_DIR, "hello.json")))
check("package installs once the user agrees", out.startswith("Installed") and os.access(os.path.join(ch.BIN_DIR, "hello"), os.X_OK))
check("install records exactly what it wrote, and how it was verified", set(reg["paths"]) == {
    os.path.join(ch.BIN_DIR, "hello"), os.path.join(ch.LIB_ROOT, "nexa-hello"),
    os.path.join(ch.ICON_DIR, "nexa-hello.svg"), os.path.join(ch.APPS_DIR, "nexa-hello.desktop")} and reg["trust"] == "unsigned", str(reg))

# Trust
try:
    install(make_pkg("nosig"), allow=False); unv = None
except ch.UnverifiedPackage as e:
    unv = str(e)
check("unsigned package is held back until the user agrees", unv is not None and "isn't signed" in unv, str(unv))
check("...and nothing was written", not os.path.exists(os.path.join(ch.BIN_DIR, "nosig")))
proj = tempfile.mkdtemp()
subprocess.run([NEXA_PACK, "keygen", os.path.join(proj, "pub")], check=True, capture_output=True)
key_id = open(os.path.join(proj, "pub.pub")).readline().split()[2]
os.makedirs(os.path.join(proj, "app", "bin")); open(os.path.join(proj, "app", "bin", "run"), "w").write("#!/bin/sh\n")
open(os.path.join(proj, "app", "nexa.toml"), "w").write('[package]\nid = "signed"\nname = "Signed"\nversion = "2.0"\n'
                                                       '[build]\nbinary = "bin/run"\n[requires]\ndepends = ["gtk+3.0"]\npermissions = ["network"]\n')
subprocess.run([NEXA_PACK, "build", os.path.join(proj, "app"), "--key", os.path.join(proj, "pub.key"),
                "-o", os.path.join(proj, "signed.nexa")], check=True, capture_output=True)
try:
    install(os.path.join(proj, "signed.nexa"), allow=False); unknown = None
except ch.UnverifiedPackage as e:
    unknown = str(e)
check("signed by a key this system doesn't trust: held back too", unknown is not None and "doesn't trust" in unknown, str(unknown))
os.rename(os.path.join(proj, "pub.pub"), os.path.join(ch.KEYS_DIR, f"{key_id}.pub"))
check("signed by a trusted publisher: installs without asking", install(os.path.join(proj, "signed.nexa"), allow=False).startswith("Installed"))
reg = json.load(open(os.path.join(ch.REGISTRY_DIR, "signed.json")))
check("...its dependencies are installed and its permissions recorded",
      installed_deps == ["gtk+3.0"] and reg["trust"] == "trusted" and reg["publisher"] == key_id and reg["permissions"] == ["network"], str(reg))
check("tampered package is refused even when the user agrees",
      "changed after" in (error_of(install, make_pkg("tamp", tamper={"bin/tamp": b"#!/bin/sh\nrm -rf ~\n"})) or ""))
check("...and nothing was written", not os.path.exists(os.path.join(ch.BIN_DIR, "tamp")))
check("file not listed in the manifest is refused", "not in the manifest" in (error_of(install, make_pkg("sneak", extra={"lib/evil.so": b"x"})) or ""))

# Overwriting system files
check("id matching a root helper is refused", error_of(install, make_pkg("nexa-setup-helper")) is not None)
check("...and the helper is untouched", open(helper).read() == "REAL HELPER")
check("id matching a system component's code dir is refused", error_of(install, make_pkg("setup")) is not None)
check("...and its code is untouched", open(os.path.join(ch.LIB_ROOT, "nexa-setup", "x.py")).read() == "REAL")
check("id shadowing a system command is refused", "system command" in (error_of(install, make_pkg("su")) or ""))
check("refused installs leave nothing behind",
      not os.path.exists(os.path.join(ch.BIN_DIR, "su")) and not os.path.exists(os.path.join(ch.REGISTRY_DIR, "su.json")))

# Hostile manifests are refused outright by the verifier
for label, kw in (("icon outside the package", {"manifest_extra": {"icon": "../../../shadow"}}),
                  ("absolute icon path", {"manifest_extra": {"icon": "/etc/shadow"}}),
                  ("newline in the name (desktop-entry injection)", {"name": "Nice\nExec=/tmp/evil"}),
                  ("newline in categories", {"manifest_extra": {"categories": "Utility;\nExec=/tmp/evil2"}}),
                  ("zip-slip path", {"extra": {"../../escape.txt": b"x"}}),
                  ("wrong architecture", {"manifest_extra": {"arch": "aarch64"}}),
                  ("unknown permission", {"manifest_extra": {"permissions": ["root"]}}),
                  ("dependency name with shell characters", {"manifest_extra": {"depends": ["gtk; reboot"]}})):
    check(f"refused: {label}", error_of(install, make_pkg("bad", **kw)) is not None)
check("...none of them left files behind",
      not [f for f in os.listdir(ch.ICON_DIR) if f.startswith("nexa-bad")] and not os.path.exists(os.path.join(ch.BIN_DIR, "bad")))

# Upgrade
install(make_pkg("hello", lib=False, icon=False))
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
    install(make_pkg("half")); raised = False
except OSError:
    raised = True
check("a failing install raises", raised)
check("a failed install leaves no files and no record",
      not os.path.exists(os.path.join(ch.BIN_DIR, "half")) and not os.path.exists(os.path.join(ch.LIB_ROOT, "nexa-half"))
      and not os.path.exists(os.path.join(ch.REGISTRY_DIR, "half.json")))
ch.ICON_DIR = icon_dir
check("...so the same package can be installed afterwards", install(make_pkg("half")).startswith("Installed"))

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
