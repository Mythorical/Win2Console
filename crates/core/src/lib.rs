//! Data model and persistent storage for Win2Console.
//!
//! Everything here is platform-independent so it can be unit-tested on any OS.
//! See `docs/ARCHITECTURE.md` for the on-disk layout.

pub mod config;
pub mod library;
pub mod model;
pub mod paths;
pub mod profiles;

mod fsutil;

pub use config::Config;
pub use library::{GameEntry, GameStats, Library};
pub use model::{
    Avatar, Background, Game, GameMedia, LaunchSpec, LibrarySettings, LinkedAccount, Profile, Provider, Service,
};
pub use paths::AppPaths;
pub use profiles::ProfileStore;

/// Current time as Unix seconds.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
