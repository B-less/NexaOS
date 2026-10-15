# The system broker (`nexa-brokerd`)

Written 2026-10-14. `nexa-brokerd` (Rust, `tools/nexa-brokerd`) is the one
place where NexaOS changes the system as root. Apps, Settings, the Files
app, the assistant and udev all ask it; none of them runs as root and no
Python runs as root for any of this.

## How a request goes

1. An app sends one JSON line to `/run/nexa-broker.sock`:
   `{"action": "set_brightness", "args": {"percent": 40}}`.
2. **Who:** the kernel says who is asking (SO_PEERCRED). Only root and
   members of the `wheel` group (administrators; the desktop user is one)
   may ask at all.
3. **What:** the action must be one of the list below, and every argument is
   checked field by field (names, paths, numbers, packages, VPN files...).
4. **Password:** actions marked *password* need the asker's password,
   checked against `/etc/shadow` with the C library's `crypt()`. Three
   wrong tries lock that person out for a minute. The live USB has no
   password, so these go straight through there.
5. **Do it, confined:** a separate worker process performs the action,
   first locking itself down with Landlock to exactly the files (and, for
   software, network ports 80/443) that action needs. Requests run one at
   a time, so system changes never race each other.
6. **Log:** every request, allowed or refused, goes to
   `/var/log/nexa-broker.log` (root only), without passwords.
7. **Undo:** setting changes answer with the previous value as an `undo`
   request (CPU mode, brightness, time zone, computer name, services,
   packages, apps).

Two kinds of actions run without Landlock on purpose: **starting
services** (a Landlock domain is inherited for life, so a service started
inside one couldn't open its ports) and **mounting drives** (Landlock
forbids `mount(2)`). For those, the checks in step 3 are the guard: only
services that exist and aren't part of the system itself, only removable
USB/SD drives (re-checked against sysfs), only under `/media`.

## Actions

| Action | Arguments | Password | Notes |
|---|---|---|---|
| `reboot`, `poweroff`, `suspend` | | | |
| `set_governor` | `governor` | | performance, powersave, ondemand, ... |
| `set_brightness` | `percent` | | never fully dark (1% minimum) |
| `set_timezone` | `zone` | | must exist in /usr/share/zoneinfo |
| `set_hostname` | `name` | yes | RFC 1123; /etc/hosts kept in step |
| `bluetooth_unblock` | | | can't override a hardware switch |
| `write_ai_config` | `data` | | only the assistant's own fields |
| `start_service`, `restart_service` | `name` | | not the system's own services |
| `stop_service` | `name` | yes | |
| `enable_service`, `disable_service` | `name` | yes | at startup (default runlevel) |
| `update_index` | | | checks for updates |
| `system_update` | | yes | `apk upgrade` |
| `install_packages`, `remove_packages` | `names` | yes | names only (no options/paths); NexaOS's own packages can't be removed |
| `install_app`, `uninstall_app` | `path` / `id` | yes | .nexa apps, see below |
| `vpn_list`, `vpn_import`, `vpn_up`, `vpn_down`, `vpn_remove` | `name`, `config` | | WireGuard; files that run commands are refused |
| `list_drives`, `mount_drive`, `unmount_drive`, `eject_drive` | `name` | | removable drives only |
| `change_password` | `new_password` | yes | your own |
| `add_user`, `remove_user` | `name`, ... | yes | can't remove yourself or the last administrator; files are kept |

udev also runs `nexa-brokerd drive-added NAME` / `drive-removed NAME`
directly (as root) when a drive comes or goes.

## Apps (`.nexa`)

The package is read **with the asker's own file permissions** (setfsuid),
so nobody can make root read a file they couldn't, then copied to a private
folder and checked by `nexa-pack` (every file's SHA-256 and the publisher's
signature). An app only ever places files under its own names
(its program in `/usr/local/libexec/nexa-apps/<id>`, its command
`/usr/local/bin/<id>` as a link to `nexa-run`, which starts it confined to
its declared permissions; `/usr/local/lib/nexa-<id>`,
`nexa-<id>.desktop`, `nexa-<id>` icons), never over a system command or
anything the system or another app installed, and records what it placed,
so removing it is exact. A failed install removes everything it started to
place. Unsigned or unknown-publisher packages come back as "unverified"
and install only if the person agrees.

## VPN files

WireGuard files are handed to `wg-quick` as root, which runs
PreUp/PostUp/PreDown/PostDown as shell commands, so imports accept only
plain tunnel settings, and every value is checked (keys are 32-byte
base64, addresses are CIDR, MTU/ports/marks are numbers, no shell
characters anywhere). DNS servers are applied by the broker and the
previous `/etc/resolv.conf` comes back when the tunnel goes down.

## Still separate

- **OS installer and the live USB's persistence partition** (disk
  partitioning) stay with the live-only setup helpers for now; they move
  last, with the installer itself.

## Using it

```sh
nexa-brokerd call set_brightness '{"percent": 60}'
NEXA_BROKER_PASSWORD=... nexa-brokerd call set_hostname '{"name": "studio"}'
```

From Python: `nexa_common/broker.py` (`broker.run(action, args,
ask_password)` asks for the password when needed).

## Tests

`make test-native`: policy (every argument check), passwords (real SHA-512
`crypt()`), the worker against a fake system tree, VPN validation and
up/down with DNS, drives against a fake sysfs with fake mount tools, and
the real service on a test socket asked by different users (refused
non-admins, password prompts, lockout, audit log, root refusing to read a
file the asker can't). `make test`: the .nexa security tests drive the real
broker worker; the Python client's password flow against a fake broker.
