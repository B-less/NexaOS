# nexa-speak: Nexa Mind's voice (Kokoro through onnxruntime). Built by
# scripts/build-c-tools.sh in the nexa-rust-apps image; nexa-mindd runs it.
# speak-test: unit tests for the text side, no model needed (make test).
CFLAGS ?= -O2 -g0
CFLAGS += -std=gnu11 -Wall -Wextra -Wno-unused-parameter -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fPIE
LDFLAGS += -pie -Wl,-z,relro,-z,now

nexa-speak: nexa-speak.c kokoro.c kokoro.h
	$(CC) $(CFLAGS) -o $@ nexa-speak.c kokoro.c $(LDFLAGS) -lonnxruntime

speak-test: speak-test.c kokoro.c kokoro.h
	$(CC) $(CFLAGS) -o $@ speak-test.c kokoro.c $(LDFLAGS)

test: speak-test
	./speak-test

clean:
	rm -f nexa-speak speak-test
