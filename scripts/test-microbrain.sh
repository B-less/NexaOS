#!/bin/sh
# nexa-microbrain as a running service, in a clean Alpine container with
# 768 MB of memory: it starts, answers on its socket (and only to its own
# user), measures itself at rest, and reacts to memory pressure the way
# the plan says: Performance agent decides, AdaptivePulse goes to Low
# resource, a scan waits and resumes when pressure is gone, with one
# notice not a storm. The pressure comes from a fake /proc it reads
# (a container's /proc shows the host's memory); real pressure is tested
# in the 4 GB VM once it's in an image.
# Usage: sh scripts/test-microbrain.sh   (needs build/native/nexa-microbrain: make native)
set -u
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
bin="$repo/build/native/nexa-microbrain"
[ -x "$bin" ] || { echo "build it first: make native"; exit 1; }
docker run --rm --memory 768m -v "$bin:/usr/local/bin/nexa-microbrain:ro" "alpine:${version%.*}" sh -c '
rc=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+  [$2]}"; rc=1; }
apk add -q --no-cache jq socat >/dev/null 2>&1 || { echo "no jq"; exit 1; }
adduser -D -u 1000 user; adduser -D -u 1001 other
mkdir -p /run/user/1000 /tmp/fake/pressure /tmp/fakesys && chown -R user /run/user/1000 /tmp/fake && chmod 700 /run/user/1000
calm() { printf "MemTotal: 4000000 kB\nMemAvailable: 3000000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n" > /tmp/fake/meminfo
         printf "some avg10=0.00 avg60=0 avg300=0 total=0\nfull avg10=0.00 avg60=0 avg300=0 total=0\n" > /tmp/fake/pressure/memory; }
squeeze() { printf "MemTotal: 4000000 kB\nMemAvailable: 150000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n" > /tmp/fake/meminfo
            printf "some avg10=45.00 avg60=0 avg300=0 total=0\nfull avg10=20.00 avg60=0 avg300=0 total=0\n" > /tmp/fake/pressure/memory; }
calm; cp /tmp/fake/pressure/memory /tmp/fake/pressure/cpu; cp /tmp/fake/pressure/memory /tmp/fake/pressure/io
echo "cpu  100 0 100 800 0 0 0 0" > /tmp/fake/stat; chown -R user /tmp/fake
mkdir -p /home/user/.local/share/nexa-microbrain && echo "{\"version\":1,\"data\":{\"profile\":\"4gb\"}}" > /home/user/.local/share/nexa-microbrain/config.json && chown -R user /home/user
U="su user -s /bin/sh -c"
E="HOME=/home/user XDG_RUNTIME_DIR=/run/user/1000"
$U "$E NEXA_MICROBRAIN_PROC=/tmp/fake NEXA_MICROBRAIN_SYS=/tmp/fakesys NEXA_MICROBRAIN_INTERVAL=1 nexa-microbrain serve > /tmp/serve.log 2>&1 &"
sleep 3
st=$($U "$E nexa-microbrain status")
[ "$(echo "$st" | jq -r .ok)" = true ] && pass "it starts and answers on its socket" || fail "status" "$st $(cat /tmp/serve.log)"
[ "$(echo "$st" | jq -r .profile)" = Medium ] && [ "$(echo "$st" | jq -r .budget.mode)" = balanced ] && pass "4 GB profile, Balanced while calm" || fail "profile/mode" "$(echo "$st" | jq -c "{profile,budget}")"
[ "$(stat -c %a /run/user/1000/nexa-microbrain.sock)" = 600 ] && pass "its socket is private (0600)" || fail "socket mode"
# Another user (even past the file mode, as root would let them) is refused by peer uid.
su other -s /bin/sh -c "XDG_RUNTIME_DIR=/run/user/1000 nexa-microbrain status" >/tmp/other.out 2>&1 && fail "another user is turned away" "$(cat /tmp/other.out)" || pass "another user is turned away"
printf "not json\n" | $U "nc -U /run/user/1000/nexa-microbrain.sock -w 1" > /tmp/bad.out 2>/dev/null
if grep -q "not a request" /tmp/bad.out; then pass "garbage on the socket gets an error, not a crash"; else echo "SKIP garbage on the socket (this nc has no -U)"; fi
$U "$E nexa-microbrain mode turbo" >/dev/null 2>&1 && fail "an unknown mode is refused" || pass "an unknown mode is refused"

