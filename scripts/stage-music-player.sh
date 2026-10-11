#!/bin/sh
# Stage the nexa-music GStreamer-backed music player into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-music" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-music/nexa_music"
cp -a "$base/music-player/nexa_music" "$rootfs/usr/local/lib/nexa-music/nexa_music"
find "$rootfs/usr/local/lib/nexa-music/nexa_music" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-music:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_music "$@"' \
    > "$rootfs/usr/local/bin/nexa-music"
chmod +x "$rootfs/usr/local/bin/nexa-music"

cat > "$rootfs/usr/share/applications/nexa-music.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Music
Comment=Play local audio files
Exec=/usr/local/bin/nexa-music %f
Icon=nexa-music
Terminal=false
Categories=AudioVideo;Player;
MimeType=audio/mpeg;audio/x-wav;audio/ogg;audio/flac;
DESKTOP_EOF

echo "nexa-music staged into $rootfs"
