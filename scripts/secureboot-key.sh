#!/bin/sh
# The NexaOS Secure Boot key: signs GRUB and the kernel on the live USB.
# Made once, kept on the build machine only, next to the release key
# (~/.config/nexaos-keys, never pushed). PCs learn the public half once,
# in shim's MokManager (docs/SECURE_BOOT.md). RSA-2048, which MokManager
# accepts everywhere.
#   sh scripts/secureboot-key.sh   (does nothing if the key exists)
set -eu
dir=${NEXA_SB_KEYS:-$HOME/.config/nexaos-keys/secureboot}
if [ -f "$dir/nexa-sb.key" ] && [ -f "$dir/nexa-sb.crt" ] && [ -f "$dir/nexa-sb.cer" ]; then
    echo "Secure Boot key: $dir (already made)"
    exit 0
fi
mkdir -p "$dir"
chmod 700 "$dir"
umask 077
openssl req -new -x509 -newkey rsa:2048 -sha256 -nodes -days 7300 \
    -subj "/CN=NexaOS Secure Boot Signing Key/O=NexaOS/" \
    -addext "extendedKeyUsage=codeSigning" \
    -keyout "$dir/nexa-sb.key" -out "$dir/nexa-sb.crt" 2>/dev/null
openssl x509 -in "$dir/nexa-sb.crt" -outform DER -out "$dir/nexa-sb.cer"
chmod 600 "$dir/nexa-sb.key"
chmod 644 "$dir/nexa-sb.crt" "$dir/nexa-sb.cer"
echo "Secure Boot key made: $dir"
openssl x509 -in "$dir/nexa-sb.crt" -noout -subject -fingerprint -sha256
