#!/usr/bin/make -f
# NexaOS Build System - Modern Makefile
# This replaces all individual build scripts with a unified, enterprise-grade build system

include build.conf

empty :=
space := $(empty) $(empty)
comma := ,
PKG_CORE := $(subst $(comma),$(space),$(ALPINE_PACKAGES_CORE))
PKG_WAYLAND := $(subst $(comma),$(space),$(ALPINE_PACKAGES_WAYLAND))
PKG_SWAY := $(subst $(comma),$(space),$(ALPINE_PACKAGES_SWAY))
PKG_TERM := $(subst $(comma),$(space),$(ALPINE_PACKAGES_TERM))
PKG_UTILS := $(subst $(comma),$(space),$(ALPINE_PACKAGES_UTILS))

# Package installation, identity/config generation, and disk-image population all
# need root (apk leaves root-owned/unreadable files by design, e.g. busybox-suid's
# bbsuid; mke2fs -d needs to read those). Rather than requiring host `sudo`, those
# steps run as root inside a throwaway Alpine container — reproducible and CI-friendly.
ALPINE_DOCKER_TAG := $(shell echo $(ALPINE_VERSION) | cut -d. -f1,2)
# Build steps run in nexa-build (Alpine + the build tools, made once by
# scripts/build-image.sh), so a rebuild doesn't download the tools again.
DOCKER_RUN := sh $(BASE_DIR)/scripts/build-image.sh && docker run --rm -v $(BASE_DIR):/workspace -w /workspace nexa-build:$(ALPINE_DOCKER_TAG)
# The ISO step also gets the Secure Boot key (read-only; scripts/secureboot-key.sh,
# never in the repo) and Debian's Microsoft-signed shim (scripts/fetch-shim.sh).
# Without either the ISO still builds, for Secure Boot off (docs/SECURE_BOOT.md).
SB_KEYS ?= $(HOME)/.config/nexaos-keys/secureboot
DOCKER_RUN_ISO := sh $(BASE_DIR)/scripts/build-image.sh && { sh $(BASE_DIR)/scripts/fetch-shim.sh || echo "[!] couldn't fetch the shim: this ISO needs Secure Boot off"; } && docker run --rm -v $(BASE_DIR):/workspace $(if $(wildcard $(SB_KEYS)/nexa-sb.key),-v $(SB_KEYS):/sbkeys:ro -e NEXA_SB_KEYS=/sbkeys,) -w /workspace nexa-build:$(ALPINE_DOCKER_TAG)
# Paths as seen *inside* the container (workspace root == BASE_DIR), since the
# absolute host paths in ROOTFS_DIR/DIST_DIR don't exist there.
ROOTFS_RELDIR := $(patsubst $(BASE_DIR)/%,%,$(ROOTFS_DIR))
DIST_RELDIR := $(patsubst $(BASE_DIR)/%,%,$(DIST_DIR))

.PHONY: help all clean distclean setup build boot iso package docker ci test docs
.PHONY: kernel rootfs disk initramfs apps nexa-apps gui-apps stage-gui-rootfs install-deps check-env refresh native test-native test-files-app test-desktop-card test-titlebar test-store-app publish-store iconset test-theme-live test-desktop-drives preload-map test-nexa-later test-settings-devices test-bootfix
.SECONDARY:

# ============================================================================
# HELP & INITIALIZATION
# ============================================================================

help:
	@echo "$(COLOR_BLUE)=== NexaOS Build System v$(PROJECT_VERSION) ===$(COLOR_RESET)"
	@echo ""
	@echo "$(COLOR_GREEN)Targets:$(COLOR_RESET)"
	@echo "  all              - Full build (kernel, rootfs, disk, iso)"
	@echo "  setup            - Create directories and prepare environment"
	@echo "  clean            - Remove build artifacts"
	@echo "  distclean        - Remove everything including binaries and disk"
	@echo ""
	@echo "$(COLOR_GREEN)Build Targets:$(COLOR_RESET)"
	@echo "  kernel           - Build Linux kernel"
	@echo "  rootfs           - Build root filesystem"
	@echo "  disk             - Create and populate disk image"
	@echo "  initramfs        - Build initramfs"
	@echo "  apps             - Build all applications"
	@echo "  gui              - Build GUI components"
	@echo ""
	@echo "$(COLOR_GREEN)Utility Targets:$(COLOR_RESET)"
	@echo "  boot             - Boot system in QEMU (serial mode)"
	@echo "  boot-gui         - Boot system in QEMU (Wayland/Sway mode)"
	@echo "  iso              - Create bootable ISO image"
	@echo "  docker           - Build in Docker container"
	@echo "  ci               - Run CI pipeline locally"
	@echo "  check-env        - Verify build dependencies"
	@echo "  test             - Run system tests"
	@echo ""
	@echo "$(COLOR_GREEN)Configuration:$(COLOR_RESET)"
	@echo "  Edit build.conf to customize versions, flags, and features"
	@echo ""

