//! Achievements from the Playnite Achievements add-on: its SQLite cache
//! (`%APPDATA%\Playnite\ExtensionsData\<plugin id>\achievement_cache.db`), read-only, for the add-on's
//! current user. Games without points get an Xbox-style 1000 Gamerscore split evenly.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

#[derive(Debug, Clone, PartialEq)]
pub struct Achievement {
    pub name: String,
    pub description: String,
    pub icon: Option<PathBuf>,
    pub unlocked: bool,
    /// ISO-8601 UTC, as the add-on stores it.
    pub unlocked_at: Option<String>,
    pub gamerscore: u32,
    /// Share of players who unlocked it (0–100), if known.
    pub rarity: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecentUnlock {
    pub game: String,
    pub playnite_id: Option<String>,
    pub provider_id: Option<i64>,
    pub achievement: Achievement,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub unlocked: u32,
    pub total: u32,
    pub gamerscore: u32,
    pub max_gamerscore: u32,
}

pub struct Achievements {
    db: Connection,
    dir: PathBuf,
}

/// Which add-on game a Win2Console game is: its Playnite id, store app id, and name.
pub struct GameKey<'a> {
    pub playnite_id: Option<&'a str>,
    pub provider_id: Option<i64>,
    pub name: &'a str,
}

impl Achievements {
    /// Opens the add-on's database if the add-on is installed and has synced.
    pub fn open() -> Option<Self> {
        let data = std::env::var_os("APPDATA").map(PathBuf::from)?.join("Playnite").join("ExtensionsData");
        let path = std::fs::read_dir(data)
            .ok()?
            .flatten()
            .map(|e| e.path().join("achievement_cache.db"))
            .find(|p| p.is_file())?;
        Self::open_at(&path)
    }

