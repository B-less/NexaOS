# Nexa Browser

WebKitGTK (the engine, C++) in a Nexa window (Python/GTK, `browser/nexa_browser`),
with ad and tracker blocking compiled by a Rust tool (`tools/nexa-webfilter`).

## What it does

| | |
|---|---|
| Tabs | Ctrl+T new, Ctrl+W close, Ctrl+Shift+T reopen closed, Ctrl+Tab / Ctrl+1–9 switch, drag to reorder, middle-click a tab to close it. Links that open a new window (target=_blank, "Open link in new window") become tabs; middle-click or Ctrl+click a link opens it in the background. A tab playing sound shows 🔊; click it to mute that tab. |
| New-tab page | A search box (your search engine), your most-visited sites and bookmarks. Made locally, nothing fetched. |
| Ads and trackers | The 🛡 button: on by default, from a built-in list of well-known ad networks, data brokers and tracking pixels. Click it to allow them on one site. Settings adds bigger lists (EasyList, EasyPrivacy, or any https URL), refreshed weekly. |
| HTTPS-only | http:// addresses are tried over https first. If a site has no secure version you're asked, never sent there silently. Local addresses (localhost, 192.168.x.x, .local) are left alone. |
| Site permissions | Camera, microphone, location, notifications: the site asks, a bar asks you, the answer is remembered per site (never in private windows). Settings lists and forgets them. |
| Pop-ups | Only when you clicked something. |
| Find in page | Ctrl+F, with the number of matches; Enter / Shift+Enter for next / previous. |
| Zoom | Ctrl+plus / minus / 0, remembered per site. |
| Bookmarks | ☆ or Ctrl+D; Ctrl+Shift+O lists and searches them; they also come up as you type an address. |
| History | Ctrl+H; as you type an address. Clear it (with cookies and cache, or not) in Settings. |
| Session | The tabs from last time come back (normal windows only; switch off in Settings). |
| Private / Tor windows | Ctrl+Shift+N saves nothing; a Tor window also goes through Tor (`nexadark`). Both keep WebRTC off; Tor windows also refuse camera, microphone and location. |
| Also | Full screen (F11, and videos' own), Print (Ctrl+P), View source (Ctrl+U), Developer tools (F12), spell checking, downloads with speed and time left. |

## Video

Plays: H.264/AAC, VP8, VP9, AV1 (fast, with dav1d), Opus, streamed video
(MediaSource: YouTube and most sites) and **HLS** (.m3u8, news, sports,
social and live sites).

Why some videos didn't play before: WebKitGTK 2.48 only plays HLS when
`WEBKIT_GST_ENABLE_HLS_SUPPORT=1` is set, and because WebKit introduces
itself like Safari, sites hand it HLS directly, so those videos never
started. The browser sets it now. AV1 had only the slow reference decoder;
`gst-plugins-rs-dav1d` is added.

Can't play: copy-protected (DRM) video (Netflix, Spotify, Disney+, Prime
Video). WebKit has no DRM module on Linux; the browser says so on those
sites instead of showing a broken player.

## How blocking works

`nexa-webfilter compile` turns filter lists (hosts files, domain lists,
Adblock network rules with `$third-party`, `$domain=`, resource types,
exceptions, element hiding) into WebKit's content-blocker JSON; WebKit
compiles that into a matcher inside the engine, so nothing runs per request
and nothing leaves the computer. Rules it can't express (redirects, CSP,
scriptlets, regular expressions) are skipped and counted, never guessed.
At most 50,000 rules; compiled rules are cached until a list or a site
switch changes.

## Tests

- `make test-browser`: the real system image as a container, headless Sway,
  real pages: blocking (and the site switch), element hiding, HTTPS-only,
  tabs, links to new tabs with a real (virtual) mouse, pop-up blocking,
  find, zoom per site, bookmarks, a permission prompt, every video format
  including HLS and MediaSource, session restore, the menu and Settings.
  Screenshots in `dist/browser-test`.
- `scripts/test-browser-media.sh`: the video formats alone.
- `common/tests/test_browser_data.py`: settings, bookmarks, session,
  new-tab page, the compiled built-in list.
- `cargo test -p nexa-webfilter`: parsing, rule order, decisions, bounds.
