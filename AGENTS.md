# NexaOS: instructions for AI coding assistants

NexaOS is an Alpine Linux 3.24 based OS with a custom monolithic kernel, a
Sway/Hyprland desktop, and its own design language (Hexa). Build and test
steps: `BUILDING.md`. How it fits together: `ARCHITECTURE.md`.

## Rules that apply to all work here

- **New OS work is written in Rust or C.** Python is only a thin user
  interface over Rust/C code; it never runs as root and never does the
  real work.
- **Root:** nothing runs as root except through the system broker
  (`nexa-brokerd`, `docs/SYSTEM_BROKER.md`). Don't add doas rules or new
  root helpers; add a checked broker action instead.
- **Apps** are `.nexa` packages: follow `docs/WRITING_NEXA_APPS.md`
  exactly (language, permissions, Hexa design, testing).
- **Secrets:** never commit, print or package `*.key` files. The release
  key lives only in `~/.config/nexaos-keys/` on the build machine.
- **Evidence:** don't say something works until you have built it and run
  it, and looked at the output or screenshot. Report failures plainly.
- **Look:** NexaOS has its own identity. Don't copy macOS, Windows or
  GNOME layouts, icons or wording.
- **Order:** the OS installer and bootloader work comes last.

## Where things are

- `tools/`: Rust workspace (nexa-sys, nexa-pack, nexa-mindd, nexa-tools, nexa-brokerd)
- `apps/`: .nexa apps (reference: `apps/video-player`)
- `common/nexa_common/`: shared Python UI code (themes, design tokens, clients)
- `docs/`: PACKAGES, WRITING_NEXA_APPS, SYSTEM_BROKER, NEXA_MIND, HARDENING, ROADMAP
- Tests: `make test` (Python), `make test-native` (Rust), `scripts/test-live-boot.py` (ISO in QEMU)
