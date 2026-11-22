#!/bin/sh
# End-to-end test of the broker's anti-rollback (downgrade) guard. Builds two
# signed packages of one app (2.0 then 1.0) with a trusted publisher key and
# drives the REAL nexa-brokerd over its socket, as the unprivileged user:
#
#   1) install 2.0                      -> installed
#   2) install 1.0 over 2.0             -> REFUSED (downgrade), no files changed
#   3) reinstall 2.0                    -> installed (same version is not a rollback)
#   4) install 1.0 with allow_unverified-> installed (explicit override)
#
# The broker's worker re-execs with a clean environment, so it installs with
# root "/". This runs in a throwaway --rm container, so the install writes to
# the container's own /usr, /etc and /var -- nothing on the host is touched.
# Usage: sh scripts/test-package-downgrade.sh
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
sh "$repo/scripts/build-native.sh" >/dev/null   # cached: only changes recompile
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
docker run --rm -v "$repo:/src:ro" "nexa-rust:${version%.*}" sh -c '
set -eu
PACK=/src/build/native/nexa-pack
BROKER=/src/build/native/nexa-brokerd
T=/tmp/dg
fail() { echo "FAIL: $*"; echo "--- broker.log ---"; tail -20 "$T/broker.log" 2>/dev/null || true; exit 1; }

mkdir -p "$T/etc" /usr/local/bin /etc/nexa/keys /var/lib/nexa-packages /var/lib/nexa-broker/work
# The worker installs with root "/": the verifier must live where it looks.
cp "$PACK" /usr/local/bin/nexa-pack

# Auth material (same shape as tools/nexa-brokerd/tests/service.rs): uid 1000
# is an administrator (wheel), password "secret".
printf "root:x:0:0::/root:/bin/sh\nuser:x:1000:1000::/home/user:/bin/sh\n" > "$T/etc/passwd"
printf "root:x:0:\nwheel:x:10:user\nuser:x:1000:\n" > "$T/etc/group"
printf "root:!::0:::::\nuser:\$6\$nexasalt\$MzZ40LeatUOJUqhCI2oVd1bgvOTb6sBY6vwLqVCZBgpJcOF1s2DBQ8Jftnj.UdYccj83QrC89CZuYVEbDt5Zg1:0:::::\n" > "$T/etc/shadow"

# A trusted publisher key, named <id>.pub so the trust store finds it.
( cd "$T" && "$PACK" keygen pub >/dev/null )
KEYID=$(head -1 "$T/pub.pub" | cut -d" " -f3)
cp "$T/pub.pub" "/etc/nexa/keys/$KEYID.pub"

build_pkg() { # version outfile
    rm -rf "$T/proj"
    "$PACK" new demo --lang c --name Demo --dir "$T/proj" >/dev/null
    sed -i "s/^version[[:space:]]*=.*/version = \"$1\"/" "$T/proj/nexa.toml"
    "$PACK" build "$T/proj" --key "$T/pub.key" -o "$2" >/dev/null
    "$PACK" verify "$2" --keys /etc/nexa/keys --json | grep -q "\"trust\":\"trusted\"" || fail "$2 not trusted"
}
build_pkg 2.0 "$T/demo-2.0.nexa"
build_pkg 1.0 "$T/demo-1.0.nexa"
chmod 644 "$T"/demo-*.nexa
echo "ok  built demo 2.0 and demo 1.0, both signed by the trusted key"

export NEXA_BROKER_SOCKET="$T/sock" NEXA_BROKER_ETC="$T/etc" NEXA_BROKER_LOG="$T/broker.log" NEXA_BROKER_TOKEN="$T/token"
"$BROKER" serve >"$T/broker.log" 2>&1 &
BPID=$!
trap "kill $BPID 2>/dev/null || true" EXIT
for _ in $(seq 1 100); do [ -S "$T/sock" ] && break; sleep 0.05; done
[ -S "$T/sock" ] || fail "broker never created its socket"
chmod 644 "$T/token" 2>/dev/null || true

ask() { # action args-json   (always as uid 1000, password "secret")
    env NEXA_BROKER_SOCKET="$T/sock" NEXA_BROKER_TOKEN="$T/token" NEXA_BROKER_PASSWORD=secret \
        python3 -c "import os,sys; os.setgid(1000); os.setuid(1000); os.execvp(sys.argv[1], sys.argv[1:])" \
        "$BROKER" call "$1" "$2" 2>/dev/null || true
}
installed_version() { sed -n "s/.*\"version\":\"\([^\"]*\)\".*/\1/p" /var/lib/nexa-packages/demo.json; }

r=$(ask install_app "{\"path\": \"$T/demo-2.0.nexa\"}")
echo "$r" | grep -q "\"ok\":true" || fail "1) installing 2.0 should succeed: $r"
[ "$(installed_version)" = 2.0 ] || fail "1) registry should record 2.0, got $(installed_version)"
echo "ok  1) demo 2.0 installed"

r=$(ask install_app "{\"path\": \"$T/demo-1.0.nexa\"}")
echo "$r" | grep -q "\"downgrade\":true" || fail "2) installing 1.0 over 2.0 should be refused as a downgrade: $r"
echo "$r" | grep -q "\"ok\":false" || fail "2) a refused downgrade must not report ok: $r"
[ "$(installed_version)" = 2.0 ] || fail "2) the refused downgrade must leave 2.0 in place, got $(installed_version)"
echo "ok  2) demo 1.0 refused as a downgrade; 2.0 still installed, untouched"

r=$(ask install_app "{\"path\": \"$T/demo-2.0.nexa\"}")
echo "$r" | grep -q "\"ok\":true" || fail "3) reinstalling the same 2.0 should succeed: $r"
echo "ok  3) same-version reinstall (2.0 over 2.0) allowed"

r=$(ask install_app "{\"path\": \"$T/demo-1.0.nexa\", \"allow_unverified\": true}")
echo "$r" | grep -q "\"ok\":true" || fail "4) an explicit override should install 1.0: $r"
[ "$(installed_version)" = 1.0 ] || fail "4) override should install 1.0, got $(installed_version)"
echo "ok  4) deliberate rollback to 1.0 allowed with allow_unverified"

echo PASS
'
