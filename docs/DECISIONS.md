# Design decisions (ADR log)

## ADR-001: Rust + Slint
**Date:** 2026-11-15. **Status:** accepted (chosen by the user).
Rust gives native speed and memory safety. Slint is a declarative UI compiled to native code, with
animations and focus handling built in, and it uses little memory. Alternatives considered: egui (immediate mode, which makes
Xbox-style animated tiles harder) and C++/SDL3 (more stability risk).
**Consequence:** Slint's royalty-free license requires the "Made with Slint" attribution, which goes on the About screen.
The project's own license is the user's choice and is still to be decided.

## ADR-002: SDL3 for controller input
**Date:** 2026-11-15. **Status:** accepted.
SDL3 supports the widest range of controllers (XInput, DualShock/DualSense, Switch Pro, generic HID through the mapping DB),
reports battery level, and handles hot-plug. It is linked statically, so there is no DLL to ship. Building it needs CMake.

## ADR-003: Development in a clean Windows 11 VM
**Date:** 2026-11-15. **Status:** accepted (chosen by the user).
Code stays on the Linux host. The VM builds and runs it. The existing malware-analysis VM is excluded
because real accounts get signed in during testing.

## ADR-004: Milestone delivery
**Date:** 2026-11-15. **Status:** accepted (chosen by the user).
M1: shell, navigation, profiles, local library. M2: accounts, metadata, more launchers. M3: overlay.

## ADR-005: Discord integration, decision deferred to M3
**Date:** 2026-11-15. **Status:** open.
Discord's public APIs don't allow third-party clients to read or send DMs or join calls on a user's behalf.
Two options are still on the table:
- (a) Show the official web client in an overlay panel, with a controller layer on top (virtual cursor, on-screen keyboard,
  Discord's own hotkeys).
- (b) Hide the web client and drive it from a custom UI. This means scripting Discord's page, which counts as
  client modification/self-botting under Discord's ToS. The account could be flagged, and it breaks whenever Discord changes its site.

The user will choose before M3 starts.

## ADR-006: Controller-only replacements for native dialogs
**Date:** 2026-11-15. **Status:** accepted.
Native Windows file dialogs and the touch keyboard can't be fully driven with a gamepad, so the shell
ships its own file browser and on-screen keyboard.

## ADR-007: In-house VDF parser instead of `keyvalues-parser`
**Date:** 2026-11-15. **Status:** accepted.
Steam's KeyValues files are a tiny format, and we only read a few fields. A roughly 150-line parser with unit tests
(`crates/library/src/vdf.rs`) removes a dependency and its API churn.

## ADR-008: Process tracking: exact exe for custom games, install folder for launcher games
**Date:** 2026-11-15. **Status:** accepted.
Matching every process inside a custom game's folder breaks for exes that live in shared folders: a game in
`C:\Windows\System32` would "run" forever because of `svchost.exe`. Custom games are therefore tracked by launched
PID plus an exact executable-path match. Steam and Epic games, which are started by their launcher, are tracked by
any process inside their install folder (Playnite does the same).
Note: on Windows 11, `notepad.exe` is a stub that hands off to the Store app and exits immediately, so tests
use `charmap.exe`, a classic Win32 app.

## ADR-009: Navigation state lives in Rust, not in Slint focus
**Date:** 2026-11-15. **Status:** accepted.
Slint's built-in focus moves in Tab order, not spatially (left/right/up/down on a grid). A single root `FocusScope`
forwards every key to Rust. A `nav` module there owns the current screen and the focused row and column, and pushes
them to the UI as properties. That makes navigation identical for controller, keyboard and mouse, and lets it be unit-tested.

## ADR-010: No VM snapshots
**Date:** 2026-11-15. **Status:** accepted (chosen by the user, who accepted the risk).
A live snapshot (`VBoxManage snapshot take --live`) of the running 8 GB VM didn't converge: its `.sav` file grew to
45.8 GB, and the VM had to be force-killed. The partial snapshot and its differencing disk were deleted, and the
VM was reattached to its base disk. Development continues without snapshots. If one is ever needed, take it
**offline** (with the VM powered off).

## ADR-011: Match the current Xbox Series X|S dashboard as closely as possible
**Date:** 2026-11-15. **Status:** accepted (user request).
The UI is being redone to match today's Series X|S dashboard (also used on Xbox One) as closely as possible:
full-screen hero art of the focused game, the "Jump back in" square-tile row, the Guide from the left, and
My games & apps with a sidebar. Screens are matched against the user's reference screenshots in `ui-refs/`.
Layout, motion, colours and structure are reproduced. Microsoft's assets (the Xbox logo, the Segoe Xbox font, the
Xbox icon set) are not copied; the Windows system font (Segoe UI) and the Segoe MDL2 Assets icons are used instead.

## ADR-012: Work around femtovg text-drawing glitches
**Date:** 2026-11-15. **Status:** accepted.
In the VM, text after certain elements didn't draw: tile captions, Guide labels, the My games & apps title. This was
narrowed down by changing one thing at a time. A drop shadow with zero blur but a visible colour, which every
unfocused focus ring had, caused it. Fully transparent rounded rectangles were also implicated. The fix: conditional
shadows switch their colour to transparent, and conditional rounded backgrounds toggle `visible` instead of becoming
transparent. If Slint is upgraded, re-check whether this is still needed.

## ADR-013: Spatial navigation driven by Rust-computed geometry
**Date:** 2026-11-15. **Status:** accepted.
The Xbox Home mixes tile sizes (big, square, wide), and its keyboard has tall side keys, so row/column
navigation can't express the movement. Rust computes every tile's rectangle, and focus moves to the nearest
rectangle in the pressed direction (`nav::spatial_move`, unit-tested against the Home layout and the keyboard).
