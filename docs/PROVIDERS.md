# Launcher providers

Each provider implements `LibraryProvider` (`crates/library`): `scan() -> Vec<Game>` (installed games)
and `launch(&Game)`.

## Steam (M1: installed games)
1. Steam root: registry `HKCU\Software\Valve\Steam` value `SteamPath`.
2. Library folders: `<root>\steamapps\libraryfolders.vdf` (KeyValues/VDF; each entry has a `path`).
3. Installed games: `<library>\steamapps\appmanifest_<appid>.acf`. Fields used: `appid`, `name`,
   `installdir`, and `StateFlags` (4 = fully installed). Redistributables and tools such as Steamworks Common
   Redistributables (appid 228980) are filtered out.
4. Launch: `steam://rungameid/<appid>`.
5. M2: owned but uninstalled games and achievements through the Steam Web API (needs the user's SteamID64 + API key).

## Epic Games (M1: installed games)
1. Manifests: `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests\*.item` (JSON).
2. Fields used: `DisplayName`, `InstallLocation`, `LaunchExecutable`, `CatalogNamespace`,
   `CatalogItemId`, `AppName`, `bIsIncompleteInstall`. Entries are skipped when they are incomplete, or when
   `AppCategories` does not include `games` (which drops engine and plugin installs).
3. Launch: `com.epicgames.launcher://apps/<CatalogNamespace>%3A<CatalogItemId>%3A<AppName>?action=launch&silent=true`.

## Custom games (M1)
The user picks an `.exe` in the controller file browser, then names the game with the on-screen keyboard.
Arguments, working directory, cover, icon and background are optional. Assets are copied and downscaled into
`media/<game-id>/`.

## Planned (M2)
GOG Galaxy, EA app, Ubisoft Connect, Battle.net, itch.io, and Xbox / Microsoft Store. Each gets its own section here
when implemented.