# ============================================================================
# ENVIRONMENT SETUP
# ============================================================================

check-env:
	@echo "$(COLOR_BLUE)Checking build environment...$(COLOR_RESET)"
	@command -v gcc >/dev/null 2>&1 || { echo "ERROR: gcc not found"; exit 1; }
	@command -v make >/dev/null 2>&1 || { echo "ERROR: make not found"; exit 1; }
	@command -v qemu-system-x86_64 >/dev/null 2>&1 || { echo "WARN: QEMU not found"; }
	@command -v docker >/dev/null 2>&1 || { echo "WARN: Docker not found"; }
	@echo "$(COLOR_GREEN)✓ Environment verified$(COLOR_RESET)"

setup: check-env
	@echo "$(COLOR_BLUE)Setting up build directories...$(COLOR_RESET)"
	@mkdir -p $(BUILD_DIR) $(DIST_DIR) $(MNT_DIR) $(CACHE_DIR)
	@mkdir -p $(ROOTFS_DIR)/bin $(ROOTFS_DIR)/sbin $(ROOTFS_DIR)/etc $(ROOTFS_DIR)/dev $(ROOTFS_DIR)/home $(ROOTFS_DIR)/lib $(ROOTFS_DIR)/root $(ROOTFS_DIR)/tmp $(ROOTFS_DIR)/usr $(ROOTFS_DIR)/var $(ROOTFS_DIR)/apps $(ROOTFS_DIR)/packages $(ROOTFS_DIR)/logs
	@touch $(BUILD_LOG)
	@echo "$(COLOR_GREEN)✓ Build directories ready$(COLOR_RESET)"

# ============================================================================
# KERNEL BUILD
# ============================================================================

kernel: setup
	@echo "$(COLOR_BLUE)[1/5] Building Linux kernel $(KERNEL_VERSION)...$(COLOR_RESET)"
	@if [ ! -d $(KERNEL_DIR)/linux-$(KERNEL_VERSION) ]; then \
		echo "ERROR: Kernel source not found. Please extract kernel first."; \
		exit 1; \
	fi
	@# Through tee for the log, but a failed build must stop here: /bin/sh
	@# has no pipefail, so the build's own exit status goes through a file
	@# (it used to say "Kernel built" and copy the old bzImage).
	@cd $(KERNEL_DIR)/linux-$(KERNEL_VERSION) && \
		{ $(MAKE) -j$(PARALLEL_JOBS) bzImage 2>&1; echo $$? > $(BUILD_DIR)/.kernel-status; } | tee -a $(BUILD_LOG); \
		status=$$(cat $(BUILD_DIR)/.kernel-status); rm -f $(BUILD_DIR)/.kernel-status; \
		if [ "$$status" != 0 ]; then echo "$(COLOR_RED)✗ Kernel build failed (see above)$(COLOR_RESET)"; exit 1; fi
	@cp $(KERNEL_DIR)/linux-$(KERNEL_VERSION)/arch/x86/boot/bzImage $(DIST_DIR)/bzImage
	@echo "$(COLOR_GREEN)✓ Kernel built: $(DIST_DIR)/bzImage$(COLOR_RESET)"

# ============================================================================
# CORE BINARIES
# ============================================================================

# Order-only prerequisite (the "| ..." part): ensures $(BUILD_DIR) exists
# before compiling into it, without making every binary depend on the
# directory's mtime (which would force a needless relink whenever anything
# else in build/ changes). Without this, a fully-clean `build/` (e.g. right
# after `make clean`, which only removes the directory and never recreates
# it) makes the compiler fail with "cannot open output file ... No such
# file or directory" instead of just creating it.
$(BUILD_DIR):
	@mkdir -p $(BUILD_DIR)

$(BUILD_DIR)/init_bin: $(BASE_DIR)/init.c | $(BUILD_DIR)
	@echo "Compiling init..."
	@$(CC) $(CFLAGS_COMMON) $(CFLAGS_STATIC) -o $@ $<

