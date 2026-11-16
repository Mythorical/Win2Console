//! The game library: game metadata shared by all profiles, which profiles own each game (imported by
//! their library add-ons), whether it's installed on this PC, and per-profile stats.

use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::model::{Game, GameMedia, LaunchSpec, Provider};

pub type DbError = rusqlite::Error;
pub type Result<T> = std::result::Result<T, DbError>;

const SCHEMA_VERSION: i32 = 2;

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS games (
    id          TEXT PRIMARY KEY,
    provider    TEXT NOT NULL,
    name        TEXT NOT NULL,
    install_dir TEXT,
    launch      TEXT NOT NULL,      -- LaunchSpec as JSON
    cover       TEXT,
    icon        TEXT,
    background  TEXT,
    installed   INTEGER NOT NULL DEFAULT 1,   -- installed on this PC
    added_at    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS stats (
    profile_id    TEXT NOT NULL,
    game_id       TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    playtime_secs INTEGER NOT NULL DEFAULT 0,
    last_played   INTEGER,
    pinned        INTEGER NOT NULL DEFAULT 0,
    hidden        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (profile_id, game_id)
);
";

/// v2: which profile's library add-ons imported which games.
const SCHEMA_V2: &str = "
CREATE TABLE IF NOT EXISTS owned (
    profile_id TEXT NOT NULL,
    game_id    TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    provider   TEXT NOT NULL,
    PRIMARY KEY (profile_id, game_id)
);
-- Games a profile already interacted with stay in its library.
INSERT OR IGNORE INTO owned (profile_id, game_id, provider)
    SELECT s.profile_id, s.game_id, g.provider FROM stats s JOIN games g ON g.id = s.game_id;
";

/// A game as imported by a library add-on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedGame {
    pub game: Game,
    /// Installed on this PC (launchable now) vs. only owned on the account.
    pub installed: bool,
}

/// Per-profile data for one game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameStats {
    pub playtime_secs: i64,
    pub last_played: Option<i64>,
    pub pinned: bool,
    pub hidden: bool,
}

/// A game in a profile's library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameEntry {
    pub game: Game,
    pub installed: bool,
    pub stats: GameStats,
}

pub struct Library {
    conn: Connection,
}

