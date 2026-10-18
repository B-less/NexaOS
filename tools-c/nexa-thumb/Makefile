# nexa-thumb: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
PKGS = gdk-pixbuf-2.0 gio-2.0 gstreamer-1.0 gstreamer-video-1.0 cairo
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE $(shell pkg-config --cflags $(PKGS))
LDFLAGS += -pie -Wl,-z,relro,-z,now
LDLIBS = $(shell pkg-config --libs $(PKGS)) -lm

nexa-thumb: nexa-thumb.c
	$(CC) $(CFLAGS) -o $@ $< $(LDFLAGS) $(LDLIBS)

clean:
	rm -f nexa-thumb