$(BUILD_DIR)/httpd_bin: $(BASE_DIR)/httpd.c | $(BUILD_DIR)
	@echo "Compiling httpd..."
	@$(CC) $(CFLAGS_COMMON) $(CFLAGS_STATIC) -o $@ $<

$(BUILD_DIR)/monitor_bin: $(BASE_DIR)/monitor.c | $(BUILD_DIR)
	@echo "Compiling monitor..."
	@$(CC) $(CFLAGS_COMMON) $(CFLAGS_STATIC) -o $@ $<

$(BUILD_DIR)/ping_bin: $(BASE_DIR)/ping.c | $(BUILD_DIR)
	@echo "Compiling ping..."
	@$(CC) $(CFLAGS_COMMON) $(CFLAGS_STATIC) -o $@ $<

$(BUILD_DIR)/nettest_bin: $(BASE_DIR)/nettest.c | $(BUILD_DIR)
	@echo "Compiling nettest..."
	@$(CC) $(CFLAGS_COMMON) $(CFLAGS_STATIC) -o $@ $<

core-binaries: $(BUILD_DIR)/init_bin $(BUILD_DIR)/httpd_bin $(BUILD_DIR)/monitor_bin \
               $(BUILD_DIR)/ping_bin $(BUILD_DIR)/nettest_bin $(BUILD_DIR)/bootsplash_bin
	@echo "$(COLOR_GREEN)✓ Core binaries compiled$(COLOR_RESET)"

$(BUILD_DIR)/bootsplash_bin: $(BASE_DIR)/bootsplash.c $(BASE_DIR)/font8x8.h | $(BUILD_DIR)
	@echo "Compiling bootsplash..."
	@$(CC) $(CFLAGS_COMMON) $(CFLAGS_STATIC) -o $@ $< -lm

# ============================================================================
# KERNEL MODULE (nexa_proc)
# ============================================================================

kernel-module: kernel
	@echo "$(COLOR_BLUE)[2/5] Building nexa_proc kernel module...$(COLOR_RESET)"
	@cd $(BASE_DIR)/nexa_proc && \
		{ $(MAKE) -j$(PARALLEL_JOBS) 2>&1; echo $$? > $(BUILD_DIR)/.module-status; } | tee -a $(BUILD_LOG); \
		status=$$(cat $(BUILD_DIR)/.module-status); rm -f $(BUILD_DIR)/.module-status; [ "$$status" = 0 ]
	@mkdir -p $(ROOTFS_DIR)/lib/modules
	@cp $(BASE_DIR)/nexa_proc/nexa_proc.ko $(ROOTFS_DIR)/lib/modules/
	@echo "$(COLOR_GREEN)✓ Kernel module built$(COLOR_RESET)"

# ============================================================================
# ROOT FILESYSTEM (Alpine Linux Bootstrap)
# ============================================================================

alpine-download:
	@echo "$(COLOR_BLUE)Downloading Alpine Linux $(ALPINE_VERSION)...$(COLOR_RESET)"
	@mkdir -p $(CACHE_DIR)
	@if [ ! -f $(CACHE_DIR)/alpine-minirootfs-$(ALPINE_VERSION)-$(ALPINE_ARCH).tar.gz ]; then \
		curl -L -o $(CACHE_DIR)/alpine-minirootfs-$(ALPINE_VERSION)-$(ALPINE_ARCH).tar.gz \
		$(ALPINE_REPO)/releases/$(ALPINE_ARCH)/alpine-minirootfs-$(ALPINE_VERSION)-$(ALPINE_ARCH).tar.gz || exit 1; \
	fi
	@echo "$(COLOR_GREEN)✓ Alpine minirootfs cached$(COLOR_RESET)"

rootfs: alpine-download core-binaries
	@echo "$(COLOR_BLUE)[3/5] Extracting Alpine Linux base...$(COLOR_RESET)"
	@# A previous provision-rootfs run may have left root-owned files/dirs (apk
	@# packages install some as root:root by design), which the nexa user can't
	@# rm -rf; clean up as root in a container instead of requiring host sudo.
	@[ -d $(ROOTFS_DIR) ] && $(DOCKER_RUN) rm -rf $(ROOTFS_RELDIR) || true
	@mkdir -p $(ROOTFS_DIR)
	@cd $(ROOTFS_DIR) && tar -xzf $(CACHE_DIR)/alpine-minirootfs-$(ALPINE_VERSION)-$(ALPINE_ARCH).tar.gz
	@echo "$(COLOR_BLUE)  ✓ Alpine base extracted$(COLOR_RESET)"
	@mkdir -p $(ROOTFS_DIR)/etc/apk
	@printf "$(ALPINE_REPO)/main\n$(ALPINE_REPO)/community\n" > $(ROOTFS_DIR)/etc/apk/repositories
	@echo "$(COLOR_GREEN)✓ Root filesystem extracted (Alpine Linux $(ALPINE_VERSION))$(COLOR_RESET)"