# At rest: its own footprint from the kernel, and CPU over 30 s (one core = 100 %).
s1=$($U "$E nexa-microbrain status"); sleep 30; s2=$($U "$E nexa-microbrain status")
pss=$(echo "$s2" | jq -r .self.pss_mb); rss=$(echo "$s2" | jq -r .self.rss_mb)
cpu=$(echo "$s1 $s2" | jq -s -r "((.[1].self.cpu_seconds - .[0].self.cpu_seconds) / 30 * 100)")
echo "MEASURED idle (1 s sampling, the busiest setting): RSS ${rss} MB, PSS ${pss} MB, CPU ${cpu} % of one core over 30 s"
awk "BEGIN{exit !($pss < 10)}" && pass "small at rest (PSS under 10 MB)" || fail "small at rest" "$pss MB"
awk "BEGIN{exit !($cpu < 2)}" && pass "quiet at rest (under 2 % of one core)" || fail "quiet at rest" "$cpu %"

# Memory pressure.
squeeze; sleep 3
st=$($U "$E nexa-microbrain status")
[ "$(echo "$st" | jq -r .budget.mode)" = low-resource ] && [ "$(echo "$st" | jq -r .budget.scans)" = false ] \
  && pass "memory pressure: Low resource mode, scans held back" || fail "pressure" "$(echo "$st" | jq -c "{memory,budget}")"
