//! Steam: installed games from `libraryfolders.vdf` + `appmanifest_*.acf`. See `docs/PROVIDERS.md`.

use std::io;
use std::path::{Path, PathBuf};

use cs_core::{Game, GameMedia, LaunchSpec, Provider};

use crate::LibraryProvider;
use crate::vdf;

/// App ids that are installed alongside games but aren't games.
const NON_GAME_APPIDS: &[&str] = &[
    "228980", // Steamworks Common Redistributables
];

/// `StateFlags` bit meaning "fully installed".
const STATE_FULLY_INSTALLED: u32 = 4;

pub struct SteamProvider {
    /// Steam install root, e.g. `C:\Program Files (x86)\Steam`. `None` if Steam isn't installed.
    root: Option<PathBuf>,
}

impl SteamProvider {
    /// Finds Steam through the registry.
    pub fn discover() -> Self {
        // SteamPath is written when Steam first runs; the installer's InstallPath covers a fresh install.
        let root = cs_platform::registry::hkcu_string(r"Software\Valve\Steam", "SteamPath")
            .or_else(|| cs_platform::registry::hklm_string(r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"))
            .map(PathBuf::from);
        Self { root }
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: Some(root.into()) }
    }
}

impl LibraryProvider for SteamProvider {
    fn provider(&self) -> Provider {
        Provider::Steam
    }

    fn is_available(&self) -> bool {
        self.root.as_deref().is_some_and(Path::is_dir)
    }

    fn scan(&self) -> io::Result<Vec<Game>> {
        let Some(root) = self.root.as_deref() else { return Ok(Vec::new()) };
        let mut games = Vec::new();
        for lib in library_folders(root) {
            let steamapps = lib.join("steamapps");
            let entries = match std::fs::read_dir(&steamapps) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("steam: cannot read {}: {e}", steamapps.display());
                    continue;
                }
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let is_manifest = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("appmanifest_") && n.ends_with(".acf"));
                if !is_manifest {
                    continue;
                }
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| parse_app_manifest(&t, &steamapps).map_err(|e| e.to_string()))
                {
                    Ok(Some(mut game)) => {
                        let appid = game.id.trim_start_matches("steam:");
                        game.media.cover = local_cover(root, appid);
                        game.media.background = local_art(root, appid, "library_hero.jpg");
                        games.push(game);
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("steam: skipping {}: {e}", path.display()),
                }
            }
        }
        // The same library can be listed twice (e.g. different path spellings); keep the first.
        let mut seen = std::collections::HashSet::new();
        games.retain(|g| seen.insert(g.id.clone()));
        Ok(games)
    }
}

/// Box art Steam has already downloaded for its own library view, if present. Steam has used two layouts:
/// `librarycache/<appid>_library_600x900.jpg` (older) and `librarycache/<appid>/library_600x900.jpg` (2024+).
pub fn local_cover(root: &Path, appid: &str) -> Option<PathBuf> {
    local_art(root, appid, "library_600x900.jpg")
}

/// Any of Steam's cached library images (`library_600x900.jpg`, `library_hero.jpg`, `header.jpg`, ...)
/// in either cache layout.
pub fn local_art(root: &Path, appid: &str, file: &str) -> Option<PathBuf> {
    let cache = root.join("appcache").join("librarycache");
    [cache.join(format!("{appid}_{file}")), cache.join(appid).join(file)].into_iter().find(|p| p.is_file())
}

/// All library folders: the Steam root plus everything in `libraryfolders.vdf`.
pub fn library_folders(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    let file = root.join("steamapps").join("libraryfolders.vdf");
    match std::fs::read_to_string(&file) {
        Ok(text) => match parse_library_folders(&text) {
            Ok(paths) => out.extend(paths),
            Err(e) => tracing::warn!("steam: {}: {e}", file.display()),
        },
        Err(e) => tracing::warn!("steam: cannot read {}: {e}", file.display()),
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.to_string_lossy().to_lowercase().replace('/', "\\")));
    out
}

