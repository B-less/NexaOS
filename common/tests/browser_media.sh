# Inside the system image (scripts/test-browser-media.sh).
export HOME=/tmp/home XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $HOME $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
printf "output * resolution 1280x800\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
(cd /media && python3 -m http.server 8765 --bind 127.0.0.1 >/tmp/http.log 2>&1 &)
sleep 1
export PYTHONPATH=/src/browser:/src/common SHOT=/media/browser-media.png ${EXTRA_ENV:-}
python3 /src/common/tests/browser_media.py http://127.0.0.1:8765/index.html 2>/tmp/browser.err
grep -iE "gst|error|warn" /tmp/browser.err | grep -v "xdg-toplevel-icon" | sort | uniq -c | sort -rn | head -15
