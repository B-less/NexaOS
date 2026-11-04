# Writing NexaOS apps (`.nexa`): the rules

For people and for AI assistants. Follow every **must**; follow every
**should** unless you can say why not. The reference app that follows all
of this is `apps/video-player` (C, GTK 3, GStreamer). Read it before you
start. Also read `docs/PACKAGES.md` (the package format) and
`docs/SYSTEM_BROKER.md` (how apps change the system).

## 1. Language

- **Must** be written in **C** (C11/gnu11, GTK 3) or **Rust** (2021, static
  musl). Python, shell scripts, JavaScript and Electron are not accepted
  for app logic.
- C **must** build with the hardened flags in the template (do not remove
  any):
  `-Wall -Wextra -Werror -fstack-protector-strong -D_FORTIFY_SOURCE=2
  -fPIE`, linked with `-pie -Wl,-z,relro,-z,now`.
- Rust **must** have no `unsafe` except calls into C libraries, each
  with a `// SAFETY:` comment saying why it is sound.
- **Must not** use `system()`, `popen()`, `sh -c` or any shell with text
  that came from the user, a file or the network. Start programs with an
  argument list (`g_spawn_async` with an argv array, or
  `std::process::Command`).

## 2. The package

Start from the template, which builds, packages and runs as is:

```sh
nexa-pack new my-app --lang c        # or --lang rust
cd my-app && nexa-pack build         # dist/my-app-0.1.0.nexa
```

`nexa.toml`:

```toml
[package]
id = "my-app"            # must: lowercase, digits, - _ ; starts with a letter. It becomes the command name.
name = "My App"          # what people see
version = "1.0.0"        # must: semantic version; bump it on every release
description = "One plain sentence about what it does"
categories = "Utility;"  # freedesktop categories
icon = "icon.svg"        # should be SVG; Hexa style (section 4)
screenshots = ["shots/main.png"]  # optional: up to 6 PNG/JPEG pictures for the Store page (3 MB each)

[build]
command = "make"
binary = "build/my-app"

[requires]
depends = ["gtk+3.0"]    # Alpine package names
permissions = []         # see below
```

- The `id` **must not** be the name of a system command or a NexaOS
  component (the installer refuses it anyway). Don't start ids with `nexa-`.
- `depends` **should** only list packages NexaOS already ships (GTK 3,
  GStreamer and its plugins, cairo, curl, ...), so installing works
  offline. Anything else makes installing need the internet.
- `permissions` **must** be the fewest the app needs, from this list only:
  `network`, `files.home`, `files.documents`, `files.pictures`,
  `files.music`, `files.videos`, `camera`, `microphone`, `location`,
  `notifications`, `background`. Anything else makes the package invalid.
