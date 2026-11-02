# Inside the system image (scripts/test-browser.sh).
export HOME=/tmp/home XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $HOME $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
# Names the test needs, all on this machine: an ad server from the built-in
# list, and a site with no https.
printf "127.0.0.1 doubleclick.net plain.test\n" >> /etc/hosts
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
printf "output * resolution 1280x800\ndefault_border none\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
mkdir -p /tmp/www/media && cp /src/common/tests/browser_pages/* /tmp/www/ && cp /media/* /tmp/www/media/ 2>/dev/null
cp /src/common/tests/browser_media.html /tmp/www/media/index.html
(cd /tmp/www && python3 -m http.server 8765 >/tmp/http.log 2>&1 &)
sleep 1
export PYTHONPATH=/src/browser:/src/common NEXA_WEBFILTER=/src/build/native/nexa-webfilter NEXA_WEBDATA=/src/build/native/nexa-webdata OUT=/out
python3 /src/common/tests/browser_features.py 2>/tmp/browser.err; rc=$?
grep -E "Traceback|Error" -A6 /tmp/browser.err | grep -v "libEGL\|ZINK\|MESA\|Glycin" | head -30
exit $rc
