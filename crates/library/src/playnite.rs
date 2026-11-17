//! Playnite as the library backend. The Win2Console Bridge extension (`playnite-extension/`) exports
//! Playnite's library to `playnite-library.json`; this module turns that export into Win2Console games
//! and asks Playnite to sync or install through `playnite://win2console/...` links.

use std::path::{Path, PathBuf};

use cs_core::library::ImportedGame;
use cs_core::{Game, GameMedia, LaunchSpec, Provider};
use serde::Deserialize;

/// File name of the export, inside Win2Console's data folder.
pub const EXPORT_FILE: &str = "playnite-library.json";

#[derive(Debug, Deserialize)]
pub struct Export {
    pub version: u32,
    #[serde(default)]
    pub exported_at: Option<i64>,
    #[serde(default)]
    pub playnite_exe: Option<String>,
    #[serde(default)]
    pub games: Vec<ExportedGame>,
}

#[derive(Debug, Deserialize)]
pub struct ExportedGame {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub game_id: Option<String>,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub install_dir: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub play: Option<PlayAction>,
}

#[derive(Debug, Deserialize)]
pub struct PlayAction {
    pub kind: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub args: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
}

/// Which Win2Console provider a Playnite source belongs to.
pub fn provider_for_source(source: Option<&str>) -> Provider {
    let s = source.unwrap_or_default().to_lowercase();
    match s.as_str() {
        "steam" => Provider::Steam,
        "epic" | "epic games" => Provider::Epic,
        "gog" => Provider::Gog,
        "xbox" | "microsoft store" | "xbox game pass" => Provider::Xbox,
        "ea app" | "ea" | "origin" => Provider::Ea,
        "ubisoft connect" | "uplay" | "ubisoft" => Provider::Ubisoft,
        "amazon" | "amazon games" => Provider::Amazon,
        "itch.io" | "itch" => Provider::Itch,
        "battle.net" | "battlenet" => Provider::Battlenet,
        _ => Provider::Other,
    }
}

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl ExportedGame {
    /// Win2Console id: `<store>:<store id>` for store games (matching ids used before Playnite, so playtime
    /// and pins carry over), otherwise `playnite:<Playnite id>`.
    pub fn console_id(&self, provider: Provider) -> String {
        match (provider, non_empty(&self.game_id)) {
            (Provider::Other, _) | (_, None) => Game::make_id(Provider::Other, &self.id),
            (p, Some(gid)) => Game::make_id(p, gid),
        }
    }

    /// How Win2Console starts it: the exported play action, then the store's own launch link (much faster than
    /// waiting for Playnite to start), otherwise through Playnite. Not-installed games
    /// get their Playnite install link instead (choosing them starts the install).
    pub fn launch(&self) -> LaunchSpec {
        if !self.installed {
            return LaunchSpec::Uri { uri: format!("playnite://win2console/install/{}", self.id) };
        }
        match &self.play {
            Some(p) if p.kind == "url" && non_empty(&p.path).is_some() => {
                LaunchSpec::Uri { uri: non_empty(&p.path).unwrap_or_default().to_string() }
            }
            Some(p) if p.kind == "file" && non_empty(&p.path).is_some() => LaunchSpec::Exe {
                path: PathBuf::from(non_empty(&p.path).unwrap_or_default()),
                args: non_empty(&p.args).map(|a| vec![a.to_string()]).unwrap_or_default(),
                working_dir: non_empty(&p.working_dir).map(PathBuf::from),
            },
            _ => {
                let gid = non_empty(&self.game_id);
                let uri = match (provider_for_source(self.source.as_deref()), gid) {
                    (Provider::Steam, Some(id)) => format!("steam://rungameid/{id}"),
                    (Provider::Epic, Some(id)) => format!("com.epicgames.launcher://apps/{id}?action=launch&silent=true"),
                    (Provider::Ubisoft, Some(id)) => format!("uplay://launch/{id}/0"),
                    _ => format!("playnite://playnite/start/{}", self.id),
                };
                LaunchSpec::Uri { uri }
            }
        }
    }

    pub fn to_imported(&self) -> ImportedGame {
        let provider = provider_for_source(self.source.as_deref());
        ImportedGame {
            game: Game {
                id: self.console_id(provider),
                provider,
                name: self.name.clone(),
                install_dir: non_empty(&self.install_dir).map(PathBuf::from),
                launch: self.launch(),
                media: GameMedia {
                    cover: non_empty(&self.cover).map(PathBuf::from),
                    icon: non_empty(&self.icon).map(PathBuf::from),
                    background: non_empty(&self.background).map(PathBuf::from),
                },
            },
            installed: self.installed,
        }
    }
}

