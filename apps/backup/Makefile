# Backup: built by nexa-pack (make), in the nexa-rust-apps image.
# Hardened build: stack protector, fortify, position-independent, full RELRO.
# make test: the engine's unit tests (no screen needed).
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Werror -fstack-protector-strong \
          -D_FORTIFY_SOURCE=2 -fPIE
LDFLAGS += -pie -Wl,-z,relro,-z,now

build/backup: src/main.c src/engine.c src/engine.h | build
	$(CC) $(CFLAGS) $(shell pkg-config --cflags gtk+-3.0) -o $@ src/main.c src/engine.c $(LDFLAGS) $(shell pkg-config --libs gtk+-3.0)

build/unit: tests/unit.c src/engine.c src/engine.h | build
	$(CC) $(CFLAGS) $(shell pkg-config --cflags gio-2.0) -o $@ tests/unit.c src/engine.c $(LDFLAGS) $(shell pkg-config --libs gio-2.0)

test: build/unit
	./build/unit

build:
	mkdir -p build

clean:
	rm -rf build dist
.PHONY: clean test
