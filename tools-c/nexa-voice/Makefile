# nexa-voice: Nexa Mind's voice and ears on the desktop, with the helix.
# Built by scripts/build-c-tools.sh in the nexa-rust-apps image.
# voice-test: unit tests for the helix and voiceprint, no screen (make test).
PKGS = gtk+-3.0 gtk-layer-shell-0 gstreamer-1.0 gstreamer-app-1.0
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE -I../nexa-living
LDFLAGS += -pie -Wl,-z,relro,-z,now

nexa-voice: nexa-voice.c helix.c helix.h ../nexa-living/json.c ../nexa-living/json.h
	$(CC) $(CFLAGS) $(shell pkg-config --cflags $(PKGS)) -o $@ nexa-voice.c helix.c ../nexa-living/json.c $(LDFLAGS) $(shell pkg-config --libs $(PKGS)) -lm

voice-test: voice-test.c helix.c helix.h
	$(CC) $(CFLAGS) $(shell pkg-config --cflags cairo) -o $@ voice-test.c helix.c $(LDFLAGS) $(shell pkg-config --libs cairo) -lm

test: voice-test
	./voice-test

clean:
	rm -f nexa-voice voice-test
