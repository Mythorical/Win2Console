use std::io;
use std::path::{Path, PathBuf};

/// Locations of everything Win2Console stores on disk.
///
/// The root is `%APPDATA%\Win2Console` unless the `WIN2CONSOLE_HOME` environment variable is set
/// (used by tests and portable installs).
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    /// Resolves the default root (see type docs).
    pub fn discover() -> io::Result<Self> {
        if let Some(home) = std::env::var_os("WIN2CONSOLE_HOME") {
            return Ok(Self::at(home));
        }
        let base = directories::BaseDirs::new()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user data directory"))?;
        let root = base.config_dir().join("Win2Console");
        // The project used to be called ConsoleShell: carry its data over on first launch.
        adopt_old_dir(&base.config_dir().join("ConsoleShell"), &root);
        Ok(Self::at(root))
    }

    /// Uses `root` as-is.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Creates the root and all subdirectories if missing.
    pub fn ensure(&self) -> io::Result<()> {
        for dir in [self.root.clone(), self.profiles_dir(), self.media_dir(), self.logs_dir()] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub fn library_db(&self) -> PathBuf {
        self.root.join("library.db")
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub fn media_dir(&self) -> PathBuf {
        self.root.join("media")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Media folder for one game. Game ids contain `:`, which Windows forbids in file names.
    pub fn game_media_dir(&self, game_id: &str) -> PathBuf {
        self.media_dir().join(sanitize(game_id))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Replaces characters that are invalid in Windows file names.
pub fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_reserved_chars() {
        assert_eq!(sanitize("steam:440"), "steam_440");
        assert_eq!(sanitize("a/b\\c*d?"), "a_b_c_d_");
    }
}

/// Moves `old` to `new` if `new` doesn't exist yet (data from before the rename to Win2Console).
pub fn adopt_old_dir(old: &std::path::Path, new: &std::path::Path) {
    if !new.exists()
        && old.is_dir()
        && let Err(e) = std::fs::rename(old, new)
    {
        eprintln!("couldn't move {} to {}: {e}", old.display(), new.display());
    }
}
