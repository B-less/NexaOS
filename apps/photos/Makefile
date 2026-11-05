# Photos: built by nexa-pack (make), in the nexa-rust-apps image.
# Hardened build: stack protector, fortify, position-independent, full RELRO.
# make test: the engine's unit tests (no screen needed).
PKGS = gtk+-3.0 libjpeg
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Werror -fstack-protector-strong \
          -D_FORTIFY_SOURCE=2 -fPIE
LDFLAGS += -pie -Wl,-z,relro,-z,now
ENGINE = src/exif.c src/library.c src/store.c src/thumbs.c
HEADERS = src/exif.h src/library.h src/store.h src/thumbs.h

build/photos: src/main.c $(ENGINE) $(HEADERS) | build
	$(CC) $(CFLAGS) $(shell pkg-config --cflags $(PKGS)) -o $@ src/main.c $(ENGINE) $(LDFLAGS) $(shell pkg-config --libs $(PKGS)) -lm

build/unit: tests/unit.c $(ENGINE) $(HEADERS) | build
	$(CC) $(CFLAGS) $(shell pkg-config --cflags gio-2.0 gdk-pixbuf-2.0 cairo libjpeg) -o $@ tests/unit.c $(ENGINE) \
	    $(LDFLAGS) $(shell pkg-config --libs gio-2.0 gdk-pixbuf-2.0 cairo libjpeg) -lm

test: build/unit
	./build/unit

build:
	mkdir -p build

clean:
	rm -rf build dist
.PHONY: clean test
