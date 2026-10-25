# NexaHexa: the NexaOS icon set (for future use)

Not switched on yet: the OS still uses its current icons. This set is ready
for when NexaOS moves to its own icon theme.

![preview](preview.png)

- **41 app icons** (`NexaHexa/scalable/apps/nexa-*.svg`, 128x128): a white
  glyph on the NexaOS hexagon tile in the app's own gradient. The names
  match the `Icon=` of today's apps (nexa-files, nexa-settings, ...) plus
  apps to come (Mail, Calendar, Camera, Maps, Weather, Backup, ...).
- **121 symbolic icons** (`NexaHexa/scalable/<context>/<name>-symbolic.svg`,
  24x24) with freedesktop names (go-previous, edit-copy, battery-low,
  network-wireless-signal-good, ...), so GTK picks them up by name. They
  are made of filled shapes only: GTK 3 recolours symbolic icons through
  their fill, so they follow the theme's text colour in light and dark.

## The icons shift with the theme

Each NexaOS theme has its own variant, `NexaHexa-<theme>` (horizon, nebula,
aurora, eclipse, daybreak): the same app tiles, each app keeping its own
colour but tinted toward the theme's accent (lifted on the light Daybreak),
and the NexaOS mark (Nexus) in the theme's own accent. The variants only
hold app tiles and inherit everything else from NexaHexa.

![the variants](preview-themes.png)

Once installed, `themes.icon_theme_for()` picks the variant for the current
theme (written to GTK's settings.ini), and `themes.follow_icon_theme()`
switches open apps, the Dock and the desktop live when the theme changes
(the same watcher that recolours them). The variants are generated from
`themes.txt`, which `make iconset` writes from themes.py, so they always
match the real themes.

## Made by code, not drawn by hand

Every icon is a list of shapes on a 24-unit grid in
`forge/src/icons.rs`; `forge` (Rust, no dependencies, not shipped in the
OS) turns them into both kinds of icon, `index.theme` and `preview.svg`.
Change a shape, a colour or the stroke width there and rebuild:

    make iconset

Its tests check that symbolic icons have no strokes, every app has its
glyph, names are unique and every shape stays inside the grid.

## Switching it on later

1. Copy `NexaHexa/` to `/usr/share/icons/NexaHexa` in the image (a stage
   script) and run `gtk-update-icon-cache` on it.
2. Set `gtk-icon-theme-name = NexaHexa` (themes.py writes the GTK
   settings); `Inherits=Adwaita,hicolor` covers anything not in the set.
3. App icons with the same names as today's replace them automatically.
