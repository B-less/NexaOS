# Runs inside the app-test container (scripts/test-why.sh).
rc=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+  [$2]}"; rc=1; }
install -m 0755 /src/build/native/nexa-microbrain /usr/local/bin/nexa-microbrain
export HOME=/tmp/home XDG_RUNTIME_DIR=/tmp/xdg PYTHONPATH=/src/common
mkdir -p $HOME $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
nexa-microbrain serve >/tmp/serve.log 2>&1 &
for i in $(seq 30); do [ -S $XDG_RUNTIME_DIR/nexa-microbrain.sock ] && break; sleep 0.2; done
sleep 1
# why TOPIC -> the findings, one per line: "proven|text"
why() { nexa-microbrain why "$1" | python3 -c '
import json, sys
r = json.load(sys.stdin)
if not r.get("ok"): print("ERROR|" + str(r.get("error"))); sys.exit()
for f in r["findings"]: print(f["certainty"] + "|" + f["text"] + "|" + f["evidence"])'; }

if [ "${1:-}" = offline ]; then
    w=$(why network)
    echo "$w" | grep -q "^proven|This computer has no network adapter" && pass "offline: no adapter, said plainly (proven)" || fail "offline" "$w"
    exit $rc
fi

# Apps started the way the desktop starts them; one crashes, one ends with
# an error, one is ended from Pulse.
python3 - <<'PYEND'
import os, signal, subprocess, time
from gi.repository import GLib
from nexa_common import desktopapps, exits
desktopapps.launch("sh -c 'kill -SEGV $$'")
desktopapps.launch("sh -c 'exit 4'")
p = exits.watch(subprocess.Popen(["sleep", "60"]), "nexa-notes")
os.kill(p.pid, signal.SIGKILL); exits.note_ended("Notes", p.pid, signal.SIGKILL)
loop = GLib.MainLoop(); GLib.timeout_add(1500, loop.quit); loop.run()
PYEND
w=$(why crash)
echo "$w" | grep -q "^proven|sh crashed at [0-9:]*: a fault in the program.|it ended with SIGSEGV" && pass "a crash is named as a crash, with its signal (proven)" || fail "crash" "$w"
echo "$w" | grep -q "^proven|sh closed right after starting at .* (error code 4)" && pass "an error at start is told apart" || fail "error at start" "$w"
echo "$w" | grep -q "^proven|Notes was ended at .* by you, with Force quit in Pulse" && pass "Force quit in Pulse is known to be you, not a mystery" || fail "pulse kill" "$w"
echo "$w" | grep -q "^possible|nexa-notes" && fail "the Pulse kill is also shown as unexplained" || pass "...and not repeated as unexplained"

# Memory: the real numbers, the biggest program first.
python3 -c "b = bytearray(300 * 1024 * 1024); import time; time.sleep(20)" &
HOG=$!; sleep 3
w=$(why memory)
echo "$w" | head -1 | grep -q "^proven|.* of .* is in use; .* is available.|/proc/meminfo" && pass "memory: in use and available, from /proc/meminfo" || fail "memory" "$w"
echo "$w" | sed -n 2p | grep -q "^proven|python3: 3[0-9][0-9] MB" && pass "memory: the 300 MB program is named first" || fail "memory top" "$(echo "$w" | sed -n 2p)"
kill $HOG

# Stopped: a paused program.
python3 -c "import time; time.sleep(300)" & S=$!; sleep 0.3; kill -STOP $S; sleep 0.3
w=$(why stopped)
echo "$w" | grep -q "^proven|python3 is paused (stopped), not closed.|state T in /proc/$S/stat" && pass "a paused program is found (state T, proven)" || fail "stopped" "$w"
kill -CONT $S; kill $S

# Slow: a program eating a core is named, with the measurement.
yes > /dev/null & Y=$!; sleep 1
w=$(why slow)
echo "$w" | grep -qE "^proven\|yes is using [0-9]+ % of a processor core" && pass "slow: the busy program is measured and named" || fail "slow" "$w"
kill $Y

# Network (the container has a real one): an answer, from real checks.
w=$(why network)
echo "$w" | grep -qE "^(proven|possible)\|" && ! echo "$w" | grep -q "^ERROR" && pass "network: answered from link, address, route and DNS: $(echo "$w" | head -1 | cut -d"|" -f2 | cut -c1-90)" || fail "network" "$w"

# Nexus: the words lead to Why, and the text answer works.
a=$(nexa-microbrain ask why did files crash | python3 -c "import json,sys; print(json.dumps(json.load(sys.stdin)[\"results\"][\"hits\"][0][\"action\"], sort_keys=True))")
[ "$a" = "{\"topic\": \"crash\", \"type\": \"answer\"}" ] && pass "Nexus: \"why did files crash\" asks Why" || fail "nexus" "$a"
nexa-microbrain answer crash | grep -q "crashed" && pass "Nexus shows the answer as text" || fail "answer text"

# The switch in Settings > Magic Layer is real.
python3 -c "from nexa_common import magic; magic.set_feature(\"why\", enabled=False)"
nexa-microbrain why slow | grep -q "switched off in Settings" && pass "switched off: Why says so and looks at nothing" || fail "switch"
python3 -c "from nexa_common import magic; magic.set_feature(\"why\", enabled=True)"

# Its cost: the service after all of that.
st=$(nexa-microbrain status)
echo "MEASURE microbrain after the Why questions: $(echo "$st" | python3 -c "import json,sys; s=json.load(sys.stdin)[\"self\"]; print(f\"RSS {s[\"rss_mb\"]} MB, PSS {s[\"pss_mb\"]} MB, CPU {s[\"cpu_seconds\"]} s total\")")"

# Nexa Pulse, Why tab, in a headless Sway.
export WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
printf "output * resolution 1366x768 bg /src/assets/wallpaper.png fill\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
python3 -c "from nexa_common import themes; themes.apply(reload=False)" >/dev/null 2>&1
(cd /src/pulse && PYTHONPATH=/src/pulse:/src/common python3 -m nexa_pulse --why crash >/tmp/pulse.log 2>&1 &)
for i in $(seq 40); do swaymsg -t get_tree | grep -q nexa-pulse && break; sleep 0.3; done
sleep 3
grim /out/pulse-why.png && pass "Pulse opens on its Why tab (screenshot: dist/why-test/pulse-why.png)" || fail "pulse" "$(tail -5 /tmp/pulse.log)"
grep -E "Traceback" -A8 /tmp/pulse.log | head -20
grep -q Traceback /tmp/pulse.log && fail "pulse raised an error"
exit $rc
