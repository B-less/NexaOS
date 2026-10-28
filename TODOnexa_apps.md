# Full apps: the plan (TODO)

Started 2026-10-28: "build all apps with their full functions as real apps".
Meaning agreed: every app a complete everyday app, not a basic version.
Order: the big apps first (Files, Settings, Browser, Store), then the
everyday ones (Editor, Music, Image Viewer, PDF), the small tools and the
system apps. A box is ticked only when the feature works and its test
passes. Engines in Rust/C (AGENTS.md), Python/GTK only as the window.

## Files (file-manager, engines nexa-fileops + nexa-archiver)

Has: grid/list, sort, hidden files, zoom, copy/cut/paste/move with
progress and undo, Trash and restore, rename, new file/folder, Properties,
Open With, terminal here, drives (mount/eject), drag and drop, thumbnails
(nexa-thumb), FileTrail, Nexa Link send.

- [x] Compress (zip, tar.gz) and Extract Here (zip, tar, gz, xz, bz2,
      zst) through a Rust engine, nexa-archiver (2026-10-28): never
      overwrites (a free name, "Project (2)"), skips entries that would land
      outside the folder, links pointing out, hard links and devices, drops
      set-id bits, caps the unpacked size by the drive's free space, stages
      in a hidden folder so Stop or an error leaves nothing; Landlock to the
      archive and the folder. Undo takes the result to the Trash. Tests:
      cargo test (8), real xz/bz2/zst/gz/zip archives, test_window.py.
  - [ ] The Archive app on the same engine (it still runs `unzip -o` / `tar -xf`).
- [x] Sidebar (2026-10-28): places (Home, Desktop, Documents, Downloads,
      Music, Pictures, Videos that exist), the Trash, pinned folders (Pin to
      Sidebar / Remove from Sidebar, remembered), drives with Eject; marks
      the open folder; F9 or View > Sidebar hides it (then the Home, Trash
      and Devices buttons come back).
- [x] Search (Ctrl+F, 2026-10-28): nexa-fileops op "search", names
      containing every word in any order and case, nearest first, streamed
      into the list while it looks, capped at 2000, no links followed, other
      drives not entered; Escape or opening a folder ends it. Results say
      which folder each is in.
- [ ] Permissions in Properties: change who can read/write/run (engine op).
- [ ] Rename many files at once (find/replace, numbering), with undo.
- [ ] Tabs (Ctrl+T), each with its own folder and history.
- [ ] Recent files and Starred.

## Settings

Has: Overview (Living Field), Personalization, Magic Layer, Bluetooth,
Nexa Link, VPN, System, Users, Updates, Startup, Time & Keyboard, Power,
Privacy, Assistant, About.

- [ ] Display: resolution, refresh rate, scale, rotation, arrangement of
      several screens, night light (native tool over
      wlr-output-management; Sway and Hyprland).
- [ ] Sound: output and input device, volumes, per-app volume (PipeWire).
- [ ] Mouse & Touchpad: speed, natural scrolling, tap to click, left-handed.
- [ ] Wi-Fi page in Settings (today in the Control Centre and the Network app).
- [ ] Notifications: do not disturb, per-app on/off (mako).
- [ ] Default apps: browser, mail, files, music, video, images, PDF.
- [ ] Accessibility: text size, high contrast, larger pointer, reduce
      motion, screen keyboard.
- [ ] Date & time: automatic time on/off, set by hand, 24-hour clock.

## Browser

Has: tabs, ad and tracker blocking (nexa-webfilter), HTTPS-only, site
permissions, find in page, zoom per site, bookmarks, history, downloads,
session restore, private and Tor windows, developer tools, printing,
spell checking, clear browsing data.

- [ ] Saved passwords: offer to save, fill on return, a manager; stored
      encrypted (Rust vault, key in the keyring).
- [ ] Reader view.
- [ ] Pinned tabs, mute a tab, reopen closed tab, tab list.
- [ ] Bookmark manager (folders, edit, import from Firefox/Chrome HTML).

## Store

Has: signed catalogue (nexa-store), categories, app pages with
permissions, install/remove/open through the broker.

- [ ] Updates: which installed apps have a newer version, update one or all.
- [ ] Search.
- [ ] Screenshots on app pages.
- [ ] Per-app permission switches after install (with nexa-run).
- [ ] More apps in the catalogue (see TODO.md: Photos, Backup, Mail).