# Installs the full Wayland/Sway/Waybar/Wofi/Foot stack via apk, (re)writes the
# NexaOS identity/config files *after* install so apk can't clobber them, and
# stages the Nexa GUI apps + OpenRC services. Runs as root in a container so it
# never needs host sudo/chroot — see scripts/provision-rootfs.sh.
provision-rootfs: rootfs gui-apps native wallpaper icons
	@echo "$(COLOR_BLUE)Provisioning rootfs: Wayland/Sway packages, identity, GUI apps, configs...$(COLOR_RESET)"
	@$(DOCKER_RUN) sh -c "{ apk info -e bash >/dev/null || apk add --no-cache bash >/dev/null; } && sh scripts/provision-rootfs.sh $(ROOTFS_RELDIR) '$(PKG_CORE)' '$(PKG_WAYLAND)' '$(PKG_SWAY)' '$(PKG_TERM)' '$(PKG_UTILS)' '$(DNS_PRIMARY)' '$(DNS_SECONDARY)'"
	@echo "$(COLOR_GREEN)✓ rootfs provisioned$(COLOR_RESET)"
	@du -sh $(ROOTFS_DIR) 2>/dev/null || true

# Re-applies every NexaOS package, app, config and service to the existing
# rootfs (no wipe and no fresh Alpine base), then rebuilds the initramfs,
# disk image and ISO. Much faster than `make iso` after NexaOS code, config
# or package-list changes; after changing ALPINE_VERSION use `make iso`.
refresh: core-binaries gui-apps native nexa-apps
	@test -d $(ROOTFS_DIR)/etc || { echo "No rootfs yet: run 'make iso' once first."; exit 1; }
	@echo "$(COLOR_BLUE)Refreshing rootfs (packages, apps, configs)...$(COLOR_RESET)"
	@$(DOCKER_RUN) sh -c "{ apk info -e bash >/dev/null || apk add --no-cache bash >/dev/null; } && sh scripts/provision-rootfs.sh $(ROOTFS_RELDIR) '$(PKG_CORE)' '$(PKG_WAYLAND)' '$(PKG_SWAY)' '$(PKG_TERM)' '$(PKG_UTILS)' '$(DNS_PRIMARY)' '$(DNS_SECONDARY)'"
	@mkdir -p $(DIST_DIR)
	@bash scripts/build-initramfs.sh $(ROOTFS_DIR) $(DIST_DIR)/initramfs.cpio.gz
	@rm -f $(DIST_DIR)/nexa.img
	@$(DOCKER_RUN) sh scripts/build-disk-image.sh $(ROOTFS_RELDIR) $(DIST_RELDIR)/nexa.img $(DISK_SIZE)
ifeq ($(BUILD_ISO),yes)
	@$(DOCKER_RUN_ISO) sh scripts/build-iso.sh $(DIST_RELDIR) $(patsubst $(BASE_DIR)/%,%,$(ISO_OUTPUT))
endif
	@echo "$(COLOR_GREEN)✓ Refreshed: initramfs, disk image$(if $(filter yes,$(BUILD_ISO)), and ISO)$(COLOR_RESET)"

# ============================================================================
# INITRAMFS
# ============================================================================

# Minimal early-boot initramfs (busybox + /init only) that finds the disk image
# and switch_roots into it. The full Wayland/Sway desktop lives on the disk
# image, not here, so this stays a few hundred KB instead of embedding rootfs/.
initramfs: provision-rootfs
	@echo "$(COLOR_BLUE)[4/5] Building initramfs...$(COLOR_RESET)"
	@mkdir -p $(DIST_DIR)
	@bash scripts/build-initramfs.sh $(ROOTFS_DIR) $(DIST_DIR)/initramfs.cpio.gz

# ============================================================================
# DISK IMAGE
# ============================================================================

