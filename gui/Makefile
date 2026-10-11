CC ?= gcc
CFLAGS ?= -Wall -Wextra -O2
PKG_CONFIG ?= pkg-config

SDL_CFLAGS := $(shell $(PKG_CONFIG) --cflags sdl2 SDL2_ttf SDL2_image)
SDL_LIBS := $(shell $(PKG_CONFIG) --libs sdl2 SDL2_ttf SDL2_image)

TARGET := nexa-desktop
FB_TARGET := nexa-fbdesktop
SRC := nexa_desktop.c
FB_SRC := fb_desktop.c
APP_BUILD_DIR := build_apps
APP_NAMES := about archive audio calc clock csv_view editor files image md_view props sysmon term todo
APP_TARGETS := $(addprefix $(APP_BUILD_DIR)/,$(APP_NAMES))

.PHONY: all apps clean run

all: $(TARGET) $(FB_TARGET) apps

$(TARGET): $(SRC)
	$(CC) $(CFLAGS) $(SDL_CFLAGS) -o $@ $(SRC) $(SDL_LIBS)

$(FB_TARGET): $(FB_SRC)
	$(CC) $(CFLAGS) -static -o $@ $(FB_SRC)

apps: $(APP_TARGETS)

$(APP_BUILD_DIR):
	mkdir -p $@

$(APP_BUILD_DIR)/clock: apps/clock.c libnexa.h | $(APP_BUILD_DIR)
	$(CC) $(CFLAGS) -static -o $@ $< -lm

$(APP_BUILD_DIR)/%: apps/%.c libnexa.h | $(APP_BUILD_DIR)
	$(CC) $(CFLAGS) -static -o $@ $<

run: $(TARGET)
	./$(TARGET)

clean:
	rm -f $(TARGET) $(FB_TARGET) $(APP_TARGETS)
