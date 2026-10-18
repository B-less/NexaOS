"""Run with: python3 common/tests/test_kernel.py

The assistant's kernel tools (system_pressure, energy_users, kernel_events)
through nexa_common/kernel.py against the real daemon (build/native/
nexa-kerneld serve) on a fake /sys, as an ordinary user: what it can't do
without root (process events, app boost) it reports instead of failing."""
import json, os, subprocess, sys, tempfile, time
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path[:0] = [os.path.join(REPO, "common"), os.path.join(REPO, "nexa-ai")]

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

daemon = os.path.join(REPO, "build", "native", "nexa-kerneld")
if not os.access(daemon, os.X_OK):
    print("note: build/native/nexa-kerneld not built (make native); skipping")
    sys.exit(0)

root = tempfile.mkdtemp()
sys_dir, etc = os.path.join(root, "sys"), os.path.join(root, "etc")
bat = os.path.join(sys_dir, "class/power_supply/BAT0")
os.makedirs(bat); os.makedirs(etc)
for f, v in (("type", "Battery"), ("status", "Discharging"), ("capacity", "55"), ("power_now", "12000000")):
    open(os.path.join(bat, f), "w").write(v)
open(os.path.join(etc, "passwd"), "w").write("root:x:0:0::/root:/bin/sh\n")
open(os.path.join(etc, "group"), "w").write("wheel:x:10:root\n")
open(os.path.join(root, "kmsg"), "w").write("")
sock = os.path.join(root, "k.sock")
os.environ["NEXA_KERNELD_SOCKET"] = sock
proc = subprocess.Popen([daemon, "serve"], stderr=subprocess.PIPE, text=True,
                        env=dict(os.environ, NEXA_KERNELD_SYS=sys_dir, NEXA_KERNELD_ETC=etc,
                                 NEXA_KERNELD_KMSG=os.path.join(root, "kmsg")))
try:
    for _ in range(100):
        if os.path.exists(sock): break
        time.sleep(0.05)
    from nexa_common import kernel
    kernel.SOCKET = sock
    from nexa_ai import tools

    p = tools.system_pressure({})
    check("system_pressure answers in plain words", isinstance(p["summary"], str) and len(p["summary"]) > 10, str(p))
    check("...with the kernel's numbers when it has PSI",
          not os.path.exists("/proc/pressure/cpu") or "avg10" in p["pressure"]["cpu"]["some"], str(p))
    time.sleep(2.5)  # two samples, so CPU shares exist
    e = tools.energy_users({})
    check("energy_users: watts from the battery when there's no power meter", e["measured"] is True and e["battery"]["watts"] == 12.0, str(e))
    check("...and programs ranked (only this user's)", all(prog.get("name") for prog in e["programs"]), str(e))
    ev = tools.kernel_events({"limit": 5})
    check("kernel_events answers even when nothing happened", isinstance(ev["events"], list) and ev["summary"], str(ev))
    s = kernel.status()
    check("it says what it can't do without root instead of failing",
          set(s["can"]) >= {"process_events", "app_boost", "pressure_triggers", "ai_cgroup"}, str(s["can"]))
    try:
        kernel.request("focus", pid=1)
        check("someone else's program can't be boosted", os.getuid() == 0)
    except kernel.KernelError as err:
        check("someone else's program can't be boosted", "your own programs" in str(err), str(err))
    kernel.SOCKET = os.path.join(root, "missing.sock")
    try:
        tools.system_pressure({})
        check("a stopped daemon is a clear tool error", False)
    except tools.ToolError as err:
        check("a stopped daemon is a clear tool error", "isn't running" in str(err), str(err))
finally:
    proc.terminate(); proc.wait()

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