- **Permissions are enforced.** An installed app is started by `nexa-run`,
  which confines it with Landlock to what it declared (and the person
  hasn't switched off). Without a permission the app simply can't:
  | Always | the system read-only; screen, sound and GPU; `/tmp`; its own `~/.local/share/<id>`, `~/.config/<id>`, `~/.cache/<id>`; the theme and fonts |
  |---|---|
  | `files.videos` (etc.) | that one folder, read and write |
  | `files.home` | the whole home folder (ask for this only if you must) |
  | `network` | TCP connections; without it every TCP connect is refused |
  | `camera` | `/dev/video*` |
  | a file opened with the app | that file, read-only (`app FILE` on the command line) |

  Not enforced yet (Landlock can't see them): `microphone` and
  `notifications` (PipeWire and D-Bus sockets), `location`, `background`,
  and UDP. Declare them honestly anyway. `nexa-run --explain <id>` prints
  exactly what an app may use. **Test your app installed**, not only from
  `build/`: an app that reads `~/Downloads` without `files.home` will find
  it empty.

## 3. How an app behaves

- **Must** run as the person using it. **Must never** call `doas`, `sudo`
  or `su`, ask for a password itself, be setuid, or write outside the home
  folder.
- System changes (brightness, Wi-Fi radio, packages, mounting a drive, ...)
  **must** go through the system broker, `/run/nexa-broker.sock` (one JSON
  line in, one out; see `docs/SYSTEM_BROKER.md`). The broker checks the
  request, asks for the password itself when needed, and logs it. Only ask
  for actions listed there. If you need one that doesn't exist, say so
  instead of working around it.
- Own data **must** live in the XDG folders for its id:
  `$XDG_DATA_HOME/<id>` (default `~/.local/share/<id>`), config in
  `$XDG_CONFIG_HOME/<id>`, cache in `$XDG_CACHE_HOME/<id>`.
- The person's files: only through a file chooser, drag and drop, a path
  on the command line, or the folders its `files.*` permission names.
  **Must never** read or write `~/.ssh`, `~/.gnupg`, keyrings, password
  stores, browser profiles, cloud credentials, or `~/.local/share/nexa-*`.
- Network **only** with the `network` permission, HTTPS only. **Must
  never** run, load or `eval` anything downloaded.
- Every file, network reply and argument is untrusted: check sizes and
  bounds, refuse what doesn't parse, and **never crash on bad input**:
  show a message.
- **Should** start in under a second on the reference laptop (HP EliteBook
  840 G6). Do slow work (scanning, decoding, network) off the GTK main
  thread, and touch widgets only from the main thread.
- Background work after the window closes needs the `background`
  permission. No services that start at boot.

## 4. Design: Hexa, NexaOS's own look

NexaOS has its own identity. **Must not** copy the look of macOS, Windows,
GNOME, iOS or Android (their layouts, icons, controls or wording).

**Colours: must use the theme's colour names**, so the app follows the
theme the person picked. NexaOS defines them for every GTK app
(`~/.config/gtk-3.0/gtk.css`, written by the theme). Ship this fallback
block at the top of your CSS, so the app also looks right where the theme
isn't loaded. The theme overrides it automatically:

```css
/* Fallback: NexaOS "Horizon". The person's theme overrides these. */
@define-color nexa_bg #0c1228;         @define-color nexa_bg_deep #080c1e;
@define-color nexa_surface #101732;    @define-color nexa_surface_hi #222a46;
@define-color nexa_hover #333a54;      @define-color nexa_field #0f152d;
@define-color nexa_border #363d5a;     @define-color nexa_text #d9e0f6;
@define-color nexa_text_strong #e6ecff;@define-color nexa_muted #8f9bc4;
@define-color nexa_disabled #525b79;   @define-color nexa_accent #4da3ff;
@define-color nexa_accent_hover #6db4ff;@define-color nexa_on_accent #080c1e;
@define-color nexa_selected #264779;   @define-color nexa_selected_text #e6ecff;
@define-color nexa_crit #ff5c7a;       @define-color nexa_ok #5ee6a8;
@define-color nexa_warn #ffb547;
```

**Follow theme changes live (should):** the theme's colours are in
`~/.config/gtk-3.0/gtk.css`, which GTK reads only when the app starts. Watch
that file (`g_file_monitor_file`) and load it again into a provider at
`GTK_STYLE_PROVIDER_PRIORITY_USER + 1` when it changes, so an open window
follows a theme change; `apps/video-player` (`follow_theme()`) does exactly
this.

Load your CSS with `GTK_STYLE_PROVIDER_PRIORITY_APPLICATION` (not
`USER`, or the theme can't override your fallbacks), then use only these
names: `background-color: @nexa_surface; color: @nexa_text;`. **Must
not** hard-code other colours, except inside media (a video frame, a photo).

| Token | Value |
|---|---|
| Type | **Inter** for the interface, **JetBrains Mono** for code, numbers that update, terminals. Sizes (pt): caption 8.5, body 10, title 12, heading 15, display 22 |
| Corners | 6 px small (fields), 10 px medium (buttons, rows), 16 px large (cards) |
| Spacing | 4, 8, 12, 16, 24, 32 px only |
| Motion | 160 ms (state), 240 ms (panels), 320 ms (pages); ease-out `cubic-bezier(0.22, 1, 0.36, 1)`. Nothing loops or bounces |
| Shape | the **pointy-top hexagon** is the NexaOS mark: badges, the main action button (for example a play button), app icons. Use it for one or two things per screen, not everything |
| Buttons | one primary action per view: `@nexa_accent` background, `@nexa_on_accent` text, bold. Danger actions use `@nexa_crit` |

- Dark first. Text **must** stay readable: at least 4.5:1 contrast for
  body text (the tokens already meet this).
- Every view **should** have a friendly empty state ("No videos yet. Put
  some in ~/Videos, or press Open.") rather than a blank area.
- Full keyboard use: every action has a shortcut or is reachable with Tab;
  `Ctrl+O` open, `Ctrl+Q` or `Esc` close, `F11` fullscreen where it makes
  sense. Show shortcuts in tooltips.
- Works at 360 px wide and up to 4K; text wraps or ellipsises, never clips.
- **Words:** plain and short. Say what happened and what to do next ("The
  drive is still in use - close anything using it and try again."). No
  jargon, no error codes on their own, no blame, no exclamation marks.

## 5. Building and signing

- Build with `nexa-pack build`. Packages are reproducible (same source,
  same bytes), so don't put timestamps or random values in the build.
- Apps that are part of NexaOS live in `apps/<id>/` and are built and
  signed by `make nexa-apps` (scripts/build-apps.sh). It signs with the
  NexaOS release key when the build machine has it.
- **Must never** put a secret key (`*.key`) in the repository, a package,
  a log, a commit message or a chat. Only `.pub` keys are shared.

## 6. Testing: required before saying an app works

Show the evidence: commands and their output, plus screenshots you
actually looked at.

1. It builds with no warnings (`-Werror`).
2. `nexa-pack verify dist/<id>-<ver>.nexa --keys assets/keys` reports
   `"ok":true` (and `"trust":"trusted"` when signed with the release key).
3. Building twice gives byte-identical packages.
4. **Run it from the package**, the way the installer does: `nexa-pack
   extract` into a fresh folder, then run `bin/<id>` in a headless Sway
   (see `scripts/test-app-headless.sh` and how `apps/video-player` was
   tested) and take screenshots of:
   - the empty state
   - a normal use
   - an error (bad file, missing permission, no network)

   Look at them: no clipped text, no default-grey GTK, no dark-on-dark.
5. Feed it hostile input: an empty file, a huge file, a file with the
   wrong extension, a filename with spaces and quotes. It must not crash.
6. Before release, the live-boot test (`scripts/test-live-boot.py`) must
   show `NEXA_INSTALL=ok:true,command,menu` for the app on a real ISO.

## 7. Checklist

- [ ] C or Rust; hardened flags; no shell with untrusted text
- [ ] `nexa.toml`: valid id/version, fewest permissions, offline-friendly depends
- [ ] Works when installed (confined by nexa-run): checked with `nexa-run --explain <id>` and a real run
- [ ] Runs as the user; system changes only via the broker; data in XDG folders
- [ ] Never touches secrets or files outside what it declared
- [ ] Hexa: `@nexa_*` colours with the fallback block, Inter, 6/10/16 radii, hexagon mark, one primary action
- [ ] Empty states, keyboard shortcuts, plain words, no copied look
- [ ] Builds clean, verifies, reproducible, runs from the extracted package
- [ ] Screenshots looked at; hostile input tested
- [ ] No secret keys anywhere

## For AI assistants specifically

- Read this file, `docs/PACKAGES.md`, `docs/SYSTEM_BROKER.md` and
  `apps/video-player/` first. Don't invent permissions, broker actions,
  `nexa.toml` fields or design tokens that aren't listed here.
- Don't say an app works until section 6 is done, and include the evidence.
- When something can't be done within these rules, stop and say so,
  rather than loosening a rule (for example adding Python, a shell call, a
  broad permission or a hard-coded colour).
