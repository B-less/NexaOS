# Windows Boot Repair (2026-11-01)

For a PC where Windows won't start (the firmware's boot menu blinks and
returns, or Windows Boot Manager isn't listed). Start the NexaOS USB on it,
then open **Windows Boot Repair** from the app menu. It runs
`nexa-bootfix` (Rust, `tools/nexa-bootfix`) as root in a terminal.

## What you do

1. Plug in a **second USB drive** (FAT32, exFAT or NTFS). The report, a
   backup of the boot partition and a screenshot are saved there, in a
   folder `NexaOS-bootfix-<date>-<time>`.
2. Read what it found. If it offers fixes, type `yes`.
3. Restart and choose Windows Boot Manager. If something is left, the
   report says what to do, and you can show it (or the screenshot) to
   whoever is helping.

On the live USB it needs no password; an installed NexaOS asks for yours.

## What it looks at (read-only)

| Part | How |
|---|---|
| The internal disk's partitions | GPT read from the disk (`disk.rs`); MBR drives from sysfs |
| EFI boot partition | mounted read-only: `\EFI` folders, `\EFI\Microsoft\Boot\bootmgfw.efi`, `BCD` (a registry hive, "regf"), the fallback `\EFI\Boot\bootx64.efi`, free space |
| Windows Boot Manager | PE/COFF headers, every section inside the file, the Authenticode signature's certificate names: which Microsoft key signed it (Windows PCA 2011, Windows UEFI CA 2023) (`pe.rs`) |
| The Windows drive | BitLocker or NTFS; NTFS mounted read-only: `\Windows\System32\winload.efi` and Windows' own copy of the boot manager, `\Windows\Boot\EFI\bootmgfw.efi` |
| Firmware boot entries | Boot####, BootOrder, BootCurrent from efivarfs: description, on/off, which partition (GPT GUID) and file (`efi.rs`) |
| Secure Boot keys | nexa-doctor's `secureboot.rs`: setup mode, PK/KEK/db/dbx, Windows and third-party CAs |

A boot manager signed with a key the PC's db doesn't have, with Secure Boot
on, is reported as the reason Windows is refused.

## Deleted partitions: getting your files back

If the internal disk's partition list is empty (the partitions were deleted
in the Windows installer's "Where do you want to install Windows?" screen,
or with `diskpart clean`), Windows can't start and the Windows installer's
Repair has nothing to repair. The files are often still there: deleting a
partition only empties the list.

- It looks for the partitions on every 1 MiB boundary (where Windows starts
  them) by their boot sectors: NTFS, BitLocker, FAT32, exFAT (`lost.rs`).
  It also reads the backup partition list in the disk's last sector.
- Each one found is mounted read-only through a read-only loop device over
  the disk (opened read-only), to tell Windows (C:), the recovery and EFI
  partitions and data drives apart, and to list the users.
- In the step-by-step run it lists each user's folders (Desktop, Documents,
  Pictures, ...) with their sizes; you type the numbers, or `all`. They're
  copied to the USB drive under `Files/<user>/<folder>`. AppData, the
  recycle bin and Windows' own folders are left out. On a FAT32 drive,
  files of 4 GB or more are skipped and listed in `files-copied.txt`.
- It says plainly when the disk reads back blank (SSDs erase deleted space
  by themselves): then the files can't be brought back from it.
- BitLocker partitions are found and reported, but not opened yet.

Nothing is written to that disk. Don't install anything on it until the
files are copied and checked on another computer.

## What it fixes, and only after a backup

First the whole EFI boot partition and every Boot#### variable are copied
to the second USB drive. If the copy fails, nothing is changed.

- **Damaged or missing Windows Boot Manager**: replaced with Windows' own
  copy from the Windows drive (same version, same signature), if that copy
  is whole.
- **Missing or damaged fallback loader** (`\EFI\Boot\bootx64.efi`, started
  by firmware entries named after the disk, like "NVMe0 TOSHIBA"): a copy of
  Windows Boot Manager. A working loader of another system is never
  replaced.
- **No Windows Boot Manager entry, or one pointing at a partition that isn't
  there or switched off**: a new entry pointing at the boot partition,
  first in the boot order.

## What it never does

It never writes Secure Boot keys, never edits BCD, never formats or
resizes anything. For those it explains the step:

- BCD missing or damaged, `\EFI\Microsoft\Boot` gone, or Windows' drive
  locked by BitLocker: `bcdboot C:\Windows /s S: /f UEFI` from the Windows
  installer's command prompt (Shift+F10), with the diskpart steps.
- Keys cleared: the BIOS's Restore Factory Keys (docs/SECURE_BOOT.md).

## Checked

- Unit tests: GPT, filesystems, boot entries (parse, build, order), PE
  (whole, cut short, unsigned, which key), the findings for each fault.
- `make test-bootfix`: the live ISO in QEMU with a fake Windows disk as an
  internal NVMe drive and a FAT32 USB stick. The boot manager cut short, the
  fallback removed, no Windows entry: all three found, fixed, and the report,
  backup and screenshot saved on the stick.
- The same run with a second NVMe disk holding Windows and a user's
  folders, whose partition list (primary and backup) is erased in the VM:
  both partitions found, Documents and Pictures copied (AppData not), and
  the disk's SHA-256 the same before and after.
- Not checked yet: a real Windows install, a real BitLocker drive.
