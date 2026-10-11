CC = musl-gcc
CFLAGS = -I. -O3 -Wall -Ilinux_hdrs
LDFLAGS = -static -lm
CSRCS = $(shell find lvgl/src lv_drivers/display lv_drivers/indev -name \*.c) nexa_ui.c
OBJS = $(CSRCS:.c=.o)

nexa_gui: $(OBJS)
	$(CC) -o $@ $^ $(LDFLAGS)

clean:
	rm -f $(OBJS) nexa_gui
