#!/bin/sh
# nexa-sessiond (one desktop per person, Switch person) with real accounts
# in a clean Alpine container. There are no VTs in a container, so the
# "active VT" is a number in a file (NEXA_SESSIOND_VT_FILE) and each
# "desktop" is a stand-in that runs as its person; everything else is the
# real service: who may switch, who can have a desktop, lock on leaving,
# the arrival signal, sign-out going back, the first person's restart.
# The real VT switch (seatd pausing one compositor and resuming another)
# is checked in the VM: scripts/test-live-boot.py, USERSWITCH line.
# Usage: sh scripts/test-sessions.sh   (needs build/native/nexa-sessiond: make native)
set -u
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
bin="$repo/build/native/nexa-sessiond"
[ -x "$bin" ] || { echo "build it first: make native"; exit 1; }
docker run --rm -v "$bin:/usr/local/bin/nexa-sessiond:ro" "alpine:${version%.*}" sh -c '
rc=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+  [$2]}"; rc=1; }
addgroup -g 86 seat
adduser -D -u 1000 -g "Nexa User" user; adduser -D -u 1001 -g "Ama Mensah" ama; adduser -D -u 1002 kofi
addgroup user seat; addgroup ama seat
printf "user:pw1\nama:pw2\nkofi:pw3\n" | chpasswd >/dev/null 2>&1

# The stand-in desktop: notes what it was started with, makes its Wayland
# socket name, and runs as its person until killed.
cat > /usr/local/bin/fake-session <<EOF
#!/bin/sh
echo "\$NEXA_SESSION_USER vt=\$NEXA_SESSION_VT primary=\$NEXA_SESSION_PRIMARY run=\$XDG_RUNTIME_DIR" >> /tmp/sessions.log
exec su "\$NEXA_SESSION_USER" -s /bin/sh -c "touch \$XDG_RUNTIME_DIR/wayland-0; exec sleep 100000"
EOF
cat > /usr/local/bin/fake-lock <<EOF
#!/bin/sh
echo "\$USER uid=\$(id -u) display=\$WAYLAND_DISPLAY" >> /tmp/locks.log
EOF
# Named like the lock screen, so the arrival signal finds it.
cat > /usr/local/bin/nexa-locker <<EOF
#!/bin/sh
trap "echo \$(id -un) arrived >> /tmp/arrive.log" USR2
while :; do sleep 0.1; done
EOF
chmod 755 /usr/local/bin/fake-session /usr/local/bin/fake-lock /usr/local/bin/nexa-locker
touch /tmp/locks.log /tmp/arrive.log /tmp/sessions.log; chmod 666 /tmp/locks.log /tmp/arrive.log
echo 1 > /tmp/vt
export NEXA_SESSIOND_VT_FILE=/tmp/vt NEXA_SESSIOND_SESSION=/usr/local/bin/fake-session
export NEXA_SESSIOND_LOCK=/usr/local/bin/fake-lock NEXA_SESSIOND_LOG=/tmp/sessiond.log
nexa-sessiond serve 2>/tmp/serve.err &
sleep 1
as() { who=$1; shift; su "$who" -s /bin/sh -c "NEXA_SESSION_SOCKET=/run/nexa-session.sock $*"; }
vt() { cat /tmp/vt; }
procs() { ps -o user,comm | awk -v u="$1" "\$1 == u" | wc -l; }

grep -q "^user vt=1 primary=1 run=/run/user/1000" /tmp/sessions.log && pass "the first person'"'"'s desktop starts at boot on the VT on screen" || fail "boot session" "$(cat /tmp/sessions.log /tmp/serve.err)"
[ "$(stat -c "%U %a" /run/user/1000)" = "user 700" ] && pass "their runtime folder is theirs alone (0700)" || fail "runtime dir" "$(stat -c "%U %a" /run/user/1000)"
[ "$(stat -c %a /run/nexa-session.sock)" = 666 ] && pass "anyone may ask (the kernel says who)" || fail "socket mode"

ppl=$(as kofi nexa-sessiond people)
echo "$ppl" | grep -q "^user	1000	Nexa User	1	1$" && echo "$ppl" | grep -q "^ama	1001	Ama Mensah	0	0$" \
  && ! echo "$ppl" | grep -q "^kofi" && pass "people: who can sign in, who is, who is on screen" || fail "people" "$ppl"

out=$(as ama nexa-sessiond switch ama)
echo "$out" | grep -q "only the person whose desktop is on screen" && [ "$(vt)" = 1 ] && pass "someone not on screen can'"'"'t switch" || fail "outsider switch" "$out"
out=$(as user nexa-sessiond switch kofi)
echo "$out" | grep -q "seat group" && pass "an account outside the seat group gets no desktop" || fail "kofi" "$out"
out=$(as user nexa-sessiond switch root)
echo "$out" | grep -q "system account" && pass "nor does root" || fail "root" "$out"
out=$(as user nexa-sessiond switch nobody2)
echo "$out" | grep -q "no account" && pass "nor a name that does not exist" || fail "nobody" "$out"
out=$(as user nexa-sessiond switch user)
echo "$out" | grep -q "^ok already on screen" && pass "switching to yourself changes nothing" || fail "self" "$out"

