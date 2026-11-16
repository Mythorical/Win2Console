//! Connected accounts of one profile (e.g. Discord), stored encrypted with the
//! Windows user's key (DPAPI) in `profiles/<id>/accounts.bin`.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A connected account. `tokens` is service-specific and stored encrypted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tokens: serde_json::Value,
}

pub struct AccountStore {
    path: PathBuf,
}

impl AccountStore {
    pub fn for_profile(profile_dir: &Path) -> Self {
        Self { path: profile_dir.join("accounts.bin") }
    }

    /// Accounts by key (a provider key, or "discord"). Missing or unreadable files give an empty map.
    pub fn load(&self) -> BTreeMap<String, Account> {
        let Ok(sealed) = std::fs::read(&self.path) else { return BTreeMap::new() };
        match cs_platform::secret::unprotect(&sealed)
            .map_err(|e| e.to_string())
            .and_then(|plain| serde_json::from_slice(&plain).map_err(|e| e.to_string()))
        {
            Ok(map) => map,
            Err(e) => {
                tracing::warn!("couldn't read saved accounts ({e}); they need to be connected again");
                BTreeMap::new()
            }
        }
    }

    pub fn save(&self, accounts: &BTreeMap<String, Account>) -> io::Result<()> {
        let plain = serde_json::to_vec(accounts).map_err(io::Error::other)?;
        let sealed = cs_platform::secret::protect(&plain)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, sealed)?;
        std::fs::rename(tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("cs-accounts-{}", std::process::id()));
        let store = AccountStore::for_profile(&dir);
        assert!(store.load().is_empty());
        let mut map = BTreeMap::new();
        map.insert(
            "steam".to_string(),
            Account { id: "1".into(), name: "me".into(), tokens: serde_json::json!({"t": 1}) },
        );
        store.save(&map).unwrap();
        assert_eq!(store.load(), map);
        let raw = std::fs::read(dir.join("accounts.bin")).unwrap();
        if cfg!(windows) {
            assert!(!String::from_utf8_lossy(&raw).contains("\"t\""), "stored encrypted");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
