"""Run with: python3 common/tests/test_patterns.py

Nexa Patterns from the desktop's side (nexa_common/patterns.py) against the
real daemon (build/native/nexa-patternd serve) holding a synthetic month of
use: off until switched on, suggestions resolved to installed apps with
reasons in words, Don't suggest, Hide for today, forgetting."""
import json, os, subprocess, sys, tempfile, time
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO, "common"))

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

daemon = os.path.join(REPO, "build", "native", "nexa-patternd")
if not os.access(daemon, os.X_OK):
    print("note: build/native/nexa-patternd not built (make native); skipping")
    sys.exit(0)

root = tempfile.mkdtemp()
home, run, apps_dir = (os.path.join(root, d) for d in ("home", "run", "apps"))
for d in (home, run, apps_dir, os.path.join(home, ".local/share/nexa-patterns")):
    os.makedirs(d, exist_ok=True)
os.environ.update(HOME=home, XDG_RUNTIME_DIR=run, XDG_CONFIG_HOME=os.path.join(home, ".config"))
os.environ.pop("NEXA_PATTERNS_SOCKET", None)
names = {"nexa-mail": "Mail", "nexa-browser": "Nexa Browser", "foot": "Terminal", "nexa-files": "Files",
         "nexa-edit": "Editor", "nexa-music": "Music", "video-player": "Video Player", "nexa-calc": "Calculator",
         "nexa-notes": "Notes", "nexa-pulse": "Nexa Pulse", "nexa-settings": "Settings", "nexa-viewer": "Viewer"}
marker = os.path.join(root, "launched")
for app_id, name in names.items():
    open(os.path.join(apps_dir, app_id + ".desktop"), "w").write(
        f"[Desktop Entry]\nType=Application\nName={name}\nExec=sh -c 'echo {app_id} >> {marker}'\nIcon={app_id}\n")
subprocess.run([daemon, "synthetic", "30", os.path.join(home, ".local/share/nexa-patterns/journal.bin")], check=True)
proc = subprocess.Popen([daemon, "serve"], stderr=subprocess.DEVNULL,
                        env=dict(os.environ, SWAYSOCK=os.path.join(run, "no-sway.sock")))
try:
    for _ in range(100):
        if os.path.exists(os.path.join(run, "nexa-patterns.sock")): break
        time.sleep(0.05)
    from nexa_common import desktopapps, patterns
    apps = desktopapps.load_apps(dirs=[apps_dir])

    card = patterns.suggestions("card", apps=apps)
    check("off until switched on: no card", card["show"] is False and card["apps"] == [], str(card))
    check("the settings file says off by default", patterns.load_config()["enabled"] is False)

    patterns.update_config(enabled=True)
    card = patterns.suggestions("card", apps=apps)
    check("switched on: the card shows up to 3 apps", card["show"] and 1 <= len(card["apps"]) <= 3, str(card))
    first = card["apps"][0]
    check("each resolved to the installed app", first["entry"]["name"] == names[first["app"]], str(first))
    check("with a reason in words, using display names",
          first["text"].startswith(("You opened", "You often open", "You've used")) and first["entry"]["name"] in first["text"]
          and "nexa-" not in first["text"], first["text"])
    check("why_text: the 'after' reason names the earlier app too",
          patterns.why_text({"why_parts": {"kind": "after", "after": "foot", "times": 6}}, "Files", apps)
          == "You often open Files after Terminal (6 times in the last two weeks).")

    patterns.launch(first)
    time.sleep(0.5)
    check("opening a suggestion starts the app", open(marker).read().strip() == first["app"])
    check("...and tells the model it was used", any(r["kind"] == "used" for r in patterns.request("export")["records"]))

    patterns.never_suggest(first["app"])
    later = patterns.suggestions("card", apps=apps)
    check("Don't suggest: gone from the card", all(s["app"] != first["app"] for s in later["apps"]), str(later["apps"]))
    check("...and saved in the settings", first["app"] in patterns.load_config()["never_suggest"])

    patterns.hide_for_today()
    hidden = patterns.suggestions("card", apps=apps)
    check("Hide for today: nothing until midnight", hidden["hidden"] and hidden["apps"] == [], str(hidden))
    check("...which is the next local midnight", time.localtime(patterns.load_config()["hidden_until"]).tm_hour == 0)
    patterns.update_config(hidden_until=0, card_position="bottom-right")
    check("the card's position comes from the settings", patterns.request("card")["position"] == "bottom-right")

    patterns.request("forget", all=True)
    check("forget everything", patterns.request("status")["records"] == 0
          and patterns.suggestions("card", apps=apps)["apps"] == [])
    os.environ["NEXA_PATTERNS_SOCKET"] = os.path.join(run, "missing.sock")
    try:
        patterns.request("card")
        check("a stopped daemon is a clear error", False)
    except patterns.PatternsError as e:
        check("a stopped daemon is a clear error", "isn't running" in str(e), str(e))
finally:
    proc.terminate(); proc.wait()

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
