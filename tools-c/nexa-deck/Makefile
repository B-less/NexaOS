# nexa-deck: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
# deck-test: unit tests for the parts with no screen in them (make test).
PKGS = gtk+-3.0 gtk-layer-shell-0 gio-unix-2.0
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE -I../nexa-living
LDFLAGS += -pie -Wl,-z,relro,-z,now
SRC = nexa-deck.c model.c ../nexa-living/json.c

nexa-deck: $(SRC) model.h
	$(CC) $(CFLAGS) $(shell pkg-config --cflags $(PKGS)) -o $@ $(SRC) $(LDFLAGS) $(shell pkg-config --libs $(PKGS)) -lm

deck-test: deck-test.c model.c model.h ../nexa-living/json.c
	$(CC) $(CFLAGS) -o $@ deck-test.c model.c ../nexa-living/json.c $(LDFLAGS)

test: deck-test
	./deck-test

clean:
	rm -f nexa-deck deck-test
