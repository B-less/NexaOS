#!/bin/sh
# Video in Nexa Browser, inside the real system image (rootfs/ imported as a
# container): clips in each codec sites use, HLS, and streamed playback
# (MediaSource, as YouTube plays), each checked for really playing.
# Clips are made with ffmpeg in a throwaway container (dist/media-test).
# Usage: sh scripts/test-browser-media.sh   (needs rootfs/: make refresh)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
media=$repo/dist/media-test
if [ ! -s "$media/frag.mp4" ]; then
    mkdir -p "$media"
    docker run --rm -v "$media:/o" alpine:3.24 sh -c '
apk add -q --no-cache ffmpeg >/dev/null 2>&1; cd /o
src="-f lavfi -i testsrc2=size=640x360:rate=30:duration=6 -f lavfi -i sine=frequency=440:duration=6"
ffmpeg -loglevel error -y $src -c:v libx264 -pix_fmt yuv420p -c:a aac -movflags +faststart h264.mp4
ffmpeg -loglevel error -y $src -c:v libvpx-vp9 -b:v 500k -c:a libopus vp9.webm
ffmpeg -loglevel error -y $src -c:v libvpx -b:v 500k -c:a libvorbis vp8.webm
ffmpeg -loglevel error -y $src -c:v libaom-av1 -cpu-used 8 -b:v 300k -c:a libopus av1.webm
ffmpeg -loglevel error -y $src -c:v libx264 -pix_fmt yuv420p -c:a aac -f hls -hls_time 2 -hls_playlist_type vod hls.m3u8
ffmpeg -loglevel error -y $src -c:v libx264 -pix_fmt yuv420p -c:a aac -movflags frag_keyframe+empty_moov+default_base_moof frag.mp4'
fi
cp "$repo/common/tests/browser_media.html" "$media/index.html"
# The system image as it is now (rebuilt when rootfs/ changed).
stamp=$(stat -c %Y "$repo/rootfs/usr/lib" 2>/dev/null || echo 0)
if [ "$(docker image inspect -f '{{index .Config.Labels "stamp"}}' nexa-sys:test 2>/dev/null)" != "$stamp" ]; then
    echo "importing the system image (rootfs/) ..."
    docker run --rm -v "$repo/rootfs:/r:ro" alpine:3.24 tar -C /r -c . \
        | docker import -c "LABEL stamp=$stamp" - nexa-sys:test >/dev/null
fi
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$media:/media" nexa-sys:test sh /src/common/tests/browser_media.sh
