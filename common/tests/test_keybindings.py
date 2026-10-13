"""Run with: python3 common/tests/test_keybindings.py

No key is bound twice in the generated Sway and Hyprland configs. Sway
shows a red "errors in your config file" bar for a duplicate binding."""
import os, re, sys
HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(os.path.dirname(os.path.dirname(HERE)), "scripts", "generate-configs.sh")

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

text = open(SRC).read()

# Sway: "bindsym [--flags] <combo> <command>"; modifiers in any order/case.
sway = {}
for n, line in enumerate(text.splitlines(), 1):
    m = re.match(r"\s*bindsym\s+((?:--\S+\s+)*)(\S+)\s+(.*)", line)
    if not m:
        continue
    flags = " ".join(sorted(m.group(1).split()))
    combo = "+".join(sorted(p.lower() for p in m.group(2).split("+")))
    sway.setdefault((flags, combo), []).append(n)
dups = {k: v for k, v in sway.items() if len(v) > 1}
check(f"sway: {len(sway)} bindings, none bound twice", sway and not dups, str(dups))
check("sway: Nexus on Super+Space", ("", "$mod+space") in sway)
check("sway: Nexa Pulse on Ctrl+Shift+Esc", ("", "ctrl+escape+shift") in sway)

# Hyprland: "bind = MODS, KEY, dispatcher, args". Super+Tab is bound twice
# on purpose (cycle focus, then raise it), so pairs listed here are allowed.
ALLOWED = {("super", "tab")}
hypr = {}
for n, line in enumerate(text.splitlines(), 1):
    m = re.match(r"\s*bind[a-z]*\s*=\s*([^,]*),\s*([^,]+),", line)
    if not m:
        continue
    key = (" ".join(sorted(m.group(1).lower().split())), m.group(2).strip().lower())
    hypr.setdefault(key, []).append(n)
dups = {k: v for k, v in hypr.items() if len(v) > 1 and k not in ALLOWED}
check(f"hyprland: {len(hypr)} bindings, none bound twice by accident", hypr and not dups, str(dups))
check("hyprland: Nexus and Nexa Pulse bound", ("super", "space") in hypr and ("ctrl shift", "escape") in hypr)

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