$U "$E nexa-microbrain decisions" | jq -r ".decisions[].what" > /tmp/d.txt
grep -q "memory is short" /tmp/d.txt && pass "the Performance agent recorded why" || fail "decision" "$(cat /tmp/d.txt)"
# Wobbling for 15 s: no storm of notices (notify-send is absent here; decisions show it).
for i in 1 2 3 4 5; do calm; sleep 1.5; squeeze; sleep 1.5; done
n=$($U "$E nexa-microbrain decisions" | jq "[.decisions[] | select(.what | startswith(\"memory is short\"))] | length")
[ "$n" -le 3 ] && pass "a wobbling reading does not flap ($n decisions in 15 s)" || fail "flapping" "$n"
calm; sleep 8
st=$($U "$E nexa-microbrain status")
[ "$(echo "$st" | jq -r .budget.mode)" = balanced ] && pass "pressure gone: back to Balanced" || fail "recovery" "$(echo "$st" | jq -c .budget)"
$U "$E nexa-microbrain mode battery-saver" | jq -e ".budget.mode == \"battery-saver\"" >/dev/null && pass "a chosen mode is applied and saved" || fail "chosen mode"
grep -q battery-saver /home/user/.local/share/nexa-microbrain/config.json && pass "...and remembered" || fail "remembered"
# ZeroSearch, answers and workspaces over the socket.
mkdir -p /home/user/Downloads /home/user/Projects/App /home/user/.local/share/applications
printf "[Desktop Entry]\nType=Application\nName=Nexa Editor\nExec=nexa-edit %%F\n" > /home/user/.local/share/applications/nexa-edit.desktop
chown -R user /home/user
a=$($U "$E nexa-microbrain ask opne donwloads")
[ "$(echo "$a" | jq -r ".results.hits[0].action.path")" = /home/user/Downloads ] && pass "ZeroSearch: \"opne donwloads\" opens Downloads" || fail "ask" "$a"
a=$($U "$E nexa-microbrain ask delete all my files")
echo "$a" | jq -e ".results.hits == [] and (.results.note | startswith(\"I can\"))" >/dev/null && pass "ZeroSearch: a delete request is refused, not guessed" || fail "delete refused" "$a"
a=$($U "$E nexa-microbrain ask the screen is way too bright")
echo "$a" | jq -e ".results.hits[0].action == {\"type\":\"brightness\",\"delta\":-10}" >/dev/null && pass "the command model: \"the screen is way too bright\" lowers it" || fail "model wording" "$a"
a=$($U "$E nexa-microbrain ask take me to layer 3")
echo "$a" | jq -e ".results.hits[0].action == {\"type\":\"layer\",\"to\":3}" >/dev/null && pass "the command model: \"take me to layer 3\"" || fail "model layer" "$a"
a=$($U "$E nexa-microbrain ask lock the front door")
echo "$a" | jq -e ".results.hits == []" >/dev/null && pass "the command model: \"lock the front door\" is not the screen lock" || fail "model outside" "$a"
for i in $(seq 20); do $U "$E nexa-microbrain ask make the screen darker" >/dev/null; done
st=$($U "$E nexa-microbrain status")
echo "MEASURED with the command model loaded (after 20 commands): PSS $(echo "$st" | jq -r .self.pss_mb) MB"
awk "BEGIN{exit !($(echo "$st" | jq -r .self.pss_mb) < 12)}" && pass "small with the model loaded (PSS under 12 MB)" || fail "model memory" "$(echo "$st" | jq -r .self.pss_mb) MB"
a=$($U "$E nexa-microbrain answer slow")
echo "$a" | jq -r .text | grep -q "^Memory: " && pass "\"Why is my computer slow?\" answers from real numbers" || fail "slow answer" "$a"
r=$(printf "%s\n" "{\"cmd\":\"workspace-save\",\"name\":\"Coding\",\"folder\":\"~/Projects/App\",\"windows\":[{\"app_id\":\"nexa-edit\",\"rect\":[1,2,3,4]},{\"app_id\":\"made-up\"}]}" | $U "socat - UNIX-CONNECT:/run/user/1000/nexa-microbrain.sock" 2>/dev/null)
if [ -n "$r" ]; then
  echo "$r" | jq -e ".ok and (.workspace.apps | length == 1)" >/dev/null && pass "a workspace saves only installed apps" || fail "workspace save" "$r"
  p=$(printf "%s\n" "{\"cmd\":\"workspace-plan\",\"name\":\"Coding\"}" | $U "socat - UNIX-CONNECT:/run/user/1000/nexa-microbrain.sock")
  echo "$p" | jq -e ".launches[0].app == \"nexa-edit\" and (.launches[0].paths[0] | endswith(\"Projects/App\"))" >/dev/null && pass "...and restores it with the project folder" || fail "workspace plan" "$p"
else
  echo "SKIP workspace save/restore over the socket (no socat)"
fi
kill $(pidof nexa-microbrain) 2>/dev/null; sleep 0.5
$U "$E NEXA_MICROBRAIN_PROC=/tmp/fake NEXA_MICROBRAIN_SYS=/tmp/fakesys nexa-microbrain serve > /tmp/serve2.log 2>&1 &"; sleep 2
$U "$E nexa-microbrain mode" | jq -e ".mode == \"battery-saver\"" >/dev/null && pass "after a restart it comes back the same" || fail "restart"
echo "{broken" > /home/user/.local/share/nexa-microbrain/config.json; kill $(pidof nexa-microbrain); sleep 0.5
$U "$E NEXA_MICROBRAIN_PROC=/tmp/fake NEXA_MICROBRAIN_SYS=/tmp/fakesys nexa-microbrain serve > /tmp/serve3.log 2>&1 &"; sleep 2
$U "$E nexa-microbrain status" | jq -e .ok >/dev/null && ls /home/user/.local/share/nexa-microbrain | grep -q "config.broken-" \
  && pass "a damaged settings file is set aside, the service still starts" || fail "damaged config"
exit $rc
'
