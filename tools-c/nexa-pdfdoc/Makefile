# nexa-pdfdoc: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
PKGS = poppler-glib gio-2.0 cairo
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Werror -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE $(shell pkg-config --cflags $(PKGS))
LDFLAGS += -pie -Wl,-z,relro,-z,now
LDLIBS = $(shell pkg-config --libs $(PKGS)) -lm

nexa-pdfdoc: nexa-pdfdoc.c
	$(CC) $(CFLAGS) -o $@ $< $(LDFLAGS) $(LDLIBS)

clean:
	rm -f nexa-pdfdoc
