# Secure Boot (2026-10-30)

The live USB starts with **Secure Boot on**, on PCs that trust Microsoft's
third-party UEFI CA (nearly all of them; some "Secured-core" models need it
switched on in the firmware, which doesn't turn Secure Boot off).

## How it starts

| Step | What runs | Signed by | Why it's trusted |
|---|---|---|---|
| 1 | `EFI/BOOT/BOOTX64.EFI`: **shim** 16.1, from Debian's `shim-signed` | Microsoft (UEFI CA 2011 **and** 2023) | The firmware trusts Microsoft's CAs out of the box; the 2023 one covers new PCs that no longer carry the 2011 one |
| 2 | `EFI/BOOT/grubx64.efi`: NexaOS's GRUB 2.14, every module built in, SBAT `grub,5` | the **NexaOS key** | shim trusts it once the PC has learned the NexaOS key (below) |
| 3 | `boot/vmlinuz`: the kernel | the NexaOS key | GRUB has shim check it |
| (once) | `EFI/BOOT/mmx64.efi`: MokManager | Debian | shim trusts Debian's helpers |

Microsoft signs only shim: a small, publicly reviewed loader. Nothing of
NexaOS goes to Microsoft; the check happens offline, in the PC's firmware.

## The first time on a PC

shim doesn't know the NexaOS key yet, so it shows MokManager (a blue
screen) instead of NexaOS:

1. Press a key, choose **Enroll key from disk**.
2. Choose the USB stick, then **ENROLL-THIS-KEY-NexaOS.cer** (at the top
   of the stick, or in `EFI/BOOT`).
3. **Continue**, **Yes**, **Reboot**.

From then on that PC starts NexaOS with Secure Boot on. The same steps are
in `SECURE-BOOT-README.txt` on the stick.

## The key

- Made once on the build machine: `sh scripts/secureboot-key.sh` (RSA-2048,
  `~/.config/nexaos-keys/secureboot/`, next to the release key). **Never in
  the repo, never pushed.** The build mounts it read-only into the ISO step.
- Keep a backup of `nexa-sb.key` somewhere safe and offline: a new key means
  every PC has to enroll again.
- The public half, `nexa-sb.cer`, is what goes on the stick.
- Fingerprint (SHA-256) of the key made 2026-10-30:
  `5D:50:B5:1A:DF:7F:08:14:76:48:C3:12:F4:C2:4D:5F:9C:18:FD:C1:5F:11:51:15:1B:9E:7F:F8:71:4E:30:3E`

## Building

`make refresh` / `make iso` / `make preload-map` fetch the shim
(`scripts/fetch-shim.sh`: Debian's apt checks the package signatures, then
pinned checksums) and sign GRUB and the kernel when the key is there. Without
the key or the shim the ISO is built as before, for Secure Boot off.

## Checked

QEMU, OVMF with Secure Boot and Microsoft's keys (q35, SMM), the ISO as a
USB stick:

- NexaOS key in the firmware's db: boots to the desktop; the kernel reports
  "Secure boot enabled".
- Key not enrolled (a new PC): shim refuses GRUB ("Security Violation") and
  opens MokManager.
- Secure Boot off: boots as before.

Still to check on real hardware: the HP with Secure Boot on and the key
enrolled through MokManager.

## Not covered yet

- **Installed systems**: the installer writes an unsigned GRUB (grub-install),
  so an installed NexaOS still needs Secure Boot off. It needs the same shim,
  signed GRUB and signed kernel on the disk's EFI partition (installer step
  I7, docs/INSTALLER_PLAN.md).
- **No enrollment step at all** needs NexaOS's own Microsoft-signed shim with
  the NexaOS key built in: the shim-review process
  (https://github.com/rhboot/shim-review), for an established project.
- **Kernel lockdown** (`SECURITY_LOCKDOWN_LSM`): not on yet (HARDENING.md
  step 4); with it, root couldn't change the running kernel either.
