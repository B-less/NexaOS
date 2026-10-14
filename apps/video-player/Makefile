# Hardened build: stack protector, fortify, position-independent, full RELRO.
PKGS = gtk+-3.0 gstreamer-1.0 gstreamer-app-1.0
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Werror -fstack-protector-strong \
          -D_FORTIFY_SOURCE=2 -fPIE $(shell pkg-config --cflags $(PKGS))
LDFLAGS += -pie -Wl,-z,relro,-z,now
LDLIBS += $(shell pkg-config --libs $(PKGS)) -lm

build/video-player: src/main.c | build
	$(CC) $(CFLAGS) -o $@ $< $(LDFLAGS) $(LDLIBS)

build:
	mkdir -p build

clean:
	rm -rf build dist
.PHONY: clean