impl Library {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// In-memory database for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        // WAL keeps reads fast while a background update writes; returns a row, so use query.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(SCHEMA_V1)?;
        }
        if version < 2 {
            conn.execute_batch(SCHEMA_V2)?;
        }
        if version < SCHEMA_VERSION {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(Self { conn })
    }

    /// Replaces what `provider`'s add-on imported for `profile`. Games no longer reported leave the
    /// profile's library (their stats are kept, so they return with playtime if re-imported).
    /// Existing artwork is kept when the import has none.
    pub fn sync_library(&mut self, profile: Uuid, provider: Provider, games: &[ImportedGame]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM owned WHERE profile_id = ?1 AND provider = ?2",
            params![profile.to_string(), provider.key()],
        )?;
        {
            let mut upsert = tx.prepare(
                "INSERT INTO games (id, provider, name, install_dir, launch, cover, icon, background, installed, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    install_dir = COALESCE(excluded.install_dir, games.install_dir),
                    launch = excluded.launch,
                    cover = COALESCE(excluded.cover, games.cover),
                    icon = COALESCE(excluded.icon, games.icon),
                    background = COALESCE(excluded.background, games.background),
                    installed = excluded.installed",
            )?;
            let mut own =
                tx.prepare("INSERT OR IGNORE INTO owned (profile_id, game_id, provider) VALUES (?1, ?2, ?3)")?;
            let now = crate::now_unix();
            for ig in games.iter().filter(|ig| ig.game.provider == provider) {
                let g = &ig.game;
                upsert.execute(params![
                    g.id,
                    g.provider.key(),
                    g.name,
                    path_str(&g.install_dir),
                    launch_json(&g.launch),
                    path_str(&g.media.cover),
                    path_str(&g.media.icon),
                    path_str(&g.media.background),
                    ig.installed,
                    now,
                ])?;
                own.execute(params![profile.to_string(), g.id, provider.key()])?;
            }
        }
        tx.commit()
    }

    /// Adds or fully replaces a game owned by `profile` (custom games and edits).
    pub fn upsert_game(&mut self, profile: Uuid, g: &Game) -> Result<()> {
        self.conn.execute(
            "INSERT INTO games (id, provider, name, install_dir, launch, cover, icon, background, installed, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name, install_dir = excluded.install_dir, launch = excluded.launch,
                cover = excluded.cover, icon = excluded.icon, background = excluded.background",
            params![
                g.id,
                g.provider.key(),
                g.name,
                path_str(&g.install_dir),
                launch_json(&g.launch),
                path_str(&g.media.cover),
                path_str(&g.media.icon),
                path_str(&g.media.background),
                crate::now_unix(),
            ],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO owned (profile_id, game_id, provider) VALUES (?1, ?2, ?3)",
            params![profile.to_string(), g.id, g.provider.key()],
        )?;
        Ok(())
    }

    /// Deletes a game and (via cascade) everyone's stats and ownership of it.
    pub fn remove_game(&mut self, game_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM games WHERE id = ?1", [game_id])?;
        Ok(())
    }

    pub fn game(&self, game_id: &str) -> Result<Option<Game>> {
        self.conn
            .query_row(
                "SELECT id, provider, name, install_dir, launch, cover, icon, background FROM games WHERE id = ?1",
                [game_id],
                game_from_row,
            )
            .optional()
            .map(Option::flatten)
    }

    /// The profile's library (installed and not installed), sorted by name. Hidden games only if
    /// `include_hidden`.
    pub fn list(&self, profile: Uuid, include_hidden: bool) -> Result<Vec<GameEntry>> {
        self.query_entries(
            "WHERE (?2 OR COALESCE(s.hidden, 0) = 0) ORDER BY g.name COLLATE NOCASE",
            params![profile.to_string(), include_hidden],
        )
    }

    /// Most recently played games in the profile's library.
    pub fn recent(&self, profile: Uuid, limit: u32) -> Result<Vec<GameEntry>> {
        self.query_entries(
            "WHERE s.last_played IS NOT NULL AND COALESCE(s.hidden, 0) = 0 ORDER BY s.last_played DESC LIMIT ?2",
            params![profile.to_string(), limit],
        )
    }

    /// Pinned games in the profile's library, sorted by name.
    pub fn pinned(&self, profile: Uuid) -> Result<Vec<GameEntry>> {
        self.query_entries("WHERE s.pinned = 1 ORDER BY g.name COLLATE NOCASE", params![profile.to_string()])
    }

    fn query_entries(&self, tail: &str, params: impl rusqlite::Params) -> Result<Vec<GameEntry>> {
        let sql = format!(
            "SELECT g.id, g.provider, g.name, g.install_dir, g.launch, g.cover, g.icon, g.background,
                    COALESCE(s.playtime_secs, 0), s.last_played, COALESCE(s.pinned, 0), COALESCE(s.hidden, 0),
                    g.installed
             FROM owned o
             JOIN games g ON g.id = o.game_id
             LEFT JOIN stats s ON s.game_id = g.id AND s.profile_id = o.profile_id
             {tail}"
        );
        // `owned` must be restricted to the profile: prefix the tail's WHERE.
        let sql = sql.replacen("WHERE", "WHERE o.profile_id = ?1 AND", 1);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params, |r| {
            let stats =
                GameStats { playtime_secs: r.get(8)?, last_played: r.get(9)?, pinned: r.get(10)?, hidden: r.get(11)? };
            let installed: bool = r.get(12)?;
            Ok(game_from_row(r)?.map(|game| GameEntry { game, installed, stats }))
        })?;
        rows.filter_map(|r| r.transpose()).collect()
    }

    /// Adds a finished play session to this profile's playtime.
    pub fn record_session(&mut self, profile: Uuid, game_id: &str, secs: i64, ended_at: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO stats (profile_id, game_id, playtime_secs, last_played) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(profile_id, game_id) DO UPDATE SET
                playtime_secs = playtime_secs + excluded.playtime_secs,
                last_played = excluded.last_played",
            params![profile.to_string(), game_id, secs.max(0), ended_at],
        )?;
        Ok(())
    }

    pub fn set_pinned(&mut self, profile: Uuid, game_id: &str, pinned: bool) -> Result<()> {
        self.set_flag(profile, game_id, "pinned", pinned)
    }

    pub fn set_hidden(&mut self, profile: Uuid, game_id: &str, hidden: bool) -> Result<()> {
        self.set_flag(profile, game_id, "hidden", hidden)
    }

    fn set_flag(&mut self, profile: Uuid, game_id: &str, column: &'static str, value: bool) -> Result<()> {
        let sql = format!(
            "INSERT INTO stats (profile_id, game_id, {column}) VALUES (?1, ?2, ?3)
             ON CONFLICT(profile_id, game_id) DO UPDATE SET {column} = excluded.{column}"
        );
        self.conn.execute(&sql, params![profile.to_string(), game_id, value])?;
        Ok(())
    }

    /// Removes everything a library add-on imported for a profile (when the add-on is uninstalled).
    pub fn remove_library(&mut self, profile: Uuid, provider: Provider) -> Result<()> {
        self.conn.execute(
            "DELETE FROM owned WHERE profile_id = ?1 AND provider = ?2",
            params![profile.to_string(), provider.key()],
        )?;
        Ok(())
    }

    /// Removes all data of a deleted profile.
    pub fn delete_profile_stats(&mut self, profile: Uuid) -> Result<()> {
        self.conn.execute("DELETE FROM stats WHERE profile_id = ?1", [profile.to_string()])?;
        self.conn.execute("DELETE FROM owned WHERE profile_id = ?1", [profile.to_string()])?;
        Ok(())
    }
}

