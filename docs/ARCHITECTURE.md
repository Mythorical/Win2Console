# Architecture

## Goals → constraints
| Spec goal | How it is met |
|---|---|
| Minimal memory | Native Rust binary, no browser runtime in the shell. Images are downscaled to tile size on cache, with an LRU of decoded textures. Target: **< 100 MB idle** with the library loaded. |
| High performance / fast menus | Slint UI compiled to native code at build time, GPU rendering via femtovg (OpenGL). The library loads from SQLite and scans run on a background thread. |
| Fast load | No scanning on the startup path: the cached library is shown immediately and a rescan runs in the background. |
| Stable | Memory-safe language, crash-safe SQLite storage, logs in `%APPDATA%\Win2Console\logs`. |
| Any controller | SDL3 gamepad API with the SDL controller mapping DB (Xbox, PlayStation, Switch Pro, 8BitDo, generic). |

## Crates
```
crates/
  shell/     Win2Console.exe: Slint window, screens, app state, input → UI glue
    ui/      .slint components (theme, tiles, screens, on-screen keyboard, file browser)
  core/      Models (Profile, Game, LinkedAccount), SQLite + JSON storage, app paths
  input/     SDL3 event pump thread → NavAction stream
  library/   LibraryProvider trait + Steam / Epic / Custom providers, process tracker
  platform/  Win32 wrappers (registry, process launch/kill, power, start-with-Windows)
```
Dependency direction: `shell → {core, input, library, platform}`, `library → {core, platform}`.
`core` and the provider parsers have no Windows-only dependencies, so they test on any OS.

## Input → navigation
```
SDL3 thread ── NavAction ──▶ slint::invoke_from_event_loop ──▶ App::on_nav(action)
Keyboard ──▶ root FocusScope ──▶ App::on_key(text) ──▶ App::on_nav(action)
Mouse ──▶ TouchArea ──▶ App::on_pointer(area, row, col, click?)
```
All navigation state (screen, focused row/column, modal stack) lives in `crates/shell/src/app.rs` and is
pushed to the Slint window as properties (ADR-009). Movement rules are pure functions in `nav.rs`, with unit tests.
Holding a direction or the stick auto-repeats it: 400 ms delay, then every 80 ms. The stick uses hysteresis
(press at 50 % deflection, release at 35 %). Mouse hover is ignored for 800 ms after controller or keyboard
input, so tiles scrolling under a resting cursor don't steal focus.

| Controller | Keyboard | Action |
|---|---|---|
| D-pad / left stick | Arrow keys | Move focus |
| A / Cross | Enter / Space | Select / Play |
| B / Circle | Esc / Backspace | Back |
| X / Square | X | Pin / unpin (OSK: backspace) |
| Y / Triangle | Y | Sort (OSK: space; file browser: This PC) |
| Menu / Options | M | Options for the focused game (OSK: done) |
| View / Share | V | Add a game (in My games) |
| LB / RB | PageUp / PageDown | Switch tabs / jump to My games |
| Guide / PS | Home | Back to Home (overlay in M3) |

SDL names face buttons by *position* (south/east/west/north), so on PlayStation and Nintendo pads the
bottom button is always "A".

## Measured (2026-11-15, release build, Win11 VM with VirtualBox SVGA 3D)
| Metric | Value |
|---|---|
| `Win2Console.exe` size | 17.9 MB (static SDL3 + SQLite, LTO) |
| Working set after visiting Home, My games & apps and the Guide | 84 MB |
| Private memory | 49 MB |

Much of the working set is the OpenGL driver. Each decoded tile cover adds about 0.9 MB, up to the cache limit.

## UI structure
- The look follows the Xbox Series X|S dashboard (ADR-011). Sizes are design pixels on a 1920×1080 canvas,
  multiplied by `Theme.scale = min(width/1920, height/1080)`.
- **Rust owns tile geometry.** `app.rs` computes every tile's rectangle (Home hero grid, sections, library grid,
  settings tiles, keyboard keys), and Slint only draws them. The same rectangles drive **spatial navigation**
  (`nav::spatial_move`: nearest tile in the pressed direction, preferring tiles in the same row or column band),
  so focus always moves the way the screen looks.
- `ui/`: `theme.slint` (colours, icons), `components.slint` (tile, focus ring, hint pill, wallpaper),
  `home.slint`, `library.slint`, `settings.slint`, `overlays.slint` (Guide, menus, pickers, keyboard, toast,
  list page), `profiles.slint`, and `app.slint` (root).

## Rendering and memory
- All lists use persistent `VecModel`s that are **diffed row by row** on every render. Moving focus only changes a
  few properties, and tiles are never recreated.
- Images are decoded on a background thread at the size they're shown: tiles at most 400×600, wide tiles
  832×432, backdrops 1280×720. Each size class has its own LRU limit.
- Steam box art and hero images come from Steam's local cache (`appcache/librarycache`), with no network needed.
- Renderer quirk (femtovg): a zero-blur drop shadow with a visible colour, or a fully transparent rounded
  rectangle, can stop later text from drawing. Conditional shadows switch to a transparent colour, and conditional
  rounded backgrounds use `visible` (ADR-012).

## Data layout (`%APPDATA%\Win2Console\`)
```
config.json                 global settings (last profile, start with Windows, ...)
library.db                  SQLite: games, per-profile stats (playtime, last played, pinned, hidden)
profiles/<uuid>/profile.json  name, accent colour, linked accounts
profiles/<uuid>/avatar.png
media/<game-id>/            cover.png, icon.png, background.png (downscaled copies)
logs/                       rolling daily logs
```

## Process tracking (follows Playnite's approach)
- **Custom games:** spawned directly. The PID is tracked until it exits.
- **Launcher games (Steam/Epic URI):** after the URI launch, running processes are polled every second for
  one whose executable sits under the game's install directory. Tracking ends when no such process is left.
  The polling gives up after 3 minutes if the game never starts.
- On exit, the playtime delta and `last_played` are written for the active profile.
