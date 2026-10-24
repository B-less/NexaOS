# The NexaOS installer: research and plan (2026-10-24)

What NexaOS has today, what macOS and other systems do, and a phased plan.
Per AGENTS.md the installer and bootloader come **last**: this is the plan
for when that time comes, not work to start now.

## 1. What NexaOS has today

- `installer-os/nexa_setup` (GTK, Python) with `common/nexa_common/diskinstall.py`
  run as root by `nexa-setup-helper`: GPT layout (BIOS-boot + 320 MiB ESP +
  root), `mkfs`, a tar clone of the running live system, GRUB for BIOS
  (`i386-pc`) and UEFI (`x86_64-efi --removable`), username and password,
  hostname. QEMU install tests pass for BIOS and UEFI (TODO §3).
- Gaps against today's rules: the disk work is Python running as root
  through its own helper. AGENTS.md now says OS work is Rust/C and every
  root action goes through the broker (nexa-brokerd).
- Missing: disk encryption, keeping your files on reinstall, a recovery
  mode, A/B system updates, Secure Boot, installing next to Windows.

## 2. How macOS does it (the ideas worth taking, not the look)

- **Very few questions.** Reinstalling from Recovery is: choose Reinstall,
  pick a disk, wait, and the Mac restarts into setup. If the disk isn't
  listed, you erase it first in the disk tool; Apple notes you usually
  don't need to ([Apple Community](https://discussions.apple.com/thread/252184576)).
