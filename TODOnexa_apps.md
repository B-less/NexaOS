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
  - [x] The Archive app on the same engine (2026-10-29; it ran `unzip -o` / `tar -xf`, which replaced files).
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
- [x] Permissions in Properties (2026-10-29): Read / Change / Run for you,
      the group and everyone else, applied at once through nexa-fileops op
      "chmod" (no set-id bits; a readable folder stays openable); Ctrl+Z
      restores them; someone else's file shows them read-only.
- [x] Rename many (2026-10-29): F2 with several selected; replace text, or
      a name with numbers (extensions kept); live preview; taken or doubled
      names refused before anything happens; nexa-fileops op "rename_many"
      goes through temporary names (swaps work); one Ctrl+Z puts all back.
- [x] Tabs (2026-10-29): Ctrl+T, Ctrl+W, Ctrl+Tab / Ctrl+PgUp/PgDn,
      middle-click or "Open in New Tab" on a folder; each tab keeps its
      folder, Back/Forward and selection; the tab bar shows only with two or more.
- [x] Recent and Starred (2026-10-29) in the sidebar: Recent is GTK's shared
      list (Files adds what it opens), Starred is Star / Unstar on anything;
      both show each item's folder and have no folder of their own (no
      Paste, New Folder or search there).

## Settings

Has: Overview (Living Field), Personalization, Magic Layer, Bluetooth,
Nexa Link, VPN, System, Users, Updates, Startup, Time & Keyboard, Power,
Privacy, Assistant, About.

- [x] Display (2026-10-29): resolution, refresh rate, scale, rotation,
      position next to another screen; a new C tool, nexa-display, over the
      compositor's wlr-output-management (Sway and Hyprland): tests each
      change first, "Keep these settings?" puts it back after 15 s, kept
      settings come back at every sign-in and after a compositor reload.
      Night light (wlsunset, added to the image): on/off, warmth, hours.
- [x] Sound (2026-10-29): output and input device, volume, mute, and each
      app's volume, through PipeWire (pactl's JSON).
- [x] Mouse & Touchpad (2026-10-29): speed, natural scrolling,
      left-handed, tap to click, ignore while typing; Sway and Hyprland,
      applied at once.
- [x] Wi-Fi page in Settings (2026-10-29): networks, connect (asks the
      password), disconnect, forget; through iwd (nexa_common.wifi).
- [x] Notifications (2026-10-29): do not disturb, how long pop-ups stay,
      where they appear, apps whose pop-ups are hidden (mako).
- [x] Default apps (2026-10-29): browser, email, files, music, video,
      pictures, PDF, text (GIO's mimeapps.list, what Files opens with).
- [x] Accessibility (2026-10-29): text size, pointer size, high contrast
      (a new High Contrast theme), reduce motion (GTK, Hyprland, living
      backgrounds). Screen keyboard: not yet (no package chosen).
- [x] Date & time (2026-10-29): set the time automatically (ntpd through
      the broker), or by hand (new broker action set_time, Rust, needs the
      password), 24- or 12-hour clock in the top bar.

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