/// Maps a row to a `Game`. Rows with an unknown provider or malformed launch JSON are logged and skipped
/// (`Ok(None)`) so one bad row can't break the whole library.
fn game_from_row(r: &Row<'_>) -> Result<Option<Game>> {
    let id: String = r.get(0)?;
    let provider_key: String = r.get(1)?;
    let Some(provider) = Provider::from_key(&provider_key) else {
        tracing::warn!("game {id}: unknown provider {provider_key:?}");
        return Ok(None);
    };
    let launch_raw: String = r.get(4)?;
    let launch = match serde_json::from_str::<LaunchSpec>(&launch_raw) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("game {id}: bad launch spec ({e})");
            return Ok(None);
        }
    };
    Ok(Some(Game {
        id,
        provider,
        name: r.get(2)?,
        install_dir: r.get::<_, Option<String>>(3)?.map(PathBuf::from),
        launch,
        media: GameMedia {
            cover: r.get::<_, Option<String>>(5)?.map(PathBuf::from),
            icon: r.get::<_, Option<String>>(6)?.map(PathBuf::from),
            background: r.get::<_, Option<String>>(7)?.map(PathBuf::from),
        },
    }))
}

fn path_str(p: &Option<PathBuf>) -> Option<String> {
    p.as_ref().map(|p| p.to_string_lossy().into_owned())
}

