# Changelog

## Unreleased (M1)
- **Xbox Series X|S-style interface** (matched against a walkthrough video of the 2020 dashboard):
  - Home: hero grid (big recent game, shortcut and pin squares, My games & apps, and wide recent tiles) plus
    sections (Pinned, Steam, Epic Games, Custom).
  - The Guide (left flyout with tabs, recent games and Settings, a bottom icon row and a big clock).
  - My games & apps: a filter sidebar that slides away when you enter the full-width grid, Sort, Hidden games.
  - Settings: category sidebar plus two-column option tiles (values, checkboxes, dropdown chevrons, accent marker).
  - Xbox-layout on-screen keyboard (tall side keys, cursor movement, controller hints).
  - Search (Y) with live results and the docked keyboard.
  - Accent focus ring with glow, accent toasts, a bottom-right hint pill, and a generated accent wallpaper.
- Profile backgrounds: default wallpaper, solid colour, or a custom image.
- Steam hero art is used for wide tiles and section backdrops.
- Spatial (nearest-tile) navigation; image cache with size classes and LRU eviction.
- Win2Console returns to the front when a game exits.
- Playtime tracking: launch, detect running, record per profile.
- Profiles, Steam/Epic detection, custom games, controller file browser, SDL3 controller input.
- Project scaffold: Cargo workspace (`shell`, `core`, `input`, `library`, `platform`).
- Documentation set: README, BUILDING, ARCHITECTURE, PROVIDERS, USER_GUIDE, DECISIONS.
