#!/bin/sh
# nexa-later (tools-c/nexa-later): builds it in Alpine and checks that it
# starts the commands once the system is quiet, starts them anyway at --max
# while the CPU is saturated, and refuses bad arguments.
#   sh scripts/test-nexa-later.sh
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
docker run --rm -v "$repo/tools-c/nexa-later:/src:ro" alpine:3.24 sh -c '
set -eu
apk add -q --no-cache build-base >/dev/null
cp -r /src /b && make -s -C /b nexa-later
L=/b/nexa-later
pass=0 fail=0
ok() { echo "PASS $1"; pass=$((pass + 1)); }
no() { echo "FAIL $1"; fail=$((fail + 1)); }
[ -r /proc/pressure/cpu ] && echo "(PSI available)" || echo "(no PSI: /proc/stat fallback)"

# 1. Quiet: starts both commands well before --max, and says it settled.
# The pressure counters are the host'"'"'s (shared with every container), so
# a host still busy with something else gets three tries.
for try in 1 2 3; do
    rm -f /tmp/a /tmp/b
    t0=$(date +%s)
    $L --min 0 --max 15 "touch /tmp/a" "touch /tmp/b" 2>/tmp/log1
    sleep 0.5
    t=$(( $(date +%s) - t0 ))
    cat /tmp/log1
    grep -q "settled after" /tmp/log1 && break
    echo "(host busy; trying again)"
done
[ -f /tmp/a ] && [ -f /tmp/b ] && ok "quiet: both commands ran" || no "quiet: commands missing"
grep -q "settled after" /tmp/log1 && [ $t -lt 10 ] && ok "quiet: settled in ${t}s" || no "quiet: took ${t}s / not settled"

# 2. Busy: twice as many CPU hogs as CPUs; must wait until --max, then start.
n=$(( $(nproc) * 2 ))
i=0; while [ $i -lt $n ]; do (while :; do :; done) & i=$((i + 1)); done
sleep 1
t0=$(date +%s)
$L --min 0 --max 4 "touch /tmp/c" 2>/tmp/log2
sleep 0.5
t=$(( $(date +%s) - t0 ))
kill $(jobs -p) 2>/dev/null || true
cat /tmp/log2
[ -f /tmp/c ] && ok "busy: command ran at --max" || no "busy: command missing"
grep -q "still busy" /tmp/log2 && [ $t -ge 3 ] && ok "busy: waited ${t}s for --max" || no "busy: did not wait (${t}s)"

# 3. Order and stagger: commands start in the order given.
$L --min 0 --max 1 "echo one >> /tmp/order" "echo two >> /tmp/order" 2>/dev/null
sleep 0.5
[ "$(tr "\n" " " < /tmp/order)" = "one two " ] && ok "commands start in order" || no "order: $(cat /tmp/order)"

# 4. Bad arguments.
rc=0; $L 2>/dev/null || rc=$?
[ $rc = 2 ] && ok "no command: usage, exit 2" || no "no command accepted (exit $rc)"
rc=0; $L --bogus 1 x 2>/dev/null || rc=$?
[ $rc = 2 ] && ok "unknown option: exit 2" || no "unknown option accepted (exit $rc)"

echo "nexa-later: $pass passed, $fail failed"
[ $fail = 0 ]
'