# Populates the ext4 disk image directly from rootfs/ via `mke2fs -d` — no
# mount, no chroot, no host sudo. Runs as root in a container because apk
# leaves some files root-owned/unreadable by design (e.g. busybox-suid's
# bbsuid), which a non-root mke2fs can't read. See scripts/build-disk-image.sh.
disk: initramfs
	@echo "$(COLOR_BLUE)[5/5] Building disk image from rootfs...$(COLOR_RESET)"
	@mkdir -p $(DIST_DIR)
	@rm -f $(DIST_DIR)/nexa.img
	@$(DOCKER_RUN) sh scripts/build-disk-image.sh $(ROOTFS_RELDIR) $(DIST_RELDIR)/nexa.img $(DISK_SIZE)
	@echo "$(COLOR_GREEN)✓ Disk image complete: $(DIST_DIR)/nexa.img$(COLOR_RESET)"
	@ls -lh $(DIST_DIR)/nexa.img

# ============================================================================
# APPLICATIONS & WAYLAND DESKTOP STACK
# ============================================================================

# Native (Rust) tools under tools/: nexa-pack, and the services to come.
# Static binaries in build/native, staged into the image by stage-native.sh.
# NexaOS's own .nexa apps (apps/*), signed when the release key is here.
nexa-apps:
	@sh scripts/build-apps.sh

native:
	@sh scripts/build-native.sh
	@sh scripts/build-c-tools.sh

# The native tools' own tests (cargo test) and nexa-pack end to end
# (templates, signing, tampering), in a clean Alpine container.
test-native:
	@sh scripts/build-native.sh test
	@sh scripts/build-native.sh
	@sh scripts/test-nexa-pack.sh
	@sh scripts/test-nexa-mind.sh

# Windows Boot Repair: a broken fake Windows disk in QEMU, found, fixed and reported
test-bootfix:
	@sh scripts/test-bootfix.sh

# Settings > Display (real nexa-display), Sound, Mouse & Touchpad, Wi-Fi,
# Notifications, Default Apps, Accessibility, Time in a headless Sway.
test-settings-devices:
	@sh scripts/test-settings-devices.sh

# nexa-later (tools-c/nexa-later): starts the session's background services
# once the system has settled; built and checked in Alpine.
test-nexa-later:
	@sh scripts/test-nexa-later.sh

# The Files window driven with mouse, keys, menus, clipboard, Trash and undo
# in a headless Sway, against the real nexa-fileops engine.
test-files-app: native
	@sh scripts/test-files-app.sh

# Boot readahead: boot the fresh ISO once, record which parts of nexa.img the
# boot reads (nexa-preload), and repack the ISO with that map so every later
# boot reads them ahead in large in-order pieces. Run after `make iso`/`refresh`.
# Those stop before packing the ISO when the old map is stale (build-iso.sh),
# so an ISO older than nexa.img is packed again here first: the map must be
# recorded on this nexa.img.
preload-map:
	@rm -f $(DIST_DIR)/nexa-preload.map
	@[ $(ISO_OUTPUT) -nt $(DIST_DIR)/nexa.img ] || { $(DOCKER_RUN_ISO) sh scripts/build-iso.sh $(DIST_RELDIR) $(patsubst $(BASE_DIR)/%,%,$(ISO_OUTPUT)); }
	@python3 scripts/test-live-boot.py --no-checks --shot $(DIST_DIR)/preload-boot.png --fetch /run/nexa-preload/boot.map $(DIST_DIR)/nexa-preload.map
	@# Only a real map goes on the ISO.
	@$(BUILD_DIR)/native/nexa-preload show $(DIST_DIR)/nexa-preload.map || { rm -f $(DIST_DIR)/nexa-preload.map; echo "$(COLOR_RED)✗ No usable boot map was recorded$(COLOR_RESET)"; exit 1; }
	@$(DOCKER_RUN_ISO) sh scripts/build-iso.sh $(DIST_RELDIR) $(patsubst $(BASE_DIR)/%,%,$(ISO_OUTPUT))

# The desktop "For you" card and Nexus' Suggested row (Nexa Patterns) in a
# headless Sway, against the real nexa-patternd with a synthetic month of use.
test-desktop-card: native
	@sh scripts/test-desktop-card.sh

# Publish the signed catalogue to the online Store (B-less/nexa-apps on
# GitHub Pages). Signed here; the key never leaves this machine.
publish-store: nexa-apps
	@sh scripts/publish-store.sh

