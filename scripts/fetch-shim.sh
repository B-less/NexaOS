#!/bin/sh
# Secure Boot's first stage for the live USB: Debian's shim, signed by
# Microsoft (UEFI CA 2011 and 2023, so old and new PCs trust it), and its
# MokManager (signed by Debian, trusted by that shim). NexaOS's own GRUB
# and kernel are signed with the NexaOS key (scripts/secureboot-key.sh);
# a PC learns that key once, in MokManager (docs/SECURE_BOOT.md).
#
# Fetched with Debian's apt (which checks the archive's signatures), then
# checked against the checksums below: a different file is refused. To
# move to a newer shim, update the version and both checksums together.
#   sh scripts/fetch-shim.sh   -> build/shim/{shimx64.efi,mmx64.efi}
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
out="$repo/build/shim"
SHIM_DEB="shim-signed=1.51~1+deb13u1+16.1-2~deb13u1"
HELPERS_DEB="shim-helpers-amd64-signed=1+16.1+2~deb13u1"
SHIM_SHA=e103c5d02657879f141b086ed703584d28bb9e2721b423333be085ee1b4d397b
MM_SHA=30ea13138310f0819c1d875fcf3a6cdce8a1607cfd7578053a2df894a41630ae

ok() { [ -f "$out/$1" ] && [ "$(sha256sum "$out/$1" | cut -c1-64)" = "$2" ]; }
if ok shimx64.efi "$SHIM_SHA" && ok mmx64.efi "$MM_SHA"; then
    exit 0
fi
mkdir -p "$out"
docker run --rm -v "$out:/out" -e OWNER="$(id -u):$(id -g)" debian:trixie sh -c "
set -e
apt-get update -qq >/dev/null
cd /tmp && apt-get download -qq '$SHIM_DEB' '$HELPERS_DEB' >/dev/null
for d in *.deb; do dpkg-deb -x \"\$d\" /x; done
cp /x/usr/lib/shim/shimx64.efi.signed /out/shimx64.efi
cp /x/usr/lib/shim/mmx64.efi.signed /out/mmx64.efi
chown -R \"\$OWNER\" /out
"
if ! ok shimx64.efi "$SHIM_SHA" || ! ok mmx64.efi "$MM_SHA"; then
    rm -f "$out/shimx64.efi" "$out/mmx64.efi"
    echo "ERROR: the downloaded shim doesn't match the pinned checksums (scripts/fetch-shim.sh)" >&2
    exit 1
fi
echo "shim: $out (Debian 16.1, signed by Microsoft UEFI CA 2011 and 2023)"
