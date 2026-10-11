#!/bin/sh
# Stage the nexa-ai system assistant (CLI + GTK GUI) into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-ai" "$rootfs/usr/local/bin" \
    "$rootfs/usr/share/applications" "$rootfs/etc/nexa-ai"

rm -rf "$rootfs/usr/local/lib/nexa-ai/nexa_ai"
cp -a "$base/nexa-ai/nexa_ai" "$rootfs/usr/local/lib/nexa-ai/nexa_ai"
find "$rootfs/usr/local/lib/nexa-ai/nexa_ai" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-ai:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_ai "$@"' \
    > "$rootfs/usr/local/bin/nexa-ai"
chmod +x "$rootfs/usr/local/bin/nexa-ai"

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-ai:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_ai.gui "$@"' \
    > "$rootfs/usr/local/bin/nexa-ai-gui"
chmod +x "$rootfs/usr/local/bin/nexa-ai-gui"

# Privileged control-action helper. nexa-ai-gui/-cli run unprivileged as
# "user" (the normal Sway session account); control tools that need root
# (cpufreq governor, rc-service, modprobe, ...) reach it via
# `doas nexa-ai-control-helper`, which /etc/doas.conf permits "user" to run,
# and nothing else, without a password. See control_helper.py: it reads one
# {"tool", "args"} request from stdin and refuses anything not already an
# allowlisted control tool, so the doas grant can't be leveraged into a
# generic root shell even by something that reuses it.
printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-ai:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_ai.control_helper' \
    > "$rootfs/usr/local/bin/nexa-ai-control-helper"
chmod +x "$rootfs/usr/local/bin/nexa-ai-control-helper"

printf '%s\n' \
    'permit nopass user cmd /usr/local/bin/nexa-ai-control-helper' \
    > "$rootfs/etc/doas.conf"
# doas refuses to run at all unless this file is owned by root and 0400.
chown 0:0 "$rootfs/etc/doas.conf"
chmod 400 "$rootfs/etc/doas.conf"

if [ ! -f "$rootfs/etc/nexa-ai/config.json" ]; then
    cat > "$rootfs/etc/nexa-ai/config.json" << 'CONFIG_EOF'
{
  "api_key": "",
  "model": "claude-sonnet-5",
  "allow_control": false,
  "require_confirmation": true
}
CONFIG_EOF
fi
# root can edit it, group "user" (gid 1000, the desktop account nexa-ai-gui
# runs as) can read it (needs the api_key), nobody else can read or write it.
# Numeric ids: this runs in a build container whose own /etc/group has no
# "user" entry, so a name-based `chown root:user` fails to resolve.
chown 0:1000 "$rootfs/etc/nexa-ai/config.json"
chmod 640 "$rootfs/etc/nexa-ai/config.json"

cat > "$rootfs/usr/share/applications/nexa-ai.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=NexaOS Assistant
Comment=System AI assistant with hardware read/control access
Exec=/usr/local/bin/nexa-ai-gui
Icon=nexa-ai
Terminal=false
Categories=System;Utility;
DESKTOP_EOF

echo "nexa-ai staged into $rootfs"
