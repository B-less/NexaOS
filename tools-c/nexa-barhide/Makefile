# nexa-barhide: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
PKGS = wayland-client
WLR = /usr/share/wlr-protocols/unstable/wlr-layer-shell-unstable-v1.xml
COPY = /usr/share/wlr-protocols/unstable/wlr-screencopy-unstable-v1.xml
XDG = $(shell pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml
GEN ?= /tmp/nexa-barhide-gen
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE \
          -I$(GEN) $(shell pkg-config --cflags $(PKGS))
LDFLAGS += -pie -Wl,-z,relro,-z,now
LDLIBS = $(shell pkg-config --libs $(PKGS)) -lm
CODE = $(GEN)/layer-shell.c $(GEN)/xdg-shell.c $(GEN)/screencopy.c

nexa-barhide: nexa-barhide.c $(GEN)/layer-shell.h $(GEN)/screencopy.h $(CODE)
	$(CC) $(CFLAGS) -o $@ nexa-barhide.c $(CODE) $(LDFLAGS) $(LDLIBS)

$(GEN)/layer-shell.h:
	mkdir -p $(GEN)
	wayland-scanner client-header $(WLR) $@
$(GEN)/layer-shell.c:
	mkdir -p $(GEN)
	wayland-scanner private-code $(WLR) $@
$(GEN)/screencopy.h:
	mkdir -p $(GEN)
	wayland-scanner client-header $(COPY) $@
$(GEN)/screencopy.c:
	mkdir -p $(GEN)
	wayland-scanner private-code $(COPY) $@
$(GEN)/xdg-shell.c:
	mkdir -p $(GEN)
	wayland-scanner private-code $(XDG) $@

clean:
	rm -f nexa-barhide