fn launch_json(l: &LaunchSpec) -> String {
    serde_json::to_string(l).expect("LaunchSpec serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steam(appid: u32, name: &str, installed: bool) -> ImportedGame {
        ImportedGame {
            game: Game {
                id: Game::make_id(Provider::Steam, &appid.to_string()),
                provider: Provider::Steam,
                name: name.into(),
                install_dir: installed.then(|| PathBuf::from(format!("C:/Steam/common/{name}"))),
                launch: LaunchSpec::Uri { uri: format!("steam://rungameid/{appid}") },
                media: GameMedia::default(),
            },
            installed,
        }
    }

    fn names(entries: &[GameEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.game.name.as_str()).collect()
    }

    #[test]
    fn libraries_are_per_profile() {
        let mut lib = Library::open_in_memory().unwrap();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        lib.sync_library(a, Provider::Steam, &[steam(440, "tf2", true), steam(570, "Dota 2", false)]).unwrap();
        lib.sync_library(b, Provider::Steam, &[steam(570, "Dota 2", false)]).unwrap();
        assert_eq!(names(&lib.list(a, false).unwrap()), ["Dota 2", "tf2"]);
        assert_eq!(names(&lib.list(b, false).unwrap()), ["Dota 2"]);
        let dota = lib.list(a, false).unwrap().into_iter().find(|e| e.game.id == "steam:570").unwrap();
        assert!(!dota.installed);
    }

    #[test]
    fn resync_removes_missing_but_keeps_stats() {
        let mut lib = Library::open_in_memory().unwrap();
        let p = Uuid::new_v4();
        lib.sync_library(p, Provider::Steam, &[steam(440, "tf2", true), steam(570, "Dota 2", true)]).unwrap();
        lib.record_session(p, "steam:440", 120, 1000).unwrap();
        lib.sync_library(p, Provider::Steam, &[steam(570, "Dota 2", true)]).unwrap();
        assert_eq!(names(&lib.list(p, false).unwrap()), ["Dota 2"]);
        lib.sync_library(p, Provider::Steam, &[steam(440, "tf2", true), steam(570, "Dota 2", true)]).unwrap();
        let tf2 = lib.list(p, false).unwrap().into_iter().find(|e| e.game.id == "steam:440").unwrap();
        assert_eq!(tf2.stats.playtime_secs, 120);
        lib.remove_library(p, Provider::Steam).unwrap();
        assert!(lib.list(p, false).unwrap().is_empty());
    }

    #[test]
    fn sync_keeps_existing_artwork() {
        let mut lib = Library::open_in_memory().unwrap();
        let p = Uuid::new_v4();
        let mut g = steam(440, "tf2", true);
        g.game.media.cover = Some("C:/media/tf2.png".into());
        lib.sync_library(p, Provider::Steam, &[g.clone()]).unwrap();
        lib.sync_library(p, Provider::Steam, &[steam(440, "tf2", true)]).unwrap();
        assert_eq!(lib.game("steam:440").unwrap().unwrap().media.cover, g.game.media.cover);
    }

    #[test]
    fn sessions_pins_hidden_are_per_profile() {
        let mut lib = Library::open_in_memory().unwrap();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let games = [steam(1, "A", true), steam(2, "B", true), steam(3, "C", true)];
        lib.sync_library(a, Provider::Steam, &games).unwrap();
        lib.sync_library(b, Provider::Steam, &games).unwrap();

        lib.record_session(a, "steam:1", 60, 100).unwrap();
        lib.record_session(a, "steam:1", 30, 300).unwrap();
        lib.record_session(a, "steam:2", 10, 200).unwrap();
        assert_eq!(names(&lib.recent(a, 10).unwrap()), ["A", "B"]);
        assert_eq!(lib.recent(a, 10).unwrap()[0].stats.playtime_secs, 90);
        assert!(lib.recent(b, 10).unwrap().is_empty());

        lib.set_pinned(a, "steam:3", true).unwrap();
        assert_eq!(names(&lib.pinned(a).unwrap()), ["C"]);
        assert!(lib.pinned(b).unwrap().is_empty());

        lib.set_hidden(b, "steam:2", true).unwrap();
        assert_eq!(names(&lib.list(b, false).unwrap()), ["A", "C"]);
        assert_eq!(names(&lib.list(b, true).unwrap()), ["A", "B", "C"]);

        lib.delete_profile_stats(a).unwrap();
        assert!(lib.recent(a, 10).unwrap().is_empty());
        assert!(lib.list(a, false).unwrap().is_empty());
    }

    #[test]
    fn custom_games_and_removal() {
        let mut lib = Library::open_in_memory().unwrap();
        let p = Uuid::new_v4();
        let g = Game {
            id: Game::make_id(Provider::Custom, "x"),
            provider: Provider::Custom,
            name: "Notepad".into(),
            install_dir: None,
            launch: LaunchSpec::Exe { path: "C:/Windows/notepad.exe".into(), args: vec![], working_dir: None },
            media: GameMedia::default(),
        };
        lib.upsert_game(p, &g).unwrap();
        assert!(lib.list(p, false).unwrap()[0].installed);
        lib.record_session(p, &g.id, 5, 1).unwrap();
        lib.remove_game(&g.id).unwrap();
        assert!(lib.game(&g.id).unwrap().is_none());
        assert!(lib.list(p, false).unwrap().is_empty());
    }

    #[test]
    fn migrates_v1_ownership_from_stats() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        let p = Uuid::new_v4();
        conn.execute(
            "INSERT INTO games (id, provider, name, launch, added_at) VALUES ('custom:1', 'custom', 'Old', ?1, 0)",
            [r#"{"kind":"uri","uri":"x"}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO stats (profile_id, game_id, playtime_secs) VALUES (?1, 'custom:1', 5)",
            [p.to_string()],
        )
        .unwrap();
        let lib = Library::init(conn).unwrap();
        assert_eq!(names(&lib.list(p, false).unwrap()), ["Old"]);
    }
}
