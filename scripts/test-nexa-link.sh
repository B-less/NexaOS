#!/bin/sh
# Nexa Link between three machines on one network (three containers on a
# private Docker network): a PC and a phone that pair, and a stranger.
# Checks: pairing with the code (same six digits on both), finding each
# other by their Nexa-only signals, an encrypted file sent and accepted,
# a declined one, the stranger seeing only random bytes and being refused,
# Nexa Drop letting the stranger offer a file (asked first), and Forget.
# Usage: sh scripts/test-nexa-link.sh   (needs build/native/nexa-linkd: make native)
set -u
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
bin="$repo/build/native/nexa-linkd"
[ -x "$bin" ] || { echo "build it first: make native"; exit 1; }
net=nexa-link-test
rc=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+  [$2]}"; rc=1; }
x() { name=$1; shift; docker exec -e XDG_RUNTIME_DIR=/tmp/run -e HOME=/root "nl-$name" sh -c "$*"; }

cleanup() { docker rm -f nl-pc nl-phone nl-stranger >/dev/null 2>&1; docker network rm "$net" >/dev/null 2>&1; }
cleanup
docker network create "$net" >/dev/null || exit 1
for m in pc phone stranger; do
    docker run -d --name "nl-$m" --hostname "$m" --network "$net" -v "$bin:/usr/local/bin/nexa-linkd:ro" "alpine:${version%.*}" sleep 900 >/dev/null || exit 1
    x "$m" 'mkdir -p /tmp/run && chmod 700 /tmp/run'
done
trap cleanup EXIT
x pc 'nexa-linkd name "HP EliteBook"'
x phone 'nexa-linkd name "OnePlus 6"'
PC=$(x pc 'nexa-linkd id' | sed 's/.*"id":"\([0-9a-f]*\)".*/\1/')
PHONE=$(x phone 'nexa-linkd id' | sed 's/.*"id":"\([0-9a-f]*\)".*/\1/')
PC_IP=$(docker inspect -f "{{(index .NetworkSettings.Networks \"$net\").IPAddress}}" nl-pc)

# ---- pairing: the PC shows a code, the phone "scans" it -------------------
x pc 'nexa-linkd pair-offer --yes --json > /tmp/pair.log 2>&1 &'
for i in $(seq 30); do code=$(x pc 'sed -n "s/.*\"code\":\"\([^\"]*\)\".*/\1/p" /tmp/pair.log'); [ -n "$code" ] && break; sleep 0.2; done
case "$code" in NEXA-LINK:1:"$PC_IP":*) pass "the PC shows a pairing code with its address" ;; *) fail "pairing code" "$code" ;; esac
x phone "nexa-linkd pair-accept '$code' --yes --json" > /tmp/nl-phone-pair.log 2>&1
sleep 1
sas_pc=$(x pc 'sed -n "s/.*\"sas\":\"\([0-9]*\)\".*/\1/p" /tmp/pair.log')
sas_phone=$(sed -n 's/.*"sas":"\([0-9]*\)".*/\1/p' /tmp/nl-phone-pair.log)
[ -n "$sas_pc" ] && [ "$sas_pc" = "$sas_phone" ] && pass "both screens show the same six digits ($sas_pc)" || fail "matching digits" "$sas_pc / $sas_phone"
grep -q '"event":"paired"' /tmp/nl-phone-pair.log && x pc 'grep -q "\"event\":\"paired\"" /tmp/pair.log' \
    && pass "paired on both sides" || fail "paired on both sides" "$(cat /tmp/nl-phone-pair.log)"
x stranger "nexa-linkd pair-accept '$code' --yes --json" >/dev/null 2>&1 && fail "a used pairing code is useless afterwards" || pass "a used pairing code is useless afterwards"

# ---- the services find each other -----------------------------------------
for m in pc phone stranger; do x "$m" 'nexa-linkd serve > /tmp/serve.log 2>&1 &'; done
sleep 8
x pc 'nexa-linkd devices' | grep -q "\"id\":\"$PHONE\".*\"online\":true" && pass "the PC sees the phone nearby" || fail "the PC sees the phone" "$(x pc 'nexa-linkd devices')"
x phone 'nexa-linkd devices' | grep -q '"online":true' && pass "the phone sees the PC nearby" || fail "the phone sees the PC"
x stranger 'nexa-linkd nearby' | grep -q '"nearby":\[\]' && pass "a stranger's Nexa Link recognises nothing" || fail "stranger sees nothing" "$(x stranger 'nexa-linkd nearby')"

# What goes over the air, as the stranger hears it: 32 bytes, never the same.
# (BusyBox nc keeps only the first packet it hears, so three listens.)
docker run --rm --network "$net" "alpine:${version%.*}" sh -c 'for i in 1 2 3; do timeout 5 nc -u -l -p 47420 | head -c 32 | od -An -tx1 -v | tr -d " \n"; echo; done' > /tmp/nl-air.txt 2>/dev/null
n=$(sort -u /tmp/nl-air.txt | grep -c .)
[ "$n" -ge 2 ] && pass "signals on the air are random-looking and never repeat ($n heard)" || fail "signals heard" "$(cat /tmp/nl-air.txt)"
grep -qi "$(printf 'OnePlus' | od -An -tx1 | tr -d ' \n')" /tmp/nl-air.txt && fail "a device name leaked into the signals" || pass "no device name in the signals"

