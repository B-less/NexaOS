# nexa-constellation: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
# The sky itself (sky.c, chart.c) is built into nexa-orbit; this folder's
# program is the small command that opens and closes it (Super+Tab).
# chart-test: unit tests for chart.c (make test).
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE
LDFLAGS += -pie -Wl,-z,relro,-z,now

nexa-constellation: nexa-constellation.c
	$(CC) $(CFLAGS) -o $@ nexa-constellation.c $(LDFLAGS)

chart-test: chart-test.c chart.c chart.h
	$(CC) $(CFLAGS) $(shell pkg-config --cflags glib-2.0) -o $@ chart-test.c chart.c $(LDFLAGS) $(shell pkg-config --libs glib-2.0) -lm

test: chart-test
	./chart-test

clean:
	rm -f nexa-constellation chart-test
