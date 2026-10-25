# nexa-living: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
# nexa-living-preview (one frame to PNG, or a timing run) is for tests.
PKGS = wayland-client cairo pixman-1
WLR = /usr/share/wlr-protocols/unstable/wlr-layer-shell-unstable-v1.xml
XDG = $(shell pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml
GEN ?= /tmp/nexa-living-gen
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE \
          -I$(GEN) $(shell pkg-config --cflags $(PKGS))
LDFLAGS += -pie -Wl,-z,relro,-z,now
LDLIBS = $(shell pkg-config --libs $(PKGS)) -lm
CODE = $(GEN)/layer-shell.c $(GEN)/xdg-shell.c

nexa-living: nexa-living.c styles.c styles.h json.c json.h $(GEN)/layer-shell.h $(CODE)
	$(CC) $(CFLAGS) -o $@ nexa-living.c styles.c json.c $(CODE) $(LDFLAGS) $(LDLIBS)

nexa-living-preview: preview.c styles.c styles.h
	$(CC) $(CFLAGS) -o $@ preview.c styles.c $(LDFLAGS) $(LDLIBS)

$(GEN)/layer-shell.h:
	mkdir -p $(GEN)
	wayland-scanner client-header $(WLR) $@
$(GEN)/layer-shell.c:
	mkdir -p $(GEN)
	wayland-scanner private-code $(WLR) $@
$(GEN)/xdg-shell.c:
	mkdir -p $(GEN)
	wayland-scanner private-code $(XDG) $@

clean:
	rm -f nexa-living nexa-living-preview
