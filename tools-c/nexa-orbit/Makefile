# nexa-orbit: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
# It also hosts Strata's layer stack (../nexa-strata/hud.c).
# orbit-test: unit tests for the parts with no screen in them (make test).
PKGS = gtk+-3.0 gtk-layer-shell-0 gio-unix-2.0
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE -I../nexa-living -I../nexa-strata
LDFLAGS += -pie -Wl,-z,relro,-z,now
SRC = nexa-orbit.c model.c wm.c ../nexa-living/json.c ../nexa-strata/hud.c ../nexa-strata/layers.c

nexa-orbit: $(SRC) model.h wm.h ../nexa-strata/hud.h ../nexa-strata/layers.h
	$(CC) $(CFLAGS) $(shell pkg-config --cflags $(PKGS)) -o $@ $(SRC) $(LDFLAGS) $(shell pkg-config --libs $(PKGS)) -lm

orbit-test: orbit-test.c model.c model.h
	$(CC) $(CFLAGS) -o $@ orbit-test.c model.c $(LDFLAGS)

test: orbit-test
	./orbit-test

clean:
	rm -f nexa-orbit orbit-test
