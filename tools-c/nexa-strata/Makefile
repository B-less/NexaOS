# nexa-strata: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
# The layer stack (hud.c) is built into nexa-orbit, the tint into
# nexa-living; layers.c is shared by all three.
# strata-test: unit tests for layers.c (make test).
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE \
          -I../nexa-orbit -I../nexa-living
LDFLAGS += -pie -Wl,-z,relro,-z,now
SRC = nexa-strata.c layers.c ../nexa-orbit/wm.c ../nexa-living/json.c

nexa-strata: $(SRC) layers.h ../nexa-orbit/wm.h
	$(CC) $(CFLAGS) -o $@ $(SRC) $(LDFLAGS) -lm

strata-test: strata-test.c layers.c layers.h
	$(CC) $(CFLAGS) -o $@ strata-test.c layers.c $(LDFLAGS) -lm

test: strata-test
	./strata-test

clean:
	rm -f nexa-strata strata-test
