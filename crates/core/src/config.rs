use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::fsutil::write_atomic;

/// Global (not per-profile) settings, stored in `config.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Pre-selected on the profile picker.
    pub last_profile: Option<Uuid>,
    pub start_with_windows: bool,
    /// Discord application (Social SDK) id, from the Discord Developer Portal.
    pub discord_app_id: Option<String>,
    /// Turn off the dashboard's ambient music.
    pub music_off: bool,
    /// Window / taskbar title (`None` = "Win2Console"). The icon is `app-icon.png` in the data folder.
    pub app_title: Option<String>,
    /// Skip the startup animation (and any startup video).
    pub startup_off: bool,
    /// Menu size in percent (`None` = 100).
    pub ui_scale: Option<u32>,
    /// Don't download updates by themselves; just say one is available.
    pub manual_updates: bool,
}

impl Config {
    /// Loads the config. A missing file gives defaults; a corrupt file is logged and replaced by defaults
    /// rather than blocking startup.
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("config.json is invalid ({e}); using defaults");
                Self::default()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!("cannot read config.json ({e}); using defaults");
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        write_atomic(path, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_corrupt_files_give_defaults() {
        let dir = tempdir();
        let path = dir.join("config.json");
        assert_eq!(Config::load(&path), Config::default());
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(Config::load(&path), Config::default());
    }

    #[test]
    fn save_then_load() {
        let dir = tempdir();
        let path = dir.join("config.json");
        let cfg =
            Config { last_profile: Some(Uuid::new_v4()), start_with_windows: true, discord_app_id: Some("123".into()) };
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path), cfg);
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cs-core-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