/// Supports the current format (`"0" { "path" "..." }`) and the pre-2021 one (`"1" "D:\\Lib"`).
pub fn parse_library_folders(text: &str) -> Result<Vec<PathBuf>, vdf::ParseError> {
    let (_, root) = vdf::parse(text)?;
    let mut out = Vec::new();
    for (key, node) in root.entries() {
        if !key.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        match node {
            vdf::Node::Obj(_) => out.extend(node.get_str("path").map(PathBuf::from)),
            vdf::Node::Str(path) => out.push(PathBuf::from(path)),
        }
    }
    Ok(out)
}

/// Parses one `appmanifest_<id>.acf`. Returns `None` for non-games and incomplete installs.
pub fn parse_app_manifest(text: &str, steamapps: &Path) -> Result<Option<Game>, vdf::ParseError> {
    let (_, state) = vdf::parse(text)?;
    let field = |k: &str| state.get_str(k).ok_or_else(|| vdf::ParseError(format!("missing {k}")));
    let appid = field("appid")?;
    let name = field("name")?;
    let installdir = field("installdir")?;
    let flags: u32 = state.get_str("StateFlags").and_then(|f| f.parse().ok()).unwrap_or(0);

    if NON_GAME_APPIDS.contains(&appid) || flags & STATE_FULLY_INSTALLED == 0 {
        return Ok(None);
    }
    Ok(Some(Game {
        id: Game::make_id(Provider::Steam, appid),
        provider: Provider::Steam,
        name: name.to_owned(),
        install_dir: Some(steamapps.join("common").join(installdir)),
        launch: LaunchSpec::Uri { uri: format!("steam://rungameid/{appid}") },
        media: GameMedia::default(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY_FOLDERS: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"228980"		"297487498"
			"440"		"25864716539"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"label"		""
	}
}"#;

    const LIBRARY_FOLDERS_OLD: &str = r#"
"LibraryFolders"
{
	"TimeNextStatsReport"		"1600000000"
	"ContentStatsID"		"-123"
	"1"		"D:\\SteamLibrary"
}"#;

    fn manifest(appid: &str, name: &str, flags: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{appid}\"\n\t\"universe\"\t\t\"1\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"{name}\"\n}}\n"
        )
    }

    #[test]
    fn library_folders_both_formats() {
        assert_eq!(
            parse_library_folders(LIBRARY_FOLDERS).unwrap(),
            [PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"D:\SteamLibrary")]
        );
        assert_eq!(parse_library_folders(LIBRARY_FOLDERS_OLD).unwrap(), [PathBuf::from(r"D:\SteamLibrary")]);
    }

    #[test]
    fn app_manifest() {
        let steamapps = Path::new("D:/SteamLibrary/steamapps");
        let game = parse_app_manifest(&manifest("440", "Team Fortress 2", "4"), steamapps).unwrap().unwrap();
        assert_eq!(game.id, "steam:440");
        assert_eq!(game.name, "Team Fortress 2");
        assert_eq!(game.install_dir.unwrap(), steamapps.join("common").join("Team Fortress 2"));
        assert_eq!(game.launch, LaunchSpec::Uri { uri: "steam://rungameid/440".into() });

        // 1030 = fully installed (4) + update required (2) + update started (1024): still playable.
        assert!(parse_app_manifest(&manifest("440", "x", "1030"), steamapps).unwrap().is_some());
        // Downloading, not yet installed.
        assert!(parse_app_manifest(&manifest("440", "x", "1026"), steamapps).unwrap().is_none());
        // Redistributables are not games.
        assert!(parse_app_manifest(&manifest("228980", "Redist", "4"), steamapps).unwrap().is_none());
        assert!(parse_app_manifest(r#""AppState" { "appid" "1" }"#, steamapps).is_err());
    }

    #[test]
    fn scan_fixture_tree() {
        let root = std::env::temp_dir().join(format!("cs-steam-test-{}", std::process::id()));
        let steamapps = root.join("steamapps");
        std::fs::create_dir_all(&steamapps).unwrap();
        std::fs::write(steamapps.join("libraryfolders.vdf"), r#""libraryfolders" { }"#).unwrap();
        std::fs::write(steamapps.join("appmanifest_440.acf"), manifest("440", "Team Fortress 2", "4")).unwrap();
        std::fs::write(steamapps.join("appmanifest_228980.acf"), manifest("228980", "Redist", "4")).unwrap();
        std::fs::write(steamapps.join("appmanifest_999.acf"), "garbage").unwrap();

        let cache = root.join("appcache/librarycache/440");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("library_600x900.jpg"), b"").unwrap();
        std::fs::write(root.join("appcache/librarycache/440_library_hero.jpg"), b"").unwrap();

        let games = SteamProvider::with_root(&root).scan().unwrap();
        assert_eq!(games.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["steam:440"]);
        assert_eq!(games[0].media.cover.as_deref(), Some(cache.join("library_600x900.jpg").as_path()));
        let hero = root.join("appcache/librarycache/440_library_hero.jpg");
        assert_eq!(games[0].media.background.as_deref(), Some(hero.as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// A Steam account remembered on this PC (`config/loginusers.vdf`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamUser {
    pub account: String,
    pub persona: String,
}

const STEAM_KEY: &str = r"Software\Valve\Steam";

pub fn steam_root() -> Option<PathBuf> {
    SteamProvider::discover().root
}

pub fn parse_login_users(text: &str) -> Vec<SteamUser> {
    let Ok((_, users)) = vdf::parse(text) else { return Vec::new() };
    users
        .entries()
        .iter()
        .filter_map(|(_, u)| {
            let account = u.get_str("AccountName")?.to_string();
            let persona = u.get_str("PersonaName").unwrap_or(&account).to_string();
            Some(SteamUser { account, persona })
        })
        .collect()
}

pub fn users() -> Vec<SteamUser> {
    steam_root()
        .and_then(|r| std::fs::read_to_string(r.join("config").join("loginusers.vdf")).ok())
        .map(|t| parse_login_users(&t))
        .unwrap_or_default()
}

/// The account Steam signs in to automatically.
pub fn active_user() -> Option<String> {
    cs_platform::registry::hkcu_string(STEAM_KEY, "AutoLoginUser").filter(|s| !s.is_empty())
}

/// Makes `account` the one Steam signs in to: shuts Steam down if it's running (waits up to 20 s), then sets
/// it as the auto-login user. Launching a `steam://` link afterwards starts Steam signed in as that account
/// (it must have "Remember me" on this PC). Blocking; call off the UI thread.
pub fn switch_user(account: &str) -> io::Result<()> {
    use cs_platform::process::{ProcessSource, SystemProcesses};
    let running = || SystemProcesses.processes().iter().any(|(_, p)| p.ends_with("\\steam.exe"));
    if running() {
        let exe = steam_root().map(|r| r.join("steam.exe")).ok_or_else(|| io::Error::other("Steam not found"))?;
        std::process::Command::new(exe).arg("-shutdown").spawn()?;
        let start = std::time::Instant::now();
        while running() {
            if start.elapsed() > std::time::Duration::from_secs(20) {
                return Err(io::Error::other("Steam didn't close"));
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }
    cs_platform::registry::set_hkcu(STEAM_KEY, "AutoLoginUser", Ok(account))?;
    cs_platform::registry::set_hkcu(STEAM_KEY, "RememberPassword", Err(1))
}

#[cfg(test)]
mod login_users_tests {
    #[test]
    fn parses_login_users() {
        let text = r#""users" { "765" { "AccountName" "alice" "PersonaName" "Alice" "MostRecent" "1" } "766" { "AccountName" "bob" } }"#;
        let users = super::parse_login_users(text);
        assert_eq!(users.len(), 2);
        assert_eq!((users[0].account.as_str(), users[0].persona.as_str()), ("alice", "Alice"));
        assert_eq!(users[1].persona, "bob");
    }
}
