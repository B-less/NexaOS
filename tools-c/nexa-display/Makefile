# nexa-display: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
PKGS = wayland-client
PROTO = /usr/share/wlr-protocols/unstable/wlr-output-management-unstable-v1.xml
GEN ?= /tmp/nexa-display-gen
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE \
          -I$(GEN) $(shell pkg-config --cflags $(PKGS))
LDFLAGS += -pie -Wl,-z,relro,-z,now
LDLIBS = $(shell pkg-config --libs $(PKGS))

nexa-display: nexa-display.c ../nexa-living/json.c ../nexa-living/json.h $(GEN)/wlr-output-management-unstable-v1-client-protocol.h $(GEN)/output-management.c
	$(CC) $(CFLAGS) -o $@ nexa-display.c ../nexa-living/json.c $(GEN)/output-management.c $(LDFLAGS) $(LDLIBS)

$(GEN)/wlr-output-management-unstable-v1-client-protocol.h:
	mkdir -p $(GEN)
	wayland-scanner client-header $(PROTO) $@
$(GEN)/output-management.c:
	mkdir -p $(GEN)
	wayland-scanner private-code $(PROTO) $@

clean:
	rm -f nexa-display
