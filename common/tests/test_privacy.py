"""Run with: python3 common/tests/test_privacy.py

The Privacy Center's data (nexa_common/privacy.py) against a fake install
registry, and that a switch flipped here is what the real launcher
(build/native/nexa-run) then enforces."""
import json, os, subprocess, sys, tempfile
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO, "common"))
from nexa_common import privacy

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

root = tempfile.mkdtemp()
reg, apps, icons, home = (os.path.join(root, d) for d in ("reg", "apps", "icons", "home"))
for d in (reg, apps, icons, home): os.makedirs(d)
json.dump({"id": "video-player", "version": "1.0.2", "trust": "trusted", "publisher": privacy.NEXAOS_KEY,
           "permissions": ["files.videos", "network", "microphone"]}, open(os.path.join(reg, "video-player.json"), "w"))
json.dump({"id": "calc", "version": "2.0", "trust": "unsigned", "permissions": []}, open(os.path.join(reg, "calc.json"), "w"))
open(os.path.join(apps, "nexa-video-player.desktop"), "w").write("[Desktop Entry]\nName=Video Player\n")
open(os.path.join(icons, "nexa-video-player.svg"), "w").write("<svg/>")
where = dict(registry=reg, apps_dir=apps, icon_dir=icons, home=home)

a = privacy.list_apps(**where)
check("installed apps listed by name", [x["name"] for x in a] == ["calc", "Video Player"], str(a))
vp = a[1]
check("plain words, who signed it, its icon", vp["permissions"][0]["label"] == "Your Videos folder"
      and vp["signed"] == "signed by NexaOS" and vp["icon"].endswith("nexa-video-player.svg"))
check("an unverified app says so", "not verified" in a[0]["signed"])
check("what can't be enforced yet is marked", [p["enforced"] for p in vp["permissions"]] == [True, True, False])
check("summaries", privacy.summary(vp) == "Can use: your Videos folder, the internet, the microphone"
      and privacy.summary(a[0]) == "Uses only its own files")

privacy.set_allowed("video-player", "network", False, home)
check("switching off is saved for nexa-run", json.load(open(privacy.prefs_path(home))) == {"video-player": {"denied": ["network"]}})
vp = privacy.list_apps(**where)[1]
check("...and shown as off", [p["allowed"] for p in vp["permissions"]] == [True, False, True])
privacy.set_allowed("video-player", "network", True, home)
check("switching back on removes the entry entirely", json.load(open(privacy.prefs_path(home))) == {})
open(privacy.prefs_path(home), "w").write("not json")
check("a damaged settings file isn't fatal", privacy.list_apps(**where)[1]["permissions"][1]["allowed"])

runner = os.path.join(REPO, "build", "native", "nexa-run")
if os.access(runner, os.X_OK):
    privacy.set_allowed("video-player", "network", False, home)
    out = subprocess.run([runner, "--explain", "video-player"], capture_output=True, text=True,
                         env={"HOME": home, "NEXA_RUN_REGISTRY": reg, "PATH": os.environ["PATH"]}).stdout
    check("the real launcher enforces what the Privacy Center saved",
          "network: switched off by you" in out and "no internet" in out and "files.videos" in out, out)
else:
    print("note: build/native/nexa-run not built; skipping the launcher check")

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
