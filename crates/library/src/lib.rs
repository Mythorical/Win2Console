//! Game library providers and the running-game tracker.

pub mod accounts;
pub mod achievements;
pub mod custom;
pub mod playnite;
pub mod steam;
pub mod tracker;
pub mod vdf;

use std::io;

use cs_core::{Game, Provider};

/// A source of installed games. Launching is generic (`cs_platform::launch` on the game's `LaunchSpec`),
/// so providers only discover games.
pub trait LibraryProvider: Send + Sync {
    fn provider(&self) -> Provider;
    /// Whether the launcher appears to be installed.
    fn is_available(&self) -> bool;
    /// Installed games. Individual unreadable entries are logged and skipped, never fatal.
    fn scan(&self) -> io::Result<Vec<Game>>;
}

/// Percent-encodes a URL query value.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
