# Building NexaOS

NexaOS is Alpine Linux 3.24 with a custom 6.18 kernel, a Sway/Hyprland
desktop and its own Python/GTK apps. The build runs on a Linux host (WSL2
works) and does the Alpine parts inside throwaway `alpine` Docker containers,
so the host needs no Alpine tools.

## Requirements

- Linux or WSL2 with Docker, `make`, `gcc`, `bc`, `flex`, `bison`,
  `libelf-dev`, `libssl-dev` (for the kernel), `python3`
- QEMU with KVM and OVMF (`/usr/share/qemu/OVMF.fd`) for the test scripts
- About 15 GB free disk space

## First build

```bash
make kernel   # once, and after changing kernel.config.6.18.33 (~20 min, ~8 min incremental)
make iso      # rootfs + apps + initramfs + disk image + hybrid ISO
```

The result is `dist/NexaOS-<version>.iso`, a hybrid image: BIOS (isolinux)
and UEFI (GRUB) from a USB stick or DVD. Write it with Rufus (choose **DD
image** mode if Rufus asks), `dd`, or balenaEtcher. Secure Boot must be off.

The kernel source lives in `kernel/linux-6.18.33`; its config is
`kernel.config.6.18.33` (copy it to `.config` there). The kernel is
monolithic (no modules): every driver, including WiFi, Bluetooth, WireGuard
and graphics, is built in.

## Everyday rebuilds

```bash
make preload-map  # after an ISO build: boot it once, record what the boot reads, repack the ISO with that readahead map
make refresh  # ~7 min, ~3 MB download: re-provision packages/apps/configs into the existing rootfs,
              # then initramfs, disk image and ISO
make test     # host-side unit tests (no VM); about 15 suites
```

### Native tools (Rust)

`tools/` is one Cargo workspace: `nexa-sys` (shared Landlock sandbox code),
`nexa-pack` (.nexa packages), `nexa-mindd` (on-device AI service) and
`nexa-tools` (the assistant's file actions). They build as static musl
binaries into `build/native/`, which provisioning copies to `/usr/local/bin`.

```bash
make native        # release build of every tool
make test-native   # cargo tests + nexa-pack and nexa-mindd end-to-end tests
make test-files-app  # the Files window (selection, mouse, keys, Trash, undo, grid, thumbnails) + nexa-thumb in a headless Sway
make test-desktop-card  # Nexa Patterns' desktop card and Nexus suggestions in a headless Sway
```

Rebuilds are incremental: a builder image with Rust preinstalled
(`nexa-rust:3.24`, made once), a shared `tools/target/` folder, the crate
registry cached in `/var/tmp/nexa-cargo`, and thin LTO, so only the crates
you changed recompile.

### What a rebuild downloads

About **3 MB**: Alpine's fresh package list, so security updates reach the
image (plus any updated packages). The build tools live in two images made
once: `nexa-build` (bash, disk-image and ISO tools; ~29 MB with
`nexa-rust-apps`) and `nexa-rust-apps` (the Rust builder plus app headers).
To pick up newer build tools: `docker rmi nexa-build:3.24 nexa-rust-apps:3.24`.

Change `build.conf` to add Alpine packages (`ALPINE_PACKAGES_*`) or bump the
version. `make refresh` needs a rootfs from one earlier `make iso`.

`rootfs/` and the images are owned by root (they are built in containers), so
the `setup` step of `make kernel` cannot write there. If `make kernel` fails at
`setup`, build the kernel directly:

```bash
cd kernel/linux-6.18.33 && make -j"$(nproc)" bzImage && cp arch/x86/boot/bzImage ../../dist/bzImage
```

## Testing

| Command | What it checks |
|---|---|
| `make test` | Unit tests: installer, privilege helpers, Wi-Fi/Bluetooth against mock D-Bus services, VPN file checks, browser history/downloads/Tor helpers, themes, Nexa Pulse, the assistant (agent loop, local back-end, file tools and their cards) |
| `make test-native` | Rust workspace tests (sandbox, packages, Nexa Mind, file actions with undo/trash/Landlock), signed-package round trips, and Nexa Mind answering with the real model |
| `make test-desktop-card` | The desktop "For you" card (bottom layer, never takes the keyboard, a button per suggestion with its reason, Don't suggest, Move, Hide for today, off = no card) and Nexus' Suggested row, against the real `nexa-patternd` holding a synthetic month; screenshots in dist/card-test/ |
| `make test-files-app` | The Files window driven like a person would (clicks, right-click menus, Ctrl+A/C/X/V/Z, F2, Delete, Backspace, mouse back button, Ctrl+scroll), copying, moving, the Trash and undo through the real `nexa-fileops` engine, plus a screenshot (dist/files-test.png) |
| `python3 scripts/test-live-boot.py` | Boots the ISO (UEFI, as a USB stick) in QEMU, screenshots the desktop, logs in on serial and reports Alpine version, firewall, kernel hardening, SSH off, firmware errors, running desktop parts, Bluetooth service, WireGuard |
| `python3 scripts/test-install-cycle.py [--uefi]` | Boots the live ISO, installs to a blank virtual disk with a custom username and password, boots the installed disk alone and logs in |
| `sh scripts/test-app-headless.sh APP_DIR MODULE out.png` | Runs one app in a headless Sway container and saves a screenshot (see the script for `SETUP`, `WALLPAPER`, `APP_ARGS`) |

A VM has no real GPU, so QEMU always exercises the Sway session; Hyprland is
validated with `Hyprland --verify-config` and needs real hardware.

## Layout

```
Makefile, build.conf      build entry points and settings
kernel.config.6.18.33     kernel configuration
scripts/                  provisioning, image/ISO builders, staging (stage-*.sh), tests
common/nexa_common/       shared Python: themes, titlebar, compositor, Wi-Fi, Bluetooth,
                          privilege elevation, installer, VPN file checks
common/tests/             unit tests (make test)
tools/                    Rust workspace: nexa-sys, nexa-pack, nexa-mindd, nexa-tools
shell/                    bar panels: start menu, Control Centre, Dock, Launchpad
<app>/nexa_<app>/         one directory per app (browser, settings-app, pulse, ...)
assets/                   icons, wallpapers
legacy/                   early one-off scripts, kept for history (not used)
dist/                     build output (git-ignored)
```

See ARCHITECTURE.md for how the running system fits together.
