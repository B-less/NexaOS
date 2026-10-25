"""Run with: python3 common/tests/test_compositor.py

The window-manager layer (nexa_common/compositor.py) against fake hyprctl
and swaymsg, for both sessions."""
import json, os, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from nexa_common import compositor as c

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

calls, replies = [], {}
def fake_run(argv, **kw):
    calls.append(argv)
    key = " ".join(argv[:3])
    out = replies.get(key, "ok" if argv[0] == "hyprctl" else "[]")
    return subprocess.CompletedProcess(argv, 0, stdout=out, stderr="")
c._run = fake_run

def session(name):
    os.environ.pop("HYPRLAND_INSTANCE_SIGNATURE", None); os.environ.pop("SWAYSOCK", None)
    if name == "hyprland": os.environ["HYPRLAND_INSTANCE_SIGNATURE"] = "abc"
    if name == "sway": os.environ["SWAYSOCK"] = "/run/sway.sock"
    calls.clear()

session(None)
check("outside a session nothing is called", c.minimize(42) is False and calls == [])

# ---- Hyprland
session("hyprland")
check("detects Hyprland", c.which() == "hyprland")
c.minimize(42)
check("minimize -> special workspace, silently, by pid",
      calls[-1] == ["hyprctl", "dispatch", "movetoworkspacesilent", "special:minimized,pid:42"], str(calls))
calls.clear(); c.toggle_maximize(42)
check("maximize -> focus by pid, then maximize mode (fullscreen 1)",
      calls == [["hyprctl", "dispatch", "focuswindow", "pid:42"], ["hyprctl", "dispatch", "fullscreen", "1"]], str(calls))
replies["hyprctl clients -j"] = json.dumps([
    {"class": "nexa-files", "pid": 10, "workspace": {"id": 1, "name": "1"}},
    {"class": "nexa-calc", "pid": 11, "workspace": {"id": -99, "name": "special:minimized"}}])
wins = c.windows()
check("lists windows with minimised state",
      wins == [{"app_id": "nexa-files", "pid": 10, "minimized": False},
               {"app_id": "nexa-calc", "pid": 11, "minimized": True}], str(wins))
replies["hyprctl activeworkspace -j"] = json.dumps({"id": 3})
calls.clear(); c.focus_app("nexa-calc")
check("restoring a minimised app: back to this workspace silently, focused, then raised on top",
      calls[-3:] == [["hyprctl", "dispatch", "movetoworkspacesilent", "3,pid:11"], ["hyprctl", "dispatch", "focuswindow", "pid:11"],
                     ["hyprctl", "dispatch", "alterzorder", "top,pid:11"]], str(calls))
calls.clear(); c.focus_app("nexa-files")
check("an app already on screen is focused and raised, not moved",
      not any("movetoworkspacesilent" in x for x in calls) and calls[-1] == ["hyprctl", "dispatch", "alterzorder", "top,pid:10"], str(calls))
calls.clear(); c.screen_power(False)
check("screen off -> dpms off", calls[-1] == ["hyprctl", "dispatch", "dpms", "off"])
calls.clear(); c.set_keyboard_layout("fr")
check("keyboard layout via hyprctl keyword", calls[-1] == ["hyprctl", "keyword", "input:kb_layout", "fr"])
calls.clear(); c.reload()
check("reload", calls[-1] == ["hyprctl", "reload"])

# ---- Sway
session("sway")
check("detects sway", c.which() == "sway")
c.minimize(42)
check("minimize -> scratchpad by pid", calls[-1] == ["swaymsg", "[pid=42] move scratchpad"])
calls.clear(); c.toggle_maximize(42)
check("maximize fills the workspace", calls[-1] == ["swaymsg", "[pid=42] resize set width 100 ppt height 100 ppt, move position 0 0"])
calls.clear(); c.toggle_maximize(42, restore_size=(800, 600))
check("second click restores the old size", calls[-1] == ["swaymsg", "[pid=42] resize set width 800 px height 600 px, move position center"])
replies["swaymsg -t get_tree"] = json.dumps({"nodes": [
    {"name": "__i3_scratch", "nodes": [], "floating_nodes": [{"app_id": "nexa-calc", "pid": 11}]},
    {"name": "1", "nodes": [], "floating_nodes": [{"app_id": "nexa-files", "pid": 10}]}]})
wins = sorted(c.windows(), key=lambda w: w["pid"])
check("scratchpad windows count as minimised",
      wins == [{"app_id": "nexa-files", "pid": 10, "minimized": False},
               {"app_id": "nexa-calc", "pid": 11, "minimized": True}], str(wins))
calls.clear(); c.focus_app("nexa-calc")
check("restoring a minimised app comes out of the scratchpad (shown, focused, on top)",
      calls[-1] == ["swaymsg", '[app_id="nexa-calc"] scratchpad show'], str(calls))
calls.clear(); c.focus_app("nexa-files")
check("an app on screen is just focused", calls[-1] == ["swaymsg", '[app_id="nexa-files"] focus'], str(calls))
calls.clear(); c.screen_power(True)
check("screen on", calls[-1] == ["swaymsg", "output * power on"])

# Log out: a marker for the next session, then the compositor exits; the
# real nexa-login-lock then locks (once) even on the live USB.
import tempfile  # noqa: E402
run = tempfile.mkdtemp()
os.environ["XDG_RUNTIME_DIR"] = run
calls.clear(); c.logout()
check("log out leaves the marker, then exits", os.path.exists(os.path.join(run, "nexa-logged-out")) and calls[-1] == ["swaymsg", "exit"], str(calls))
bindir = tempfile.mkdtemp()
with open(os.path.join(bindir, "nexa-lock"), "w") as f:
    f.write(f"#!/bin/sh\necho locked >> {run}/lock.log\n")
os.chmod(os.path.join(bindir, "nexa-lock"), 0o755)
login_lock = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "power", "nexa-login-lock")
env = dict(os.environ, PATH=bindir + ":" + os.environ["PATH"], HOME=run)
subprocess.run(["sh", login_lock], env=env, check=False)
subprocess.run(["sh", login_lock], env=env, check=False)
log = open(os.path.join(run, "lock.log")).read() if os.path.exists(os.path.join(run, "lock.log")) else ""
check("the next session opens locked, once (the marker is used up)",
      log == "locked\n" and not os.path.exists(os.path.join(run, "nexa-logged-out")), repr(log))

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