# The NexaHexa icon set (assets/iconset): generated by the Rust forge from
# one shape description, plus a rendered preview. Not used by the OS yet.
iconset:
	@rm -rf assets/iconset/NexaHexa assets/iconset/NexaHexa-*
	@# The themes' colours, from themes.py, so the variants never drift from it.
	@python3 -c "import sys; sys.path.insert(0, 'common'); from nexa_common import themes; h = lambda c: c if isinstance(c, str) else '#%02x%02x%02x' % tuple(c); print('# name dark accent accent2 surface (from themes.py)'); [print(n, int(t['dark']), t['accent'], t['accent2'], h(t['surface'])) for n, t in themes.THEMES.items()]" > assets/iconset/themes.txt
	@docker run --rm -v "$(BASE_DIR):/src" -v /var/tmp/nexa-cargo:/root/.cargo/registry -w /src/assets/iconset/forge nexa-rust:$(basename $(ALPINE_VERSION)) sh -c 'cargo test -q && cargo run -q --release -- /src/assets/iconset/NexaHexa /src/assets/iconset/themes.txt && chown -R $(shell id -u):$(shell id -g) /src/assets/iconset'
	@docker run --rm -v "$(BASE_DIR)/assets/iconset:/i" nexa-apptest:$(basename $(ALPINE_VERSION)) python3 -c "import gi; gi.require_version('GdkPixbuf', '2.0'); from gi.repository import GdkPixbuf; GdkPixbuf.Pixbuf.new_from_file_at_size('/i/NexaHexa/preview.svg', 1240, -1).savev('/i/preview.png', 'png', [], []); GdkPixbuf.Pixbuf.new_from_file_at_size('/i/NexaHexa/preview-themes.svg', 1240, -1).savev('/i/preview-themes.png', 'png', [], [])" 2>/dev/null
	@echo "icons: assets/iconset/NexaHexa, preview: assets/iconset/preview.png"

# USB drives on the desktop (nexa_shell drives) in a headless Sway.
test-desktop-drives:
	@sh scripts/test-desktop-drives.sh

# Open apps (Python and the C Video Player) follow a theme change live.
test-theme-live: nexa-apps
	@sh scripts/test-theme-live.sh

# The Nexa Store window against the real engine and signed catalogues.
test-store-app: native nexa-apps
	@sh scripts/test-store-app.sh

# The window buttons (minimize / maximize / close) of NexaOS apps and of the
# Video Player clicked with a virtual mouse in a headless Sway.
test-titlebar: nexa-apps
	@sh scripts/test-titlebar.sh

# Nexa MicroBrain as a running service: socket, idle footprint, reaction
# to memory pressure, modes, restart and a damaged settings file.
test-microbrain: native
	@sh scripts/test-microbrain.sh

# Nexa Link between three machines (a PC, a phone, a stranger) on a private
# Docker network: pairing, Nexa-only signals, files, Nexa Drop, Forget.
test-nexa-link: native
	@sh scripts/test-nexa-link.sh

# Nexa Link's screens (Settings, pop-ups, Files) against a real second device.
test-nexa-link-ui: native
	@sh scripts/test-nexa-link-ui.sh

# Several selected files dragged onto a folder with a real mouse (grid, list).
test-files-drag: native
	@sh scripts/test-files-drag.sh

# The Living Field (Settings' front page) with the real services running.
test-living-field: native
	@sh scripts/test-living-field.sh

# The top bar's "Hide when not in use" with the real bar in a headless Sway,
# and its motion both ways (none with Interface DNA "motion": "none").
test-topbar: native
	@sh scripts/test-topbar.sh

# App icons clicked on the icon itself (Nexus, the desktop card, the Store).
test-tile-clicks:
	@sh scripts/test-tile-clicks.sh

# The lock screen (nexa-locker, C) in a headless Sway: lock, wrong and right
# password, a second lock refused, a crash leaves the session locked, and
# Switch person (the hand-over, coming back, a refused switch, Tab + Enter).
test-locker: native
	@sh scripts/test-locker.sh

# The Living Visual Engine (nexa-living, C) with real apps in a headless
# Sway: draws, follows windows, holds still unfocused, Static/Off, theme
# colours, app clicks; prints measured CPU, memory and frame time.
test-living: native
	@sh scripts/test-living.sh

# Why (Magic Layer): the real MicroBrain on a real /proc, real crashes,
# kills, a paused program, a busy loop, networking and none; Pulse's tab.
test-why: native
	@sh scripts/test-why.sh

# Nexa Browser in the real system image: blocking, HTTPS-only, tabs,
# permissions, pop-ups, video in every format, session restore.
test-browser: native
	@sh scripts/test-browser.sh

