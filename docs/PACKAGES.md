# Making NexaOS apps: `.nexa` packages and `nexa-pack`

`nexa-pack` (Rust, `tools/nexa-pack`, installed on NexaOS as
`/usr/local/bin/nexa-pack`) creates, builds, signs and verifies `.nexa`
packages. New NexaOS software is written in **C** (programs close to
Wayland/hardware, GTK apps) or **Rust** (services, tools, anything handling
untrusted data); Python only as a thin interface over them.

## Quick start

```bash
nexa-pack keygen me                      # me.key (secret, keep it private) + me.pub
nexa-pack new my-app --lang c            # or --lang rust
cd my-app && nexa-pack build --key ../me.key
nexa-pack verify dist/my-app-0.1.0.nexa
```

Double-clicking the `.nexa` opens Nexa Installer, which shows the publisher,
what the app asks for, and what it also installs.

## nexa.toml

```toml
[package]
id = "my-app"            # lowercase, digits, - _ ; becomes the command name
name = "My App"
version = "0.1.0"
description = "What it does"
categories = "Utility;"
icon = "icon.svg"        # .svg or .png
terminal = false         # true for command-line tools

[build]
command = "make"         # run with sh -c in the project folder (optional)
binary = "build/my-app"  # the program: installed as /usr/local/bin/my-app
lib = "data"             # optional folder: installed as /usr/local/lib/nexa-my-app

[requires]
depends = ["gtk+3.0"]    # Alpine packages installed first
permissions = ["network"]
```

Permissions: `network`, `files.home`, `files.documents`, `files.pictures`,
`files.music`, `files.videos`, `camera`, `microphone`, `location`,
`notifications`, `background`.

## The package (format 2)

A zip with `manifest.json` (the fields above plus `arch` x86_64, `abi` musl
and the **SHA-256 of every file**), an optional `SIGNATURE` (Ed25519 over
the exact bytes of manifest.json), `bin/<id>`, `lib/…` and the icon. Entries
are sorted with fixed timestamps: building the same project twice gives a
byte-identical package.

## How installing is checked

1. Nexa Installer runs `nexa-pack verify` (no root) to show the details.
2. The root helper copies the package to a private folder, and
   `nexa-pack extract` verifies it and writes the files **from the same
   bytes it checked**. Anything wrong (a changed file, a file not in the
   manifest, unsafe paths, links, bad fields, wrong architecture, a
   signature that doesn't match) and nothing is installed.
3. Signed by a key in `/etc/nexa/keys/` → installs. Unsigned or unknown
   publisher → the installer warns, and installs only if you choose to.
4. The helper never lets a package replace a system command, a root
   helper or another component, and records exactly what it installed so
   uninstalling is clean.

## The Nexa Store

The Store app (`nexa-store-app`, store/nexa_store) browses catalogues and
installs from them. A catalogue is a folder (local, or HTTPS) made with
`nexa-pack repo build DIR --key KEY`: `packages/*.nexa`, `icons/`, and an
`index.json` listing each app's id, version, size and SHA-256, signed in
`index.sig`. The image ships one with NexaOS's own apps
(`/usr/share/nexa-store/repo`, built by `make nexa-apps`); others are added
to `/etc/nexa/store.conf`:

```json
{"catalogues": [{"name": "NexaOS", "url": "file:///usr/share/nexa-store/repo"},
                {"name": "Example", "url": "https://example.org/nexa-repo"}]}
```

The work is done by `nexa-store` (Rust, Landlock-confined, network only on
port 443): an index whose signature doesn't match is refused, one signed by
an unknown key is shown as "Not verified", and a download must match the
index's size and SHA-256 before the broker's `install_app` (which checks the
package's own signature again) installs it. `nexa-store installed` lists
what is installed from any source, with updates. Tested by
`make test-store-app` (real engine and signed catalogues, a stand-in broker,
screenshots in dist/store-test).

## Keys

The NexaOS publisher key is `assets/keys/7003a4c26d287fdf.pub` (installed
to `/etc/nexa/keys/`). Its secret half lives only on the release machine
(`~/.config/nexaos-keys/`, never in the repository; `*.key` is ignored by
git). To trust another publisher, put their `.pub` file in `/etc/nexa/keys/`
named `<key id>.pub`.

## Tests

`make test` (install helper security tests use the real binary) and
`make test-native` (cargo tests + templates built, signed, tampered and run
in a clean Alpine container).