as user /usr/local/bin/nexa-locker > /dev/null 2>&1 &
sleep 5
out=$(as user nexa-sessiond switch ama)
sleep 1
[ "$out" = "ok started 7" ] && [ "$(vt)" = 7 ] && grep -q "^ama vt=7 primary=0 run=/run/user/1001" /tmp/sessions.log \
  && pass "Switch person: Ama'"'"'s desktop starts on VT 7" || fail "switch to ama" "$out vt=$(vt) $(cat /tmp/sessions.log)"
grep -q "^user uid=1000 display=wayland-0" /tmp/locks.log && pass "the desktop that left the screen was locked, as its owner" || fail "lock on leave" "$(cat /tmp/locks.log)"
as kofi nexa-sessiond people | grep -q "^ama	1001	Ama Mensah	1	1$" && pass "people shows Ama on screen" || fail "people after switch"

out=$(as user nexa-sessiond switch user)
echo "$out" | grep -q "only the person whose desktop is on screen" && pass "the paused desktop can'"'"'t pull the screen back" || fail "background switch" "$out"
as ama /usr/local/bin/nexa-locker > /dev/null 2>&1 &
sleep 5
: > /tmp/arrive.log
out=$(as ama nexa-sessiond switch user)
sleep 1
[ "$out" = "ok switched 1" ] && [ "$(vt)" = 1 ] && pass "switching back resumes the running desktop" || fail "switch back" "$out vt=$(vt)"
grep -q "^ama uid=1001 display=wayland-0" /tmp/locks.log && pass "and locks Ama'"'"'s" || fail "lock ama" "$(cat /tmp/locks.log)"
grep -q "^user arrived" /tmp/arrive.log && ! grep -q "^ama arrived" /tmp/arrive.log && pass "the arriving lock screen gets its signal (only that one)" || fail "arrival" "$(cat /tmp/arrive.log)"
[ "$(grep -c "^user vt=" /tmp/sessions.log)" = 1 ] && pass "no second copy of the first desktop" || fail "dup" "$(cat /tmp/sessions.log)"

: > /tmp/locks.log; : > /tmp/arrive.log
echo 7 > /tmp/vt; sleep 1
grep -q "^user uid=1000" /tmp/locks.log && grep -q "^ama arrived" /tmp/arrive.log && pass "Ctrl+Alt+F7 (a VT switch from outside) also locks the one leaving" || fail "outside switch" "$(cat /tmp/locks.log /tmp/arrive.log)"

# Ama signs out: her desktop ends while on screen.
pkill -u ama -x sleep; sleep 3
[ "$(vt)" = 1 ] && pass "signing out goes back to the desktop that was there before" || fail "sign out" "vt=$(vt) $(tail -3 /tmp/sessiond.log)"
[ "$(procs ama)" = 0 ] && pass "what she left running was ended" || fail "leftovers" "$(ps -o user,pid,comm | grep ama)"
[ "$(grep -c "^ama vt=" /tmp/sessions.log)" = 1 ] && pass "and her desktop is not restarted" || fail "ama restarted"

# The first person logs out: their desktop starts again (it opens locked).
pkill -u user -x sleep; sleep 3
[ "$(grep -c "^user vt=1" /tmp/sessions.log)" = 2 ] && [ "$(vt)" = 1 ] && pass "the first person'"'"'s desktop starts again after Log out" || fail "restart" "$(cat /tmp/sessions.log)"

# A desktop that ends off screen stays ended until someone switches to it.
as user nexa-sessiond switch ama > /dev/null; sleep 1
pkill -u user -x sleep; sleep 3
[ "$(grep -c "^user vt=1" /tmp/sessions.log)" = 2 ] && [ "$(vt)" = 7 ] && pass "an off-screen desktop that ends stays ended" || fail "off-screen end" "vt=$(vt) $(cat /tmp/sessions.log)"
out=$(as ama nexa-sessiond switch user); sleep 1
[ "$out" = "ok started 1" ] && [ "$(grep -c "^user vt=1" /tmp/sessions.log)" = 3 ] && pass "switching to it starts it again, on its own VT" || fail "start primary again" "$out"

apk add -q --no-cache socat >/dev/null 2>&1
printf "hello\n" | su kofi -s /bin/sh -c "socat - UNIX-CONNECT:/run/nexa-session.sock" 2>/dev/null | grep -q "^error unknown request" && pass "nonsense gets an error" || fail "nonsense"
pidof nexa-sessiond >/dev/null && pass "the service is still running" || fail "service died" "$(cat /tmp/serve.err)"
[ ! -s /tmp/serve.err ] || fail "stderr" "$(cat /tmp/serve.err)"
exit $rc
'