# Rewind (Magic Layer): real MicroBrain watching real settings files.
test-rewind: native
	@sh scripts/test-rewind.sh

# Settings > Magic Layer driven like a person, with the real renderer.
test-magic-page: native
	@sh scripts/test-magic-page.sh

# nexa-sessiond with real accounts: one desktop per person, who may switch,
# lock on leaving, sign-out going back, the first desktop's restart.
test-sessions: native
	@sh scripts/test-sessions.sh

gui-apps:
	@echo "$(COLOR_BLUE)Building Nexa framebuffer desktop and GUI apps...$(COLOR_RESET)"
	@$(MAKE) -C $(BASE_DIR)/gui nexa-fbdesktop apps
	@echo "$(COLOR_GREEN)GUI binaries built$(COLOR_RESET)"

# Rasterized once at build time (rsvg-convert + fonts stay in the build
# container, never installed in the shipped OS) from assets/wallpaper.svg.
wallpaper:
	@echo "$(COLOR_BLUE)Rendering NexaOS wallpaper...$(COLOR_RESET)"
	@$(DOCKER_RUN) sh scripts/build-wallpaper.sh assets/wallpaper.svg assets/wallpaper.png

icons:
	@echo "$(COLOR_BLUE)Rendering NexaOS app icons...$(COLOR_RESET)"
	@$(DOCKER_RUN) sh scripts/build-icons.sh assets/icons build/icons

# GUI staging (icons, launchers, Sway/Waybar configs) happens inside
# provision-rootfs (scripts/provision-rootfs.sh calls stage-gui-rootfs.sh and
# generate-configs.sh as root, since it writes into apk-owned rootfs paths like
# /etc/runlevels/default). This is a compatibility alias.
stage-gui-rootfs: provision-rootfs

apps: provision-rootfs
	@echo "$(COLOR_GREEN)Nexa GUI apps ready: about archive audio calc clock csv_view editor files image md_view props sysmon term todo$(COLOR_RESET)"

gui: provision-rootfs
	@echo "$(COLOR_GREEN)Wayland/Sway stack installed and framebuffer GUI staged$(COLOR_RESET)"

# ============================================================================
# BOOTABLE ISO CREATION
# ============================================================================

# Bootable BIOS/El-Torito ISO for VirtualBox and other BIOS-mode VMs/PCs.
# Ships the same kernel/initramfs/nexa.img as a direct-disk boot; nexa.img is
# embedded as a plain file on the ISO instead of attached as a virtual disk,
# and /init's live-CD fallback (see build-initramfs.sh) loop-mounts it with
# an overlayfs on top. See scripts/build-iso.sh.
iso: disk
ifeq ($(BUILD_ISO),yes)
	@echo "$(COLOR_BLUE)Creating bootable ISO image...$(COLOR_RESET)"
	@$(DOCKER_RUN_ISO) sh scripts/build-iso.sh $(DIST_RELDIR) $(patsubst $(BASE_DIR)/%,%,$(ISO_OUTPUT))
else
	@echo "ISO building disabled (BUILD_ISO=$(BUILD_ISO))"
endif

# ============================================================================
# QEMU BOOT
# ============================================================================

boot: disk
	@echo "$(COLOR_BLUE)Booting NexaOS in serial mode...$(COLOR_RESET)"
	@echo "Press Ctrl+A then X to exit QEMU"
	@qemu-system-x86_64 \
		-cpu $(QEMU_CPU) -m $(QEMU_RAM) -smp $(QEMU_CORES) \
		-kernel $(DIST_DIR)/bzImage \
		-initrd $(DIST_DIR)/initramfs.cpio.gz \
		-drive if=none,id=nexadisk,file=$(DIST_DIR)/nexa.img,format=raw \
		-device virtio-blk-pci,drive=nexadisk \
		-append "root=/dev/vda console=ttyS0 rootwait" \
		-nographic \
		-serial stdio \
		-net nic,model=virtio \
		-net $(QEMU_NETWORKING)

