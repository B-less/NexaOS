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

- [x] Saved passwords (2026-11-02): offered after signing in, filled only
      when you click Fill In, never in private or Tor windows; the page side
      is a script in its own world the page's scripts can't see; stored by
      nexa-webdata (Rust) encrypted with ChaCha20-Poly1305, the key either a
      0600 key file or a primary password (PBKDF2-HMAC-SHA256); a Passwords
      window (show, copy, delete, never-save list, primary password).
      NexaOS has no keyring yet, so "key in the keyring" became that.
- [x] Reader view (2026-11-02, F9): nexa-webdata finds the article (scores
      paragraph-rich parts, leaves out menus, sidebars, sharing buttons)
      and writes it again with safe tags only; shown in the theme's colours
      with a Content-Security-Policy that lets no script run.
- [x] Pinned tabs, mute a tab, reopen closed tab, tab list (2026-11-02): a
      tab menu (right click: reload, duplicate, pin, mute, close others/to
      the right), pinned tabs first and kept next time, the tab list
      (Ctrl+Shift+A) with search and recently closed tabs (kept next time).
- [x] Bookmark manager (2026-11-02): folders (new, rename, delete), edit
      name/address/folder, search, import and export of other browsers'
      HTML bookmark files through nexa-webdata.

## Store

Has: signed catalogue (nexa-store), categories, app pages with
permissions, install/remove/open through the broker.

- [ ] Updates: which installed apps have a newer version, update one or all.
- [ ] Search.
- [ ] Screenshots on app pages.
- [ ] Per-app permission switches after install (with nexa-run).
- [ ] More apps in the catalogue (see TODO.md: Photos, Backup, Mail).

## Text Editor (text-editor, engine nexa-textio)

Has: one file, syntax colours (GtkSourceView), New/Open/Save/Save As,
asks before throwing away changes, line and column.

- [x] Engine nexa-textio (Rust, 2026-11-01): open any text file whatever its encoding
      (UTF-8, UTF-8/16 with a BOM, else Windows-1252), line endings
      (LF, CRLF, CR), refuse binary files and files over 64 MB with a plain
      message; save safely (new file next to it, synced, then swapped in;
      keeps permissions; same encoding and line endings unless changed);
      refuse to overwrite a file changed by another program since it was
      opened unless asked.
- [x] Tabs: several files, Ctrl+T / Ctrl+W / Ctrl+Tab, a dot on unsaved tabs.
- [x] Find and replace bar (Ctrl+F, Ctrl+H): case, whole word, regular
      expression, next/previous, replace, replace all with a count.
- [x] Go to line (Ctrl+L); zoom (Ctrl +/-/0); word wrap, line numbers,
      show spaces, highlight current line (View menu, remembered).
- [x] Encoding and line endings shown in the status bar and changeable;
      language picker.
- [x] Recent files; drafts of unsaved changes kept (crash or power cut)
      and offered back on the next start; the open tabs come back.
- [x] Notice when the file changes on disk: reload or keep yours.
- [x] Drag and drop files onto the window.

## Image Viewer (image-viewer, engine nexa-imageinfo)

Has: one image, previous/next in the folder, zoom in/out, fit, 100%.

- [x] Engine nexa-imageinfo (Rust, 2026-11-01): size and type from the file's header
      (JPEG, PNG, GIF, WebP, BMP, TIFF), EXIF (camera, lens, date taken,
      exposure, ISO, focal length, orientation, whether GPS is in it);
      turn a JPEG losslessly by changing its EXIF orientation in place.
- [x] Zoom with the wheel around the pointer, drag to pan, double-click
      fit/100%; photos shown the right way up (EXIF orientation).
- [x] Rotate left/right and flip (saved losslessly when the engine can,
      else asks to save a copy); delete to the Trash with undo
      (nexa-fileops); copy the image.
- [x] Strip of the folder's pictures (nexa-thumb); sort by name or date.
- [x] Info panel (I): file, size, dimensions, EXIF.
- [x] Full screen (F11) and slideshow (F5), animated GIFs play.
- [x] Set as wallpaper.

## PDF Viewer (pdf-viewer, engine nexa-pdfdoc)

Has: one page at a time, previous/next, zoom in/out.

- [x] Engine nexa-pdfdoc (C, poppler-glib, 2026-11-01): document info, outline
      (contents), text search with page and position of every match,
      page thumbnails, page text; off the window's thread, so a big PDF
      never freezes it; refuses broken or locked files with a plain message.
- [x] Continuous scrolling through all pages; fit width / fit page / zoom.
- [x] Sidebar: page thumbnails and contents (outline), click to go.
- [x] Search (Ctrl+F): matches highlighted, next/previous, count.
- [x] Go to page; links in the document work; select and copy text.
- [x] Rotate; full screen and presentation mode (F5); dark reading mode.
- [x] Remembers the page and zoom of each document; recent documents.
- [x] Password-protected PDFs: asks for the password.

## Music (music-player, engine nexa-musiclib)

Has: a play list of opened files/folder, play/pause/next/previous, seek,
volume.

- [x] Engine nexa-musiclib (Rust, 2026-11-01): scan ~/Music; read tags (MP3 ID3v1/v2.3/
      v2.4, FLAC and Ogg Vorbis/Opus comments, M4A) and cover art; length;
      a library index kept in the cache and updated only for changed files;
      playlists (.m3u) read and written.
- [x] Library: Songs, Albums (with covers), Artists; search.
- [x] Queue: play next, add to queue, reorder, remove; shuffle and repeat
      (all/one).
- [x] Playlists: make, add to, rename, delete, play.
- [x] Media keys (MPRIS; play/pause/next/previous keys bound to playerctl
      in Sway and Hyprland). The top bar has no player controls yet.
- [x] Remembers the queue and position; gapless next song.
