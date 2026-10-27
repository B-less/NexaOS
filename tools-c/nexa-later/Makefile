# nexa-later: built by scripts/build-c-tools.sh in the nexa-rust-apps image.
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE
LDFLAGS += -pie -Wl,-z,relro,-z,now

nexa-later: nexa-later.c
	$(CC) $(CFLAGS) -o $@ nexa-later.c $(LDFLAGS)

clean:
	rm -f nexa-later
