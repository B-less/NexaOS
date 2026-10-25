# NexaOS Architecture

NexaOS is Alpine Linux 3.24 (musl, OpenRC, apk) on a custom monolithic Linux
6.18 kernel, with a Wayland desktop (Hyprland, or Sway as the fallback) and a
set of Python 3 / GTK 3 apps written for it. Packages come from Alpine's
signed HTTPS repositories.

## Boot

1. **Firmware → bootloader.** The ISO is hybrid: isolinux for BIOS, a
   standalone GRUB for UEFI. GRUB waits silently for 2 s (Esc shows the
   menu). Installed disks get GRUB for both BIOS and UEFI.
2. **Kernel.** Monolithic (`CONFIG_MODULES` off): storage, USB, graphics
   (Intel i915, AMD amdgpu; simpledrm as the fallback), WiFi, Bluetooth, WireGuard and the
   netfilter/policy-routing pieces are built in. GPU firmware (i915, amdgpu)
   and `regulatory.db` are inside the initramfs, because built-in graphics
   drivers ask for firmware before the root filesystem exists.
3. **initramfs `/init`** (busybox). On the live medium (`nexa.live` on the
   kernel command line) it loop-mounts `nexa.img` from the USB stick
   read-only and puts a 1 GB RAM overlay on top, so the live session writes
   nothing to the stick and forgets everything at shutdown. On an installed
   disk it finds the root partition and mounts it. Then `switch_root`.
4. **OpenRC.** The boot animation (`bootsplash`, a hexagon that builds itself
   and writes NEXA) runs on the framebuffer while services start.
   `wifi-probe` and `bt-probe` re-offer WiFi and USB Bluetooth adapters to
   their drivers now that `/lib/firmware` is mounted. Then the firewall
   (nftables: inbound dropped except replies), iwd, bluetoothd, PipeWire.
5. **Session.** `nexa-session` logs the desktop user (uid 1000, whatever the
   installer named it) in on tty1 and runs `nexa-desktop`, which starts
   Hyprland on a real GPU (its driver is i915, xe, amdgpu, radeon or nouveau;
   this kernel has i915 and amdgpu) and Sway otherwise,
   or if Hyprland exits within 20 s. `~/.config/nexa/session` overrides it.
   On installed systems `nexa-login-lock` asks for the password first.

## Desktop

| Part | What it is |
|---|---|
| Bar | waybar, with the start button, clock, Control Centre, tray |
| `nexa-start` | Start menu: search, pinned apps, power (Super+D) |
| `nexa-control` | Control Centre: Wi-Fi, volume, brightness, Do Not Disturb (Super+N) |
| `nexa-dock` | Dock with magnification, running-app dots, Launchpad first |
| `nexa-launchpad` | Full-screen app grid with type-to-search (Super+Space) |
| `nexa-lock` | Lock screen: starts `nexa-locker` (C, ext-session-lock), swaylock as fallback; idle lock/blank set in Settings > Power |
| mako | Notifications |

The shell panels are GTK layer-shell surfaces (namespaces `nexa-*`, which
Hyprland blurs). `common/nexa_common/compositor.py` hides the
Hyprland/Sway differences (window list, focus, minimize, wallpaper, keyboard
layout).

**Themes.** `themes.py` holds five themes (Horizon, Nebula, Aurora, Eclipse,
Daybreak) and renders them into waybar, mako, foot, Sway, Hyprland and GTK
settings, plus a palette of `@nexa_*` colours that every Nexa app's
stylesheet uses. Apps watch `~/.config/nexa/theme.json` and restyle live.
"Glass windows" makes app windows partly see-through (84% solid and blurred
on Hyprland, 94% on Sway); `titlebar.py` paints that colour once per window.

## Apps

Each app is a Python package `<dir>/nexa_<name>/` staged to
`/usr/local/lib/nexa-<name>/` with a launcher in `/usr/local/bin` and a
`.desktop` file (see `scripts/stage-*.sh`). All use the shared titlebar
(minimize, maximize, close) and theme palette.

Files, Nexa Browser, Settings, Nexa Software, NexaOS Assistant (Claude),
Nexa Pulse (task manager + system monitor, Ctrl+Shift+Esc), Editor, Nexa
Notes, Music, Calculator, Nexa Viewer, Nexa PDF, Nexa Archive, Screenshot,
Clipboard, Color Picker, Characters, Network, Volume, Power, Install NexaOS
(live only). Nexa Tasks and Nexa System Monitor are still installed but
hidden; Nexa Pulse replaces them.

**Browser** (WebKitGTK 4.1): web content runs in WebKit's bubblewrap sandbox;
history and sign-in cookies are saved per user, third-party cookies are
refused; downloads go to `~/Downloads` with live speed; private windows keep
nothing; Tor windows (`nexadark`) are private and send everything, DNS
included, through a per-user Tor on 127.0.0.1:9150 started on demand.

## Privileges

The desktop user is not root. Root-level actions go through small helper
programs, each run with `doas` and each accepting one JSON request on stdin,
checking every field, and touching only what it is for:

| Helper | Does | Password on installed systems |
|---|---|---|
| `nexa-software-helper` | install/remove Alpine packages | yes |
| `nexa-install-helper` | install `.nexa` app bundles | yes |
| `nexa-ai-control-helper` | the assistant's system actions | yes |
| `nexa-settings-helper` | hostname, time zone, assistant config, VPN tunnels, Bluetooth rfkill | no |
| `nexa-power-helper` | suspend, reboot, power off | no |
| `nexa-mount-helper` | mount/unmount USB drives | no |
| `nexa-setup-helper` | partition and install to a disk | live USB only; removed by the installer |

`common/nexa_common/elevate.py` runs a helper with `doas -n` and, when doas
wants a password, asks for it in a dialog (`askpass.py`) and passes it to
doas through a pseudo-terminal. The installer rewrites `/etc/doas.conf` on
the new system accordingly (`harden_privileges`).

**VPN.** Settings > VPN imports WireGuard `.conf` files through the settings
helper, which refuses `PreUp`/`PostUp`/`PreDown`/`PostDown` (wg-quick would
run them as root) and any unknown setting, stores tunnels in
`/etc/wireguard` (root only), runs one at a time with `wg-quick`, and applies
and restores the tunnel's DNS itself.

**Bluetooth.** Settings > Bluetooth talks to BlueZ over D-Bus (allowed for
local users by BlueZ's policy) and registers a pairing agent that shows or
asks for codes; only lifting an rfkill block goes through the helper.

## Hardening

- nftables: inbound traffic dropped unless it answers something we sent
- sysctl: `kptr_restrict=2`, `dmesg_restrict`, no ICMP redirects, rp_filter,
  SYN cookies, protected symlinks/hardlinks
- sshd installed but off
- password for package installs and assistant actions; lock screen after
  every boot on installed systems
- browser content sandboxed (bubblewrap); WebRTC and DNS prefetching off

See `docs/HARDENING.md` for the plan beyond this (AppArmor, signed boot).

## Installer

`installer-os` (live USB only) asks for disk, username, password, host name,
time zone and keyboard, then (`common/nexa_common/diskinstall.py`):
GPT with an EFI partition and an ext4 root, copies the running system minus
mounts, installs the kernel and GRUB for BIOS and UEFI, renames the account
(uid stays 1000), sets the password, hardens doas, and turns on the startup
lock. Wi-Fi networks joined in the live session come along.
