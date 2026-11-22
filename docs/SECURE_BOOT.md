# Secure Boot (2026-10-30)

The live USB starts with **Secure Boot on**, on PCs that trust Microsoft's
third-party UEFI CA (nearly all of them; some "Secured-core" models need it
switched on in the firmware, which doesn't turn Secure Boot off).

## How it starts

| Step | What runs | Signed by | Why it's trusted |
|---|---|---|---|
| 1 | `EFI/BOOT/BOOTX64.EFI`: **shim** 16.1, from Debian's `shim-signed` | Microsoft (UEFI CA 2011 **and** 2023) | The firmware trusts Microsoft's CAs out of the box; the 2023 one covers new PCs that no longer carry the 2011 one |
| 2 | `EFI/BOOT/grubx64.efi`: NexaOS's GRUB 2.14, every module built in, SBAT `grub,5` | the **NexaOS key** | shim trusts it once the PC has learned the NexaOS key (below) |
| 3 | `boot/vmlinuz`: the kernel (and the initramfs and `grub.cfg`) | the NexaOS key (shim/PE) **and** the NexaOS GRUB key (GPG) | GRUB verifies the GPG signature of every file it loads (`pgp`, `check_signatures=enforce`), since Alpine's GRUB has no `shim_lock` — see "How GRUB checks the kernel" |
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

If the firmware refuses the stick before MokManager, it doesn't trust
Microsoft's third-party UEFI CA: turn that on (ThinkPad: **Allow Microsoft
3rd Party UEFI CA**), or start with Secure Boot off.

## Never clear the PC's keys (2026-10-31)

On a ThinkPad, "Clear All Secure Boot Keys" was used when the stick didn't
start. That empties PK, KEK and db: the firmware drops to setup mode and no
longer has Microsoft's Windows certificate, and Windows Boot Manager then
won't start (the F12 menu blinks and returns). NexaOS never needs keys
cleared, and never writes firmware keys itself.

The fix is in the firmware: Security > Secure Boot > **Restore Factory
Keys** (other names: Reset to Default Keys, Install Default Keys). Have the
BitLocker recovery key ready (aka.ms/myrecoverykey), since the measured
boot changes. A PC's factory keys can predate Microsoft's **Windows UEFI CA
2023**; if Windows' boot manager was already updated to the 2023 signature
(KB5025885), start Windows once with Secure Boot off, install the updates
(they add the 2023 certificate to db), then turn Secure Boot back on.

**The check at boot:** `nexa-doctor` (Rust, `tools/nexa-doctor/src/secureboot.rs`)
reads SecureBoot, SetupMode, PK, KEK, db, dbx and the Boot#### entries from
efivarfs, read-only. It names each certificate and looks for Microsoft's
Windows CAs (Windows Production PCA 2011, Windows UEFI CA 2023), the
third-party CAs (UEFI CA 2011/2023) and a Windows Boot Manager entry. When
the keys were cleared, or Windows is on the PC but db doesn't trust its
certificate, it shows a notification that stays until closed (once per
boot, at session start), a Doctor finding, and a warning in Settings >
System. `nexa-doctor firmware-keys --text` prints the whole picture.

## How GRUB checks the kernel (2026-11-22)

shim verifies GRUB, but GRUB has to verify what *it* loads — the kernel, the
initramfs and `grub.cfg` — or a tampered kernel would boot with Secure Boot
still "on". The usual way is GRUB's `shim_lock` module, but **Alpine's GRUB
2.14 doesn't ship one**. So NexaOS uses GRUB's own `pgp` verifier instead:

- A separate GPG key, the **NexaOS GRUB key** (`scripts/grub-verify-key.sh`,
  RSA-3072, kept next to the Secure Boot key, never pushed), signs
  `boot/vmlinuz`, every `initramfs-*` and `boot/grub/grub.cfg` at build time
  (detached `.sig` files beside each).
- Its public half is baked into the GRUB image (`grub-mkimage --pubkey`), and
  the built-in config runs `set check_signatures=enforce` before loading
  `grub.cfg`. From then on GRUB **refuses any file whose signature doesn't
  verify** — so a swapped or modified kernel is rejected and nothing boots.
- This holds with Secure Boot on *or* off (the `pgp` check is independent),
  and leaves `grub.cfg` tamper-evident too. The kernel's shim/PE signature
  (`sbsign`) stays for the day a `shim_lock` GRUB is available.

`scripts/test-grub-verify.sh` proves it in QEMU/OVMF: a correctly signed
kernel boots; a kernel with one byte flipped is refused (`bad signature`) and
never starts.

## The keys

- **Secure Boot signing key** — `sh scripts/secureboot-key.sh` (RSA-2048,
  `~/.config/nexaos-keys/secureboot/`, next to the release key). Signs shim's
  GRUB and the kernel's PE signature; its public half `nexa-sb.cer` is what a
  PC enrolls in MokManager. **Never in the repo, never pushed.**
  Fingerprint (SHA-256) of the key made 2026-10-30:
  `5D:50:B5:1A:DF:7F:08:14:76:48:C3:12:F4:C2:4D:5F:9C:18:FD:C1:5F:11:51:15:1B:9E:7F:F8:71:4E:30:3E`
- **GRUB verification key** — `sh scripts/grub-verify-key.sh` (RSA-3072 GPG,
  same folder). Signs the files GRUB loads, as above. Also **never pushed**.
- The build mounts the folder read-only into the ISO step. Keep both keys'
  secret halves backed up offline: a new Secure Boot key means every PC
  re-enrolls; a new GRUB key just means the next ISO is re-signed.

## Building

`make refresh` / `make iso` / `make preload-map` fetch the shim
(`scripts/fetch-shim.sh`: Debian's apt checks the package signatures, then
pinned checksums) and, when the keys are there, sign GRUB and the kernel for
Secure Boot **and** GPG-sign the kernel/initramfs/`grub.cfg` for GRUB's own
check (enforcement is baked into the GRUB image). Without the Secure Boot key
or the shim the ISO is built for Secure Boot off; without the GRUB key GRUB
doesn't enforce and the files aren't signed — so make both keys once
(`secureboot-key.sh`, `grub-verify-key.sh`) before `make iso`.

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

The key check: unit tests with synthetic certificates and the cleared,
factory and own-keys cases; and the real Microsoft certificates from OVMF's
`OVMF_VARS_4M.ms.fd` (all eight names read right, 2011 and 2023).

## Not covered yet

- **Full ISO end-to-end with the real GRUB key.** The mechanism is proven in
  OVMF (`scripts/test-grub-verify.sh`) and Alpine's `grub-mkimage` accepts the
  `pgp` + `--pubkey` recipe, but a complete `make iso` with the real
  `grub-verify.key` — then booting that ISO and confirming a tampered kernel is
  refused — should be run once on the build machine before a release.
- **Installed systems**: the installer writes an unsigned GRUB (grub-install),
  so an installed NexaOS still needs Secure Boot off. It needs the same shim,
  signed GRUB and signed kernel on the disk's EFI partition (installer step
  I7, docs/INSTALLER_PLAN.md).
- **No enrollment step at all** needs NexaOS's own Microsoft-signed shim with
  the NexaOS key built in: the shim-review process
  (https://github.com/rhboot/shim-review), for an established project.
- **Kernel lockdown** (`SECURITY_LOCKDOWN_LSM`): not on yet (HARDENING.md
  step 4); with it, root couldn't change the running kernel either.