# ---- a file, phone -> PC: asked on the PC, accepted -----------------------
x phone 'head -c 300000 /dev/urandom > /tmp/IMG_2041.jpg; sha256sum /tmp/IMG_2041.jpg | cut -c1-64 > /tmp/sum'
x pc 'nexa-linkd watch > /tmp/events 2>&1 &'
sleep 0.5
x phone "nexa-linkd send $PC /tmp/IMG_2041.jpg --json > /tmp/send.log 2>&1 &"
for i in $(seq 40); do offer=$(x pc 'sed -n "s/.*\"offer\":\([0-9]*\).*/\1/p" /tmp/events | tail -1'); [ -n "$offer" ] && break; sleep 0.25; done
x pc 'grep "\"event\":\"offer\"" /tmp/events' | grep -q '"from":"OnePlus 6".*"name":"IMG_2041.jpg"' \
    && pass "the PC asks: a photo from \"OnePlus 6\"" || fail "the PC asks" "$(x pc 'cat /tmp/events')"
x pc "nexa-linkd answer $offer yes" >/dev/null
for i in $(seq 40); do x phone 'grep -q "\"event\":\"sent\"" /tmp/send.log' && break; sleep 0.25; done
[ "$(x pc 'sha256sum /root/Downloads/IMG_2041.jpg | cut -c1-64')" = "$(x phone 'cat /tmp/sum')" ] \
    && pass "accepted: it arrives whole in Downloads (checksum matches)" || fail "file arrived" "$(x phone 'cat /tmp/send.log')"

# Declined: nothing is written.
x phone "nexa-linkd send $PC /tmp/IMG_2041.jpg --json > /tmp/send2.log 2>&1 &"
for i in $(seq 40); do o2=$(x pc 'sed -n "s/.*\"offer\":\([0-9]*\).*/\1/p" /tmp/events | tail -1'); [ -n "$o2" ] && [ "$o2" != "$offer" ] && break; sleep 0.25; done
x pc "nexa-linkd answer $o2 no" >/dev/null
sleep 1
x phone 'grep -q "\"event\":\"declined\"" /tmp/send2.log' && [ "$(x pc 'ls /root/Downloads | wc -l')" = 1 ] \
    && pass "declined: the phone is told, nothing is saved" || fail "declined" "$(x phone 'cat /tmp/send2.log')"

# Never overwrite; "always" for a trusted device needs no question.
x pc "nexa-linkd allow $PHONE always" >/dev/null
x phone "nexa-linkd send $PC /tmp/IMG_2041.jpg --json" > /tmp/nl-send3.log 2>&1
grep -q '"saved":"IMG_2041 (2).jpg"' /tmp/nl-send3.log && pass "allowed always: saved without asking, as 'IMG_2041 (2).jpg' (never overwrites)" || fail "always" "$(cat /tmp/nl-send3.log)"

# ---- the stranger --------------------------------------------------------
x stranger 'echo hi > /tmp/note.txt'
x stranger "nexa-linkd send $PC_IP /tmp/note.txt --json" > /tmp/nl-str.log 2>&1 \
    && fail "a stranger can't send to the PC" "$(cat /tmp/nl-str.log)" || pass "a stranger can't send to the PC (dropped at the handshake)"
x pc 'nexa-linkd drop 5' | grep -q '"drop_open":true' && pass "Nexa Drop on: receive from any Nexa device for 5 minutes"
sleep 7
x stranger 'nexa-linkd nearby' | grep -q "\"addr\":\"$PC_IP\".*\"open_drop\":true" && pass "the stranger's Nexa Link now sees a device taking drops" || fail "drop seen" "$(x stranger 'nexa-linkd nearby')"
x stranger "nexa-linkd send $PC_IP /tmp/note.txt --json > /tmp/drop.log 2>&1 &"
for i in $(seq 40); do o3=$(x pc 'grep "\"paired\":false" /tmp/events | sed -n "s/.*\"offer\":\([0-9]*\).*/\1/p" | tail -1'); [ -n "$o3" ] && break; sleep 0.25; done
[ -n "$o3" ] && pass "a drop from an unpaired device is asked first (marked unpaired)" || fail "drop asked" "$(x pc 'cat /tmp/events')"
x pc "nexa-linkd answer $o3 yes" >/dev/null
sleep 1.5
[ "$(x pc 'cat /root/Downloads/note.txt 2>/dev/null')" = hi ] && pass "...and arrives when accepted" || fail "drop arrived" "$(x stranger 'cat /tmp/drop.log')"

# ---- Forget ---------------------------------------------------------------
x pc "nexa-linkd drop 0; nexa-linkd forget $PHONE" >/dev/null
x phone "nexa-linkd ping $PC" > /tmp/nl-ping.log 2>&1 && fail "after Forget the phone is cut off" "$(cat /tmp/nl-ping.log)" || pass "after Forget the phone is cut off at once"

exit $rc
