#!/bin/sh
# The NexaOS GRUB verification key: a GPG key GRUB uses to check every file it
# loads on the live USB -- the kernel, the initramfs and grub.cfg -- so a
# tampered kernel is refused even though Alpine's GRUB has no shim_lock.
# (Secure Boot verifies shim and GRUB; this closes the GRUB->kernel link.)
#
# Made once, kept on the build machine only, next to the Secure Boot key
# (~/.config/nexaos-keys/secureboot, never pushed). The public half is baked
# into GRUB at build time; the secret half signs the files. RSA-3072.
#   sh scripts/grub-verify-key.sh   (does nothing if the key exists)
set -eu
dir=${NEXA_SB_KEYS:-$HOME/.config/nexaos-keys/secureboot}
if [ -f "$dir/grub-verify.key" ] && [ -f "$dir/grub-verify.pub" ]; then
    echo "GRUB verification key: $dir (already made)"
    exit 0
fi
mkdir -p "$dir"
chmod 700 "$dir"
umask 077
home=$(mktemp -d)
trap 'rm -rf "$home"' EXIT
chmod 700 "$home"
cat > "$home/params" <<EOF
%no-protection
Key-Type: RSA
Key-Length: 3072
Key-Usage: sign
Name-Real: NexaOS GRUB Verification Key
Name-Comment: signs the live kernel, initramfs and grub.cfg
Expire-Date: 0
%commit
EOF
GNUPGHOME="$home" gpg --batch --quiet --gen-key "$home/params"
keyid=$(GNUPGHOME="$home" gpg --list-secret-keys --with-colons | awk -F: '/^sec:/{print $5; exit}')
# Exported, binary: the public half for grub-mkimage --pubkey, the secret half
# for the build to import and sign with.
GNUPGHOME="$home" gpg --batch --export "$keyid" > "$dir/grub-verify.pub"
GNUPGHOME="$home" gpg --batch --export-secret-keys "$keyid" > "$dir/grub-verify.key"
chmod 600 "$dir/grub-verify.key"
chmod 644 "$dir/grub-verify.pub"
echo "GRUB verification key made: $dir (key id $keyid)"
echo "Keep grub-verify.key backed up offline; a new key means re-signing every ISO."
