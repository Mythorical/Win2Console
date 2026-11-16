# Win2Console

A controller-first, console-style shell and universal game launcher for Windows 10/11.
The interface closely follows the Xbox Series X|S dashboard (see the [user guide](docs/USER_GUIDE.md)).
Playnite + CouchKit serve as the reference for features.

![Home](docs/screenshots/m1-home.png)

> Status: **Milestone 1 in progress.** The original spec is in [`touch.txt`](touch.txt).

## Features (by milestone)

| Milestone | Scope | Status |
|---|---|---|
| **M1** | Xbox-style fullscreen shell (Home, Guide, My games & apps, Settings, Search, on-screen keyboard), controller navigation everywhere, profiles (picture, colour, background), local library: Steam, Epic, custom games, playtime tracking, power menu | Working in the dev VM; still to do: real controller and Steam/Epic tests |
| **M2** | Account linking per profile (Steam, GOG, Epic), metadata download (covers, icons, achievements), more launchers | Planned |
| **M3** | In-game overlay: volume, screenshots, resume/close game, Discord | Planned |

## Docs

- [Building & dev VM setup](docs/BUILDING.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Launcher providers](docs/PROVIDERS.md)
- [User guide](docs/USER_GUIDE.md)
- [Design decisions](docs/DECISIONS.md)
- [Changelog](CHANGELOG.md)
- Screenshots: [`docs/screenshots/`](docs/screenshots/)

## Credits

The UI is built with [Slint](https://slint.dev) and is shown as "Made with Slint" on the About screen, as Slint's royalty-free license requires.
Controller support comes from [SDL3](https://libsdl.org).
