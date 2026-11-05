# nexa-signals: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
PKGS = gtk+-3.0 gtk-layer-shell-0 gio-unix-2.0
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE -I../nexa-living
LDFLAGS += -pie -Wl,-z,relro,-z,now
SRC = nexa-signals.c ../nexa-living/json.c

nexa-signals: $(SRC)
	$(CC) $(CFLAGS) $(shell pkg-config --cflags $(PKGS)) -o $@ $(SRC) $(LDFLAGS) $(shell pkg-config --libs $(PKGS)) -lm

clean:
	rm -f nexa-signals