- **Accounts come after the install.** After installing, the Mac restarts
  into the Setup Assistant (language, network, account, privacy choices);
  quitting it leaves a fresh machine ready to hand to someone else
  ([Apple Community](https://discussions.apple.com/thread/254752246)).
- **A recovery system on the disk**, with the reinstall, the disk tool and
  a way to install a different version from there
  ([Eclectic Light](https://eclecticlight.co/2021/06/28/making-a-good-recovery-installing-a-different-macos-in-recovery/)).
- **Bootable installer media** made by a tool that asks before erasing the
  target ([Eclectic Light](https://eclecticlight.co/2021/06/28/making-a-good-recovery-installing-a-different-macos-in-recovery/),
  [Chris Warrick](https://chriswarrick.com/blog/2020/06/03/reinstalling-macos-what-to-try-when-all-else-fails/)).
- **A sealed, read-only system with your data on its own volume**, so a
  reinstall or an update replaces the system and keeps your files.

AGENTS.md: NexaOS must not copy macOS's layout, icons or wording. We take
the *flow* (few questions, setup after the restart, recovery, sealed
system), drawn in Hexa.

## 3. How Linux installers are built

- **An engine library plus thin front ends.** System76 wrote distinst in
  Rust for the disk work (partitions, encryption), with elementary's
  installer as one front end and a CLI as a test bed; user setup was left
  to a separate first-boot setup. They chose not to reuse Ubiquity or
  Calamares as too dated or tied to another stack
  ([pop-os/distinst](https://github.com/pop-os/distinst),
  [elementary blog](https://blog.elementary.io/meet-the-upcoming-installer/),
  [Phoronix](https://www.phoronix.com/news/System76-Own-OS-Installer)).
  This matches NexaOS's pattern exactly (Rust engine, thin window).
- **Image-based installs and A/B updates.** systemd-repart lays out
  partitions (ESP, two system slots A and B, a data partition) from a
  declarative description; systemd-sysupdate writes a signed new system
  into the unused slot and switches at the next boot, going back if it
  fails; the kernel, initramfs and command line can be one signed Unified
  Kernel Image on the ESP
  ([x86.lol](https://x86.lol/generic/2024/08/28/systemd-sysupdate.html),
  [Igalia](https://blogs.igalia.com/mabente/how-to-integrate-systemd-sysupdate-with-your-yocto-based-image/)).
  NexaOS uses OpenRC, not systemd, so we take the *layout and method*,
  implemented in our own Rust; we don't adopt systemd.
- **Secure Boot.** Distributions boot through shim, signed by Microsoft,
  which then checks GRUB and the kernel against the distribution's key.
  Keys enrolled at install time with MokManager (`mokutil --import`) are
  limited, and since August 2024 SBAT revocations break old shims
  ([Ubuntu docs](https://documentation.ubuntu.com/security/docs/security-features/platform-protections/secure-boot/),
  [Gentoo guide](https://setphaserstostun.org/posts/secure-boot-on-gentoo-with-shim-grub/index.rst),
  [linux-surface](https://github.com/linux-surface/linux-surface/wiki/Secure-Boot),
  [kali-with-secure-boot](https://github.com/TheCjw/kali-with-secure-boot)).
  A Microsoft-signed shim for NexaOS needs the shim-review process; until
  then the realistic route is HARDENING.md step 4: a signed UKI and our own
  keys enrolled from firmware setup mode (`sbctl`).

## 4. The plan

| Phase | What | How (Rust/C, through the broker) | Done when |
|---|---|---|---|
| **I1 Engine** | `nexa-installd` (Rust): find disks (reusing nexa-brokerd's drive code, never the stick it runs from), plan a layout and show it before touching anything, write it, copy the system, install the bootloader, verify | Broker action `install_os` (password, logged). Layout: ESP 512 MiB, system A, system B (empty), data (`/home`, settings). The system is written as an image (nexa.img, block copy plus SHA-256 check) instead of a tar clone, which is faster and the same bytes every time. GRUB for BIOS and UEFI first | Rust tests on loop-device disks; the current QEMU BIOS/UEFI install tests pass on the new engine; the Python root helper is removed |
| **I2 Installer window** | Hexa flow: Welcome (language) → choose the disk (shows what's on it, size, "everything on it will be erased", typed confirmation) → optional encryption → install (progress, time left, the point after which it can't be stopped is stated) → restart | Thin GTK over the engine's JSON progress | Headless screenshots of each step; QEMU install end to end |
| **I3 First-boot setup** | After the restart (like macOS's Setup Assistant, in our own words): language and keyboard, Wi-Fi, time zone, your account and password, privacy (Patterns off unless turned on, no telemetry), theme, then the desktop | A first-boot session that runs once, as root only through the broker; the account step moves out of the installer | QEMU: install → restart → setup → desktop as the new user |
| **I4 Recovery** | A "NexaOS Recovery" boot entry: reinstall the system keeping your data, a disk tool, roll back an update, open a terminal | The live image kept on the ESP or system B; reinstall rewrites only the system partition | QEMU: break the system, recover, files intact |
| **I5 Encryption** | LUKS2 on the data partition (or everything); passphrase at boot in the boot animation; later unlock with the TPM (the drivers are in the kernel now) | cryptsetup in the initramfs; kernel `DM_CRYPT` | QEMU install with encryption; wrong passphrase fails safely |
| **I6 A/B updates** | Signed system images: an update goes into the other slot, the next boot switches, a failed boot switches back (TODO "Safe updates with rollback", "Signed updates") | A Rust updater; images signed with the release key like .nexa packages; published like the Store's catalogue | QEMU: update, reboot, roll back |
| **I7 Secure Boot** | Signed UKI, own keys enrolled from firmware setup mode (HARDENING.md step 4); a Microsoft-signed shim only if NexaOS goes through shim-review | sbctl-style signing at build time; key stays on the build machine | HP test with Secure Boot on |
| **I8 Next to Windows** | Shrink the Windows partition (NTFS resize) and add NexaOS beside it, with a boot menu | Only after I1–I4 are proven; most risky to data | Tested on a spare disk first |

Everything that erases a disk is shown first as a plan ("this disk, these
partitions"), needs a typed confirmation, refuses the disk the system runs
from, and is logged by the broker.
