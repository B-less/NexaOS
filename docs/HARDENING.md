# NexaOS hardening: where we are, and the plan

Written 2026-10-12. "Done" items are in the current build; the rest is the
plan, in the order it pays off.

## Done

- **Firewall** (nftables): inbound dropped unless it answers our own traffic;
  forwarding dropped. sshd installed but off.
- **sysctl**: `kptr_restrict=2`, `dmesg_restrict=1`, protected
  symlinks/hardlinks, rp_filter, no ICMP redirects, SYN cookies.
- **Kernel**: monolithic (no loadable modules, so nothing can be loaded into
  the kernel at runtime), KASLR (`RANDOMIZE_BASE`, `RANDOMIZE_MEMORY`),
  `STACKPROTECTOR_STRONG`, `STRICT_KERNEL_RWX`, `STRICT_DEVMEM`, seccomp,
  Landlock, no kexec, no debugfs.
- **Privileges**: no sudo-for-everything. Each root-level task is a small
  helper behind its own doas rule, taking one checked JSON request. Package
  installs and the assistant's system actions need the password on installed
  systems; the disk-installer helper is removed after installing.
- **Startup lock** on installed systems; idle lock.
- **Browser**: WebKit content in a bubblewrap sandbox; third-party cookies
  refused; WebRTC and DNS prefetching off; private and Tor windows.
- **VPN files** that would run commands as root are refused.

## Step 1: kernel self-protection (done in 3.2.0)

The Kernel Self Protection Project's baseline options, now on.
They cost little (a few % on allocation-heavy work at most) and break
nothing a desktop uses:

| Option | Why |
|---|---|
| `HARDENED_USERCOPY` | bounds-checks copies between kernel and user memory |
| `FORTIFY_SOURCE` | compile-time + runtime checks on kernel string/memory functions |
| `SLAB_FREELIST_RANDOM`, `SLAB_FREELIST_HARDENED`, `SHUFFLE_PAGE_ALLOCATOR` | make heap exploits unreliable |
| `INIT_ON_ALLOC_DEFAULT_ON` | no stale data leaks from reused memory |
| `LIST_HARDENED` | catches corrupted kernel lists |
| `SECURITY_YAMA` (+ `kernel.yama.ptrace_scope=1`) | a process can only debug its own children, so malware running as you can't read the browser's or the keyring's memory |
| `IO_STRICT_DEVMEM` | `/dev/mem` can't touch device memory a driver owns |

## Step 2: confine the root helpers with Landlock (next)

Landlock is already in the kernel and needs no profiles, policy language or
daemon: a program restricts *itself*. Each `nexa-*-helper` would, right after
reading its request, allow only the paths it needs (for example the settings
helper: `/etc/hostname`, `/etc/localtime`, `/etc/wireguard`,
`/etc/resolv.conf`, `/sys/class/rfkill`) and lose access to everything else.
A bug in a helper then can't be turned into "write anywhere as root".
Python has no Landlock module, but it is three syscalls through `ctypes`;
one shared `nexa_common/landlock.py` covers every helper. Test: a helper
that tries to open `/etc/shadow` after restricting itself must fail.

## Step 3: AppArmor for the apps that face the internet

AppArmor confines programs by path. It suits NexaOS better than SELinux
(Alpine ships `apparmor`, `apparmor-utils`; profiles are readable text).
Needs `SECURITY_APPARMOR=y` and `apparmor` added to `CONFIG_LSM`.

Catch: every Nexa app is `python3 -m ...`, so a profile on the launcher
script confines nothing and a profile on `python3` confines everything.
Plan: give the browser, Tor and the assistant their own small launcher
binaries (or `aa-exec -p <profile>` in their wrapper scripts) so each runs
under a named profile: the browser may read/write `~/Downloads` and its own
data directory, not `~/.ssh` or other apps' data. Start in complain mode,
collect denials on real use (HP test), then enforce.

## Step 4: verified boot

**Since 2026-10-30 the live USB boots with Secure Boot on** (docs/SECURE_BOOT.md):
Debian's Microsoft-signed shim, then GRUB and the kernel signed with the
NexaOS key, which each PC learns once in MokManager. Installed systems still
need it off until the installer writes the same chain. The options weighed:

1. **Microsoft-signed shim** (what Ubuntu/Fedora do): needs a shim signed by
   Microsoft for NexaOS's own key. That is a formal review process for an
   organisation; not realistic now.
2. **Own keys** (recommended): build a Unified Kernel Image (kernel +
   initramfs + command line in one EFI file), sign it and GRUB with a NexaOS
   key using `sbctl`, and offer an "Enroll NexaOS keys" step in the installer
   for machines in Secure Boot *setup mode* (HP: BIOS > Security > Secure Boot
   keys > Clear). With Secure Boot on, nothing unsigned boots.
   Add `SECURITY_LOCKDOWN_LSM` (integrity mode when booted securely) so root
   can't patch the running kernel either. Note that integrity lockdown turns
   off hibernation.
3. **TPM** (`TCG_TPM`, `TCG_TIS`, `TCG_CRB`): the drivers are in the kernel
   (2026-10-19). TPM 1.2 and 2.0 use the same drivers: `tpm_tis` for a
   separate chip (Infineon, Nuvoton, STM...), `tpm_crb` for one built into
   the processor's firmware (Intel PTT, AMD fTPM). `TCG_TPM2_HMAC` (bus
   encryption) is off: it slows every boot and nothing uses it yet.
   `nexa-doctor security` reports the chip and Secure Boot (Settings > About,
   the assistant). Still to do: sealing a disk-encryption key to the
   measured boot chain.

## Step 5: disk encryption

The installer currently writes a plain ext4 root. LUKS2 (`dm-crypt`,
`cryptsetup` in the initramfs, a passphrase prompt in the boot animation)
protects a lost or stolen laptop, which for most people is the biggest real
risk. Kernel needs `DM_CRYPT` and the crypto API options; the initramfs needs
`cryptsetup`; the installer needs a "Encrypt this disk" switch.

## Not planned

- **SELinux**: more powerful than AppArmor, but its policy is far more work
  to write and maintain for a small project.
- **"More secure than Kali"**: Kali is a toolbox for security testing, not a
  hardened OS; it runs as a normal Debian with extra tools. The comparison
  that matters is with hardened desktops (Qubes, Fedora Silverblue,
  GrapheneOS's ideas); steps 1–5 move NexaOS toward them.
