use std::io;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::fsutil::write_atomic;
use crate::model::{Avatar, Background, DEFAULT_ACCENT, Profile, parse_accent};

/// Stores each profile as `profiles/<uuid>/profile.json` (+ optional `avatar.png`).
#[derive(Debug, Clone)]
pub struct ProfileStore {
    dir: PathBuf,
}

/// Longest allowed profile name, in characters.
pub const MAX_NAME_LEN: usize = 24;

impl ProfileStore {
    pub fn new(profiles_dir: impl Into<PathBuf>) -> Self {
        Self { dir: profiles_dir.into() }
    }

    /// All readable profiles, sorted by name. Unreadable folders are logged and skipped.
    pub fn list(&self) -> io::Result<Vec<Profile>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };
        for entry in entries.flatten() {
            let file = entry.path().join("profile.json");
            match std::fs::read(&file)
                .map_err(io::Error::other)
                .and_then(|b| serde_json::from_slice::<Profile>(&b).map_err(io::Error::other))
            {
                Ok(p) => out.push(p),
                Err(e) => tracing::warn!("skipping profile {}: {e}", file.display()),
            }
        }
        out.sort_by_key(|p| p.name.to_lowercase());
        Ok(out)
    }

    /// Creates and saves a new profile with a default avatar and accent.
    pub fn create(&self, name: &str) -> io::Result<Profile> {
        let profile = Profile {
            id: Uuid::new_v4(),
            name: validate_name(name)?,
            accent: DEFAULT_ACCENT.to_owned(),
            avatar: Avatar::default(),
            background: Background::default(),
            accounts: Vec::new(),
            libraries: Default::default(),
        };
        self.save(&profile)?;
        Ok(profile)
    }

    /// Validates and writes `profile`.
    pub fn save(&self, profile: &Profile) -> io::Result<()> {
        let mut p = profile.clone();
        p.name = validate_name(&p.name)?;
        p.accent = parse_accent(&p.accent)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "accent must be #RRGGBB"))?;
        let dir = self.profile_dir(p.id);
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_vec_pretty(&p).map_err(io::Error::other)?;
        write_atomic(&dir.join("profile.json"), &json)
    }

    /// Removes the profile folder. Per-profile stats in the library DB are removed separately
    /// (`Library::delete_profile_stats`).
    pub fn delete(&self, id: Uuid) -> io::Result<()> {
        match std::fs::remove_dir_all(self.profile_dir(id)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }

    /// Stores already-encoded PNG bytes as the custom avatar. The caller sets `profile.avatar = Avatar::Custom`.
    pub fn write_avatar_png(&self, id: Uuid, png: &[u8]) -> io::Result<()> {
        let dir = self.profile_dir(id);
        std::fs::create_dir_all(&dir)?;
        write_atomic(&dir.join("avatar.png"), png)
    }

    pub fn avatar_path(&self, id: Uuid) -> PathBuf {
        self.profile_dir(id).join("avatar.png")
    }

    /// Stores already-encoded PNG bytes as the custom Home background. The caller sets
    /// `profile.background = Background::Custom`.
    pub fn write_background_png(&self, id: Uuid, png: &[u8]) -> io::Result<()> {
        let dir = self.profile_dir(id);
        std::fs::create_dir_all(&dir)?;
        write_atomic(&dir.join("background.png"), png)
    }

    pub fn background_path(&self, id: Uuid) -> PathBuf {
        self.profile_dir(id).join("background.png")
    }

    pub fn profile_dir(&self, id: Uuid) -> PathBuf {
        self.dir.join(id.to_string())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn validate_name(name: &str) -> io::Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("profile name must be 1-{MAX_NAME_LEN} characters"),
        ));
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ProfileStore {
        let dir = std::env::temp_dir().join(format!("cs-profiles-test-{}", Uuid::new_v4()));
        ProfileStore::new(dir)
    }

    #[test]
    fn crud() {
        let s = store();
        assert!(s.list().unwrap().is_empty());

        let mut b = s.create("  bob ").unwrap();
        let a = s.create("Alice").unwrap();
        assert_eq!(b.name, "bob");
        assert_eq!(s.list().unwrap().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Alice", "bob"]);

        b.accent = "#e81123".into();
        b.avatar = Avatar::Builtin { index: 3 };
        b.background = Background::Solid;
        s.save(&b).unwrap();
        let loaded = s.list().unwrap().into_iter().find(|p| p.id == b.id).unwrap();
        assert_eq!(loaded.accent, "#E81123");
        assert_eq!(loaded.avatar, Avatar::Builtin { index: 3 });
        assert_eq!(loaded.background, Background::Solid);

        s.delete(a.id).unwrap();
        s.delete(a.id).unwrap(); // idempotent
        assert_eq!(s.list().unwrap().len(), 1);
    }

    #[test]
    fn rejects_bad_input() {
        let s = store();
        assert!(s.create("   ").is_err());
        assert!(s.create(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
        let mut p = s.create("ok").unwrap();
        p.accent = "green".into();
        assert!(s.save(&p).is_err());
    }

    #[test]
    fn profiles_saved_before_backgrounds_existed_still_load() {
        let s = store();
        let id = Uuid::new_v4();
        std::fs::create_dir_all(s.profile_dir(id)).unwrap();
        let old = format!(r##"{{"id":"{id}","name":"Old","accent":"#107C10"}}"##);
        std::fs::write(s.profile_dir(id).join("profile.json"), old).unwrap();
        let p = s.list().unwrap().pop().unwrap();
        assert_eq!((p.background, p.avatar), (Background::Default, Avatar::default()));
    }

    #[test]
    fn corrupt_profile_is_skipped() {
        let s = store();
        s.create("good").unwrap();
        let bad = s.dir().join("not-a-profile");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("profile.json"), b"garbage").unwrap();
        assert_eq!(s.list().unwrap().len(), 1);
    }
}