    pub fn open_at(path: &Path) -> Option<Self> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .inspect_err(|e| tracing::warn!("achievements db: {e}"))
            .ok()?;
        Some(Self { db, dir: path.parent()?.to_path_buf() })
    }

    fn game_id(&self, key: &GameKey) -> Option<i64> {
        self.db
            .query_row(
                "SELECT Id FROM Games WHERE (?1 IS NOT NULL AND PlayniteGameId = ?1 COLLATE NOCASE)
                    OR (?2 IS NOT NULL AND ProviderGameId = ?2) OR GameName = ?3 COLLATE NOCASE
                 ORDER BY (PlayniteGameId = ?1 COLLATE NOCASE) DESC LIMIT 1",
                params![key.playnite_id, key.provider_id, key.name],
                |r| r.get(0),
            )
            .optional()
            .inspect_err(|e| tracing::warn!("achievements lookup: {e}"))
            .ok()
            .flatten()
    }

    /// The game's achievements, unlocked first (newest first), then locked.
    pub fn list(&self, key: &GameKey) -> Vec<Achievement> {
        let Some(game) = self.game_id(key) else { return Vec::new() };
        let rows = self
            .db
            .prepare(
                "SELECT d.DisplayName, d.Description, d.UnlockedIconPath, d.LockedIconPath,
                        COALESCE(d.ScaledPoints, d.Points), d.GlobalPercentUnlocked,
                        COALESCE(ua.Unlocked, 0), ua.UnlockTimeUtc
                 FROM AchievementDefinitions d
                 LEFT JOIN UserGameProgress p ON p.GameId = d.GameId
                      AND p.UserId IN (SELECT Id FROM Users WHERE IsCurrentUser = 1)
                 LEFT JOIN UserAchievements ua ON ua.UserGameProgressId = p.Id AND ua.AchievementDefinitionId = d.Id
                 WHERE d.GameId = ?1
                 ORDER BY COALESCE(ua.Unlocked, 0) DESC, ua.UnlockTimeUtc DESC, d.Id",
            )
            .and_then(|mut st| {
                st.query_map([game], |r| {
                    let unlocked: bool = r.get(6)?;
                    let icon: Option<String> =
                        if unlocked { r.get(2)? } else { r.get::<_, Option<String>>(3)?.or(r.get(2)?) };
                    Ok((
                        Achievement {
                            name: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                            description: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                            icon: icon.filter(|s| !s.is_empty()).map(|s| self.resolve(&s)),
                            unlocked,
                            unlocked_at: r.get(7)?,
                            gamerscore: 0,
                            rarity: r.get::<_, Option<f64>>(5)?.map(|v| v as f32),
                        },
                        r.get::<_, Option<i64>>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()
            })
            .inspect_err(|e| tracing::warn!("achievements list: {e}"))
            .unwrap_or_default();
        let has_points = rows.iter().any(|(_, p)| p.unwrap_or(0) > 0);
        let even = (1000 / rows.len().max(1)) as u32;
        rows.into_iter()
            .map(|(mut a, points)| {
                a.gamerscore = if has_points { points.unwrap_or(0).max(0) as u32 } else { even };
                a
            })
            .collect()
    }

    pub fn summary(&self, key: &GameKey) -> Option<Summary> {
        let list = self.list(key);
        (!list.is_empty()).then(|| summarize(&list))
    }

    /// Gamerscore across every game the add-on tracks.
    pub fn total_gamerscore(&self) -> u32 {
        let games: Vec<i64> = self
            .db
            .prepare("SELECT DISTINCT GameId FROM AchievementDefinitions")
            .and_then(|mut st| st.query_map([], |r| r.get(0))?.collect())
            .unwrap_or_default();
        games
            .into_iter()
            .map(|id| {
                let name: String = self
                    .db
                    .query_row("SELECT COALESCE(GameName, '') FROM Games WHERE Id = ?1", [id], |r| r.get(0))
                    .unwrap_or_default();
                let pid: Option<String> = self
                    .db
                    .query_row("SELECT PlayniteGameId FROM Games WHERE Id = ?1", [id], |r| r.get(0))
                    .ok()
                    .flatten();
                summarize(&self.list(&GameKey { playnite_id: pid.as_deref(), provider_id: None, name: &name }))
                    .gamerscore
            })
            .sum()
    }

    /// The most recent unlocks across all games, newest first.
    pub fn recent_unlocks(&self, limit: usize) -> Vec<RecentUnlock> {
        let rows: Vec<(String, Option<String>, Option<i64>, String)> = self
            .db
            .prepare(
                "SELECT COALESCE(g.GameName, ''), g.PlayniteGameId, g.ProviderGameId, COALESCE(d.DisplayName, '')
                 FROM UserAchievements ua
                 JOIN AchievementDefinitions d ON d.Id = ua.AchievementDefinitionId
                 JOIN UserGameProgress p ON p.Id = ua.UserGameProgressId
                 JOIN Users u ON u.Id = p.UserId
                 JOIN Games g ON g.Id = d.GameId
                 WHERE u.IsCurrentUser = 1 AND ua.Unlocked = 1
                 ORDER BY ua.UnlockTimeUtc DESC LIMIT ?1",
            )
            .and_then(|mut st| {
                st.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect()
            })
            .inspect_err(|e| tracing::warn!("recent achievements: {e}"))
            .unwrap_or_default();
        rows.into_iter()
            .filter_map(|(game, pid, provider, name)| {
                let key = GameKey { playnite_id: pid.as_deref(), provider_id: provider, name: &game };
                let achievement = self.list(&key).into_iter().find(|a| a.name == name)?;
                Some(RecentUnlock { game, playnite_id: pid.clone(), provider_id: provider, achievement })
            })
            .collect()
    }

    /// Icon paths are URLs, absolute paths, or relative to the add-on's data folder.
    fn resolve(&self, s: &str) -> PathBuf {
        if s.starts_with("http://") || s.starts_with("https://") || Path::new(s).is_absolute() {
            PathBuf::from(s)
        } else {
            self.dir.join(s)
        }
    }
}

pub fn summarize(list: &[Achievement]) -> Summary {
    let mut s = Summary { total: list.len() as u32, ..Default::default() };
    for a in list {
        s.max_gamerscore += a.gamerscore;
        if a.unlocked {
            s.unlocked += 1;
            s.gamerscore += a.gamerscore;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_addon_schema() {
        let dir = std::env::temp_dir().join(format!("cs-ach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("achievement_cache.db");
        let _ = std::fs::remove_file(&path);
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE Users (Id INTEGER PRIMARY KEY, IsCurrentUser INTEGER);
             CREATE TABLE Games (Id INTEGER PRIMARY KEY, ProviderGameId INTEGER, PlayniteGameId TEXT, GameName TEXT);
             CREATE TABLE AchievementDefinitions (Id INTEGER PRIMARY KEY, GameId INTEGER, ApiName TEXT, DisplayName TEXT,
                Description TEXT, UnlockedIconPath TEXT, LockedIconPath TEXT, Points INTEGER, ScaledPoints INTEGER,
                GlobalPercentUnlocked REAL);
             CREATE TABLE UserGameProgress (Id INTEGER PRIMARY KEY, UserId INTEGER, GameId INTEGER);
             CREATE TABLE UserAchievements (Id INTEGER PRIMARY KEY, UserGameProgressId INTEGER,
                AchievementDefinitionId INTEGER, Unlocked INTEGER, UnlockTimeUtc TEXT);
             INSERT INTO Users VALUES (1, 1);
             INSERT INTO Games VALUES (7, 2379780, 'abc', 'Balatro');
             INSERT INTO AchievementDefinitions VALUES (1, 7, 'a', 'First', 'd1', 'icons/a.png', 'icons/a_l.png', NULL, NULL, 50.0),
                (2, 7, 'b', 'Second', 'd2', NULL, NULL, NULL, NULL, 5.0),
                (3, 7, 'c', 'Third', 'd3', NULL, NULL, NULL, NULL, 1.0),
                (4, 7, 'd', 'Fourth', 'd4', NULL, NULL, NULL, NULL, 0.5);
             INSERT INTO UserGameProgress VALUES (1, 1, 7);
             INSERT INTO UserAchievements VALUES (1, 1, 1, 1, '2026-01-01T00:00:00Z');",
        )
        .unwrap();
        drop(db);
        let a = Achievements::open_at(&path).unwrap();
        let key = GameKey { playnite_id: None, provider_id: Some(2379780), name: "Balatro" };
        let list = a.list(&key);
        assert_eq!(list.len(), 4);
        assert!(list[0].unlocked && list[0].name == "First");
        assert_eq!(list[0].icon.as_deref(), Some(dir.join("icons/a.png").as_path()));
        assert_eq!(a.summary(&key), Some(Summary { unlocked: 1, total: 4, gamerscore: 250, max_gamerscore: 1000 }));
        assert_eq!(a.total_gamerscore(), 250);
        let recent = a.recent_unlocks(5);
        assert_eq!((recent.len(), recent[0].game.as_str(), recent[0].achievement.gamerscore), (1, "Balatro", 250));
    }
}
