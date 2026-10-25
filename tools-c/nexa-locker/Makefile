# nexa-locker: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
# The protocol code is generated outside the source tree (the container runs
# as root, and generated files don't belong in the repo).
PKGS = wayland-client wayland-cursor xkbcommon cairo pangocairo
PROTO = $(shell pkg-config --variable=pkgdatadir wayland-protocols)/staging/ext-session-lock/ext-session-lock-v1.xml
GEN ?= /tmp/nexa-locker-gen
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE \
          -I$(GEN) $(shell pkg-config --cflags $(PKGS))
LDFLAGS += -pie -Wl,-z,relro,-z,now
LDLIBS = $(shell pkg-config --libs $(PKGS)) -lpam -lm

nexa-locker: nexa-locker.c $(GEN)/ext-session-lock-v1-client-protocol.h $(GEN)/ext-session-lock-v1-protocol.c
	$(CC) $(CFLAGS) -o $@ nexa-locker.c $(GEN)/ext-session-lock-v1-protocol.c $(LDFLAGS) $(LDLIBS)

$(GEN)/ext-session-lock-v1-client-protocol.h:
	mkdir -p $(GEN)
	wayland-scanner client-header $(PROTO) $@

$(GEN)/ext-session-lock-v1-protocol.c:
	mkdir -p $(GEN)
	wayland-scanner private-code $(PROTO) $@

clean:
	rm -f nexa-locker