boot-gui: disk
	@echo "$(COLOR_BLUE)Booting NexaOS with Wayland/Sway...$(COLOR_RESET)"
	@echo "Wayland display will open in a window"
	@qemu-system-x86_64 \
		-cpu $(QEMU_CPU) -m $(QEMU_RAM) -smp $(QEMU_CORES) \
		-enable-kvm \
		-kernel $(DIST_DIR)/bzImage \
		-initrd $(DIST_DIR)/initramfs.cpio.gz \
		-drive if=none,id=nexadisk,file=$(DIST_DIR)/nexa.img,format=raw \
		-device virtio-blk-pci,drive=nexadisk \
		-append "root=/dev/vda console=tty0 rootwait" \
		-device virtio-gpu-pci \
		-device virtio-mouse-pci \
		-device virtio-keyboard-pci \
		-display gtk,show-cursor=on,full-screen=off \
		-net nic,model=virtio \
		-net $(QEMU_NETWORKING)

# ============================================================================
# CONTAINER BUILD
# ============================================================================

docker: setup
	@echo "$(COLOR_BLUE)Building in Docker container...$(COLOR_RESET)"
	@$(CONTAINER_RUNTIME) build -t $(DOCKER_IMAGE) -f Dockerfile .
	@$(CONTAINER_RUNTIME) run --rm -v $(BASE_DIR):/workspace $(DOCKER_IMAGE) \
		make -C /workspace all
	@echo "$(COLOR_GREEN)✓ Docker build complete$(COLOR_RESET)"

# ============================================================================
# CI/CD
# ============================================================================

ci: check-env setup
	@echo "$(COLOR_BLUE)Running CI pipeline...$(COLOR_RESET)"
	@echo "  [1/4] Building..."
	@$(MAKE) all --no-print-directory
	@echo "  [2/4] Testing..."
	@$(MAKE) test --no-print-directory
	@echo "  [3/4] Creating ISO..."
	@$(MAKE) iso --no-print-directory
	@echo "$(COLOR_GREEN)✓ CI pipeline passed$(COLOR_RESET)"

# Host-side unit/security tests (no build, no VM). test_wifi.py and
# test_bluetooth.py need a D-Bus session bus for their mock iwd / BlueZ, so
# they only run where dbus-run-session exists.
test:
	@{ test -x $(BUILD_DIR)/native/nexa-pack && test -x $(BUILD_DIR)/native/nexa-tools; } || $(MAKE) native --no-print-directory
	@echo "$(COLOR_BLUE)Running tests...$(COLOR_RESET)"
	@set -e; for t in $(BASE_DIR)/common/tests/test_*.py; do \
		case "$$t" in \
			*/test_wifi.py|*/test_bluetooth.py) command -v dbus-run-session >/dev/null 2>&1 || { echo "  - skipping $$(basename $$t) (no dbus-run-session)"; continue; }; \
				dbus-run-session -- python3 "$$t" >/dev/null ;; \
			*) python3 "$$t" >/dev/null ;; \
		esac; \
		echo "  ✓ $$(basename $$t)"; \
	done
	@echo "$(COLOR_GREEN)✓ All tests passed$(COLOR_RESET)"

# ============================================================================
# FULL BUILD
# ============================================================================

all: setup kernel initramfs disk
	@echo ""
	@echo "$(COLOR_GREEN)========================================$(COLOR_RESET)"
	@echo "$(COLOR_GREEN)✓ NexaOS $(PROJECT_VERSION) build complete!$(COLOR_RESET)"
	@echo "$(COLOR_GREEN)========================================$(COLOR_RESET)"
	@echo ""
	@echo "Boot with:"
	@echo "  Serial:  make boot"
	@echo "  GUI:     make boot-gui"
	@echo ""
	@ls -lh $(DIST_DIR)/{bzImage,initramfs.cpio.gz,nexa.img} 2>/dev/null || true

# ============================================================================
# CLEANUP
# ============================================================================

clean:
	@echo "$(COLOR_BLUE)Cleaning build artifacts...$(COLOR_RESET)"
	@rm -rf $(BUILD_DIR)
	@rm -f $(BUILD_LOG)
	@echo "$(COLOR_GREEN)✓ Clean complete$(COLOR_RESET)"

distclean: clean
	@echo "$(COLOR_BLUE)Deep clean (removing binaries, disk, iso)...$(COLOR_RESET)"
	@rm -rf $(DIST_DIR) $(MNT_DIR)
	@echo "$(COLOR_GREEN)✓ Distclean complete$(COLOR_RESET)"

# ============================================================================
# DOCUMENTATION
# ============================================================================

docs:
	@echo "$(COLOR_BLUE)Build System Documentation$(COLOR_RESET)"
	@echo ""
	@echo "Configuration:"
	@echo "  All build parameters are in build.conf"
	@echo ""
	@echo "Targets:"
	@make help | tail -20

.DEFAULT_GOAL := help