pub fn parse(json: &str) -> Result<Export, String> {
    let export: Export = serde_json::from_str(json).map_err(|e| format!("bad Playnite export: {e}"))?;
    if export.version != 1 {
        return Err(format!("unsupported Playnite export version {}", export.version));
    }
    Ok(export)
}

/// Reads the export. `Ok(None)` if Playnite hasn't exported yet.
pub fn read(data_dir: &Path) -> Result<Option<Export>, String> {
    match std::fs::read_to_string(data_dir.join(EXPORT_FILE)) {
        Ok(json) => parse(&json).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Games of the export grouped for syncing (hidden Playnite games are skipped).
pub fn imported_games(export: &Export) -> Vec<ImportedGame> {
    export.games.iter().filter(|g| !g.hidden).map(ExportedGame::to_imported).collect()
}

/// Where Playnite is installed: from the last export, or the default per-user install folder.
pub fn playnite_exe(export: Option<&Export>) -> Option<PathBuf> {
    export
        .and_then(|e| e.playnite_exe.as_deref())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Playnite").join("Playnite.DesktopApp.exe"))
        })
        .filter(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "version": 1,
      "exported_at": 1794792000,
      "playnite_exe": "C:\\Users\\u\\AppData\\Local\\Playnite\\Playnite.DesktopApp.exe",
      "games": [
        { "id": "11111111-1111-1111-1111-111111111111", "name": "Team Fortress 2", "source": "Steam",
          "game_id": "440", "installed": true, "hidden": false, "install_dir": "C:\\Steam\\common\\TF2",
          "cover": "C:\\pl\\cover.jpg", "background": null, "icon": null,
          "play": { "kind": "url", "path": "steam://rungameid/440", "args": null, "working_dir": null } },
        { "id": "22222222-2222-2222-2222-222222222222", "name": "Celeste", "source": null, "game_id": null,
          "installed": true, "hidden": false, "install_dir": "D:\\Games\\Celeste",
          "play": { "kind": "file", "path": "D:\\Games\\Celeste\\Celeste.exe", "args": "-fullscreen", "working_dir": "D:\\Games\\Celeste" } },
        { "id": "33333333-3333-3333-3333-333333333333", "name": "Owned Only", "source": "Epic", "game_id": "Sugar",
          "installed": false, "hidden": false },
        { "id": "44444444-4444-4444-4444-444444444444", "name": "Hidden", "source": "GOG", "game_id": "1",
          "installed": true, "hidden": true },
        { "id": "55555555-5555-5555-5555-555555555555", "name": "No Action", "source": "Ubisoft Connect", "game_id": "720",
          "installed": true, "hidden": false, "play": null },
        { "id": "66666666-6666-6666-6666-666666666666", "name": "Emulated", "source": null, "game_id": null,
          "installed": true, "hidden": false, "play": null }
      ]
    }"#;

    #[test]
    fn maps_export_to_games() {
        let export = parse(SAMPLE).unwrap();
        let games = imported_games(&export);
        assert_eq!(games.len(), 5); // hidden one skipped

        let tf2 = &games[0];
        assert_eq!((tf2.game.id.as_str(), tf2.game.provider, tf2.installed), ("steam:440", Provider::Steam, true));
        assert_eq!(tf2.game.launch, LaunchSpec::Uri { uri: "steam://rungameid/440".into() });
        assert_eq!(tf2.game.media.cover.as_deref(), Some(Path::new("C:\\pl\\cover.jpg")));

        let celeste = &games[1];
        assert_eq!(celeste.game.id, "playnite:22222222-2222-2222-2222-222222222222");
        assert_eq!(celeste.game.provider, Provider::Other);
        assert_eq!(
            celeste.game.launch,
            LaunchSpec::Exe {
                path: "D:\\Games\\Celeste\\Celeste.exe".into(),
                args: vec!["-fullscreen".into()],
                working_dir: Some("D:\\Games\\Celeste".into())
            }
        );

        let owned = &games[2];
        assert!(!owned.installed);
        assert_eq!(owned.game.id, "epic:Sugar");
        assert_eq!(
            owned.game.launch,
            LaunchSpec::Uri { uri: "playnite://win2console/install/33333333-3333-3333-3333-333333333333".into() }
        );

        let store = &games[3];
        assert_eq!(store.game.provider, Provider::Ubisoft);
        assert_eq!(store.game.launch, LaunchSpec::Uri { uri: "uplay://launch/720/0".into() });

        let fallback = &games[4];
        assert_eq!(
            fallback.game.launch,
            LaunchSpec::Uri { uri: "playnite://playnite/start/66666666-6666-6666-6666-666666666666".into() }
        );
    }

    #[test]
    fn rejects_unknown_versions() {
        assert!(parse(r#"{"version": 2, "games": []}"#).is_err());
        assert!(parse("not json").is_err());
    }
}
