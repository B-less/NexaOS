#!/bin/sh
# Mail (apps/mail, C): unit tests, then the window in a headless Sway,
# started through nexa-run (confined to network, files.documents, files.pictures and notifications, as when installed),
# with the real notification service (nexa-signald) for new mail,
# against a real IMAP server (Dovecot, TLS) and a test SMTP server (STARTTLS),
# then the package: built twice (byte-identical), verified, extracted and
# run from there. The Store's screenshots (apps/mail/shots) come from the
# window test. Screenshots: OUT_DIR.
# Usage: sh scripts/test-mail-app.sh [OUT_DIR]   (default dist/mail-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
out=${1:-$repo/dist/mail-test}
mkdir -p "$out" "$repo/build/test"
out=$(cd "$out" && pwd)
[ -x "$repo/build/native/nexa-pack" ] && [ -x "$repo/build/native/nexa-run" ] || sh "$repo/scripts/build-native.sh" >/dev/null
# New mail becomes a Signal: the real notification service, in the test too.
mkdir -p /var/tmp/nexa-cargo
docker run --rm -v "$repo:/src" -v /var/tmp/nexa-cargo:/root/.cargo/registry -e OWNER="$(id -u):$(id -g)" -w /src/tools "nexa-rust:${version%.*}" sh -c '
set -e
target=$(rustc -vV | sed -n "s/^host: //p")
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release -q -p nexa-signald --target $target
install -m 0755 target/$target/release/nexa-signald target/$target/release/nexa-signalctl /src/build/test/
chown -R "$OWNER" /src/tools/target /src/tools/Cargo.lock /src/build/test
'

if ! docker run --rm "nexa-mailtest:${version%.*}" sh -c 'command -v dovecot' >/dev/null 2>&1; then
    printf 'FROM nexa-apptest:%s\nRUN apk add -q dovecot openssl curl\n' "${version%.*}" | docker build -q -t "nexa-mailtest:${version%.*}" - >/dev/null
fi

echo "== build and unit tests"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-rust-apps:${version%.*}" sh -c '
set -e
cd /src/apps/mail
adduser -D -u 1000 u >/dev/null 2>&1; chown -R u .; su u -c "make -s test" | grep -v "^PASS"; chown -R "$OWNER" .
make -s build/mail
install -m 0755 build/mail /src/build/test/mail
rm -rf build
chown "$OWNER" /src/build/test/mail
'

# The window, installed and confined. $1: the program to run.
run_window() {
    docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$out:/out" -e OUT=/out -e PROGRAM="$1" -e SMOKE="${2:-}" \
        "nexa-mailtest:${version%.*}" sh -c '
set -e
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR /tmp/bin /tmp/reg /tmp/libexec /tmp/lib && chmod 700 $XDG_RUNTIME_DIR
cp /src/build/native/nexa-run /src/build/test/nexa-signald /src/build/test/nexa-signalctl /tmp/bin/
cp "$PROGRAM" /tmp/libexec/mail
echo "{\"id\": \"mail\", \"permissions\": [\"network\", \"files.documents\", \"files.pictures\", \"notifications\"]}" > /tmp/reg/mail.json
# The mail servers: Dovecot (IMAP over TLS) and the test SMTP server (STARTTLS),
# with their own certificate authority (MAIL_CA_FILE tells Mail about it).
adduser -D -H vmail 2>/dev/null; mkdir -p /tmp/mailhome /tmp/tls /tmp/outbox && chown vmail:vmail /tmp/mailhome
( cd /tmp/tls
  openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem -days 2 -subj "/CN=Test CA" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -keyout server.key -out server.csr -subj "/CN=localhost" 2>/dev/null
  printf "subjectAltName=DNS:localhost,IP:127.0.0.1\n" > ext.cnf
  openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out server.pem -days 2 -extfile ext.cnf 2>/dev/null
  chmod 644 ca.pem server.pem )
mkdir -p /root/Documents /root/Pictures
echo "ama@test.local:{PLAIN}hunter2" > /tmp/dovecot.users
dovecot -c /src/apps/mail/tests/dovecot.conf
python3 /src/apps/mail/tests/smtpd.py 587 /tmp/tls/server.pem /tmp/tls/server.key /tmp/dovecot.users /tmp/mailhome /tmp/outbox > /tmp/smtpd.log 2>&1 &
export PATH=/tmp/bin:$PATH NEXA_RUN_REGISTRY=/tmp/reg NEXA_RUN_LIBEXEC=/tmp/libexec NEXA_RUN_LIB=/tmp/lib
DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address | head -1); export DBUS_SESSION_BUS_ADDRESS
python3 -c "import sys; sys.path.insert(0, \"/src/common\"); from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1 || true
printf "output * resolution 1280x800\ndefault_border none\nseat * xcursor_theme Adwaita 24\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
# Signals after Sway: a click on a Signal asks Sway to bring its app forward (as in a session)
nexa-signald > /tmp/signald.log 2>&1 &
if [ -n "$SMOKE" ]; then
    SMOKE=1 python3 /src/apps/mail/tests/headless.py
else
    python3 /src/apps/mail/tests/headless.py
fi
'
}

echo "== the window (through nexa-run)"
run_window /src/build/test/mail

echo "== the Store's screenshots"
mkdir -p "$repo/apps/mail/shots"
cp "$out/mail-reading.png" "$repo/apps/mail/shots/reading.png"
cp "$out/mail-reply.png" "$repo/apps/mail/shots/reply.png"
cp "$out/mail-setup.png" "$repo/apps/mail/shots/setup.png"
ls -l "$repo/apps/mail/shots"

echo "== the package"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-rust-apps:${version%.*}" sh -c '
set -e
cd /src/apps/mail
rm -rf build dist
/src/build/native/nexa-pack build | tail -1
a=$(sha256sum dist/mail-*.nexa | cut -d" " -f1)
mv dist/mail-1.0.0.nexa /tmp/first.nexa
rm -rf build
/src/build/native/nexa-pack build | tail -1
b=$(sha256sum dist/mail-*.nexa | cut -d" " -f1)
[ "$a" = "$b" ] && echo "PASS built twice: byte-identical ($a)" || { echo "FAIL two builds differ"; exit 1; }
/src/build/native/nexa-pack verify dist/mail-1.0.0.nexa --keys /src/assets/keys --json
rm -rf /tmp/x
/src/build/native/nexa-pack extract dist/mail-1.0.0.nexa /tmp/x --keys /src/assets/keys >/dev/null
ls /tmp/x /tmp/x/bin
install -m 0755 /tmp/x/bin/mail /src/build/test/mail-from-package
echo "package size: $(stat -c %s dist/mail-1.0.0.nexa) bytes"
chown -R "$OWNER" build dist /src/build/test/mail-from-package
'

echo "== run from the package"
run_window /src/build/test/mail-from-package smoke
