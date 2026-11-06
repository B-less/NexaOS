# Mail: built by nexa-pack (make), in the nexa-rust-apps image.
# Hardened build: stack protector, fortify, position-independent, full RELRO.
# make test: the engine's unit tests (no screen, no network).
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Werror -fstack-protector-strong \
          -D_FORTIFY_SOURCE=2 -fPIE
LDFLAGS += -pie -Wl,-z,relro,-z,now
ENGINE = src/mime.c src/imap.c src/account.c
HEADERS = src/mime.h src/imap.h src/account.h

build/mail: src/main.c $(ENGINE) $(HEADERS) | build
	$(CC) $(CFLAGS) $(shell pkg-config --cflags gtk+-3.0 libcurl libcrypto) -o $@ src/main.c $(ENGINE) $(LDFLAGS) $(shell pkg-config --libs gtk+-3.0 libcurl libcrypto)

build/unit: tests/unit.c $(ENGINE) $(HEADERS) | build
	$(CC) $(CFLAGS) $(shell pkg-config --cflags glib-2.0 gio-2.0 libcurl libcrypto) -o $@ tests/unit.c $(ENGINE) $(LDFLAGS) $(shell pkg-config --libs glib-2.0 gio-2.0 libcurl libcrypto)

test: build/unit
	./build/unit

build:
	mkdir -p build

clean:
	rm -rf build dist
.PHONY: clean test
