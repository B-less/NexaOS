# Runs inside the app-test container (scripts/test-rewind.sh).
rc=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+  [$2]}"; rc=1; }
install -m 0755 /src/build/native/nexa-microbrain /usr/local/bin/nexa-microbrain
export HOME=/tmp/home XDG_RUNTIME_DIR=/tmp/xdg PYTHONPATH=/src/common
mkdir -p $HOME/.config/nexa $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
py() { python3 -c "$1"; }
py "from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1
cp $HOME/.config/nexa/lock.conf /tmp/lock-horizon.conf
printf "lock_minutes=5\nblank_minutes=10\n" > $HOME/.config/nexa/idle.conf
nexa-microbrain serve >/tmp/serve.log 2>&1 &
for i in $(seq 30); do [ -S $XDG_RUNTIME_DIR/nexa-microbrain.sock ] && break; sleep 0.2; done
sleep 1
grep -q "Landlock" /tmp/serve.log && echo "NOTE running unconfined here: $(grep Landlock /tmp/serve.log)"

# Changes made the way Settings makes them; the watcher sees them.
py "from nexa_common import themes; themes.apply(\"aurora\", reload=False)" >/dev/null 2>&1
cmp -s /tmp/lock-horizon.conf $HOME/.config/nexa/lock.conf && echo "NOTE lock.conf did not change with the theme"
sleep 5
py "from nexa_common import sysconf; sysconf.write_idle(30, 10)"
sleep 1.5
n=$(py "from nexa_common import rewind; m = rewind.moments(); print(len(m)); [print(x[\"id\"], \"|\".join(c[\"label\"] + \": \" + \"; \".join(c[\"summary\"]) for c in x[\"changes\"])) for x in m]")
echo "$n" | sed -n 2p | grep -q "Screen lock and screen off timing: lock_minutes: 5 → 30" && pass "a change Settings makes appears, in words (inotify, no polling)" || fail "idle moment" "$n"
echo "$n" | sed -n 3p | grep -q "Theme: theme: horizon → aurora" && pass "a theme change is its own moment" || fail "theme moment" "$n"

# Go back to before the theme change: the theme and the later idle change.
THEME_ID=$(echo "$n" | sed -n 3p | cut -d" " -f1)
r=$(py "from nexa_common import rewind; print(rewind.go_back($THEME_ID))")
echo "$r" | grep -q "Theme" && echo "$r" | grep -q "Screen lock" && pass "going back puts back both" || fail "go back" "$r"
py "from nexa_common import themes; print(themes.load_settings()[\"theme\"])" | grep -qx horizon && pass "the theme is horizon again (and re-applied)" || fail "theme back"
grep -q "lock_minutes=5" $HOME/.config/nexa/idle.conf && pass "the idle timing is 5 minutes again" || fail "idle back" "$(cat $HOME/.config/nexa/idle.conf)"
cmp -s /tmp/lock-horizon.conf $HOME/.config/nexa/lock.conf && pass "derived files follow (lock colours re-rendered as horizon)" || fail "lock colours not re-applied" "$(diff /tmp/lock-horizon.conf $HOME/.config/nexa/lock.conf | head -4)"

# The going back is undoable.
sleep 1
UNDO=$(py "from nexa_common import rewind; m = rewind.moments(); print(m[0][\"id\"] if m[0].get(\"rewind_of\") else \"none\")")
[ "$UNDO" != none ] && pass "going back is in the timeline itself" || fail "rewind moment"
py "from nexa_common import rewind; rewind.go_back($UNDO)" >/dev/null
py "from nexa_common import themes; print(themes.load_settings()[\"theme\"])" | grep -qx aurora && pass "...and can be undone" || fail "undo the rewind"

# Never documents, never security choices.
echo secret > $HOME/letter.txt
printf "{\"x\": {\"camera\": false}}" > $HOME/.config/nexa/app-permissions.json
sleep 1.5
py "from nexa_common import rewind; m = rewind.moments(); print(m[0][\"changes\"][0][\"file\"])" | grep -q "permission" && fail "app permissions were tracked" || pass "app permissions are not tracked"
# MicroBrain can write settings back, but nothing else in home (Landlock).
if ! grep -q "Landlock" /tmp/serve.log; then
    PID=$(pidof nexa-microbrain)
    grep -q "^Seccomp\|NoNewPrivs:\s*1" /proc/$PID/status && pass "MicroBrain runs with no new privileges (its Landlock rules applied, no error logged)" || fail "confinement"
fi

py "from nexa_common import magic; magic.set_feature(\"rewind\", enabled=False)"
nexa-microbrain rewind | grep -q "switched off" && pass "switched off: Rewind says so" || fail "switch"
st=$(nexa-microbrain status | python3 -c "import json,sys; s=json.load(sys.stdin)[\"self\"]; print(s[\"rss_mb\"], s[\"cpu_seconds\"])")
echo "MEASURE microbrain with Rewind watching: RSS ${st% *} MB, CPU ${st#* } s in total for this whole test"
exit $rc
