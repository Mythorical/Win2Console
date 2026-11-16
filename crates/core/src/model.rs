use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where a game comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Steam,
    Epic,
    Gog,
    Xbox,
    Ea,
    Ubisoft,
    Amazon,
    Itch,
    Battlenet,
    /// Other Playnite sources (emulated games, manually added in Playnite, ...).
    Other,
    Custom,
}

impl Provider {
    pub const ALL: [Provider; 11] = [
        Provider::Steam,
        Provider::Epic,
        Provider::Gog,
        Provider::Xbox,
        Provider::Ea,
        Provider::Ubisoft,
        Provider::Amazon,
        Provider::Itch,
        Provider::Battlenet,
        Provider::Other,
        Provider::Custom,
    ];

    /// Stable key used in game ids, add-on ids and the database.
    pub fn key(self) -> &'static str {
        match self {
            Provider::Steam => "steam",
            Provider::Epic => "epic",
            Provider::Gog => "gog",
            Provider::Xbox => "xbox",
            Provider::Ea => "ea",
            Provider::Ubisoft => "ubisoft",
            Provider::Amazon => "amazon",
            Provider::Itch => "itch",
            Provider::Battlenet => "battlenet",
            Provider::Other => "playnite",
            Provider::Custom => "custom",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Provider::Steam => "Steam",
            Provider::Epic => "Epic Games",
            Provider::Gog => "GOG",
            Provider::Xbox => "Xbox",
            Provider::Ea => "EA",
            Provider::Ubisoft => "Ubisoft",
            Provider::Amazon => "Amazon Games",
            Provider::Itch => "itch.io",
            Provider::Battlenet => "Battle.net",
            Provider::Other => "Other",
            Provider::Custom => "Custom",
        }
    }
}

/// How to start a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LaunchSpec {
    /// Hand a URI to the OS (e.g. `steam://rungameid/440`); the owning launcher starts the game.
    Uri { uri: String },
    /// Start an executable directly.
    Exe {
        path: PathBuf,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        working_dir: Option<PathBuf>,
    },
}

/// Artwork paths. All optional; the UI falls back to a generated title tile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameMedia {
    pub cover: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    pub background: Option<PathBuf>,
}

/// A game in the library, shared by all profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    /// `<provider>:<provider-specific id>`, e.g. `steam:440`, `epic:Fortnite`, `custom:<uuid>`.
    pub id: String,
    pub provider: Provider,
    pub name: String,
    /// Used by the process tracker to find the running game. Required for URI launches.
    pub install_dir: Option<PathBuf>,
    pub launch: LaunchSpec,
    pub media: GameMedia,
}

impl Game {
    pub fn make_id(provider: Provider, native_id: &str) -> String {
        format!("{}:{}", provider.key(), native_id)
    }
}

/// Profile picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Avatar {
    /// Index into the shell's built-in avatar set.
    Builtin { index: u32 },
    /// `avatar.png` inside the profile folder.
    Custom,
}

impl Default for Avatar {
    fn default() -> Self {
        Avatar::Builtin { index: 0 }
    }
}

/// Home screen background, like Xbox's "My color & background".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Background {
    /// Generated wallpaper in the accent colour.
    #[default]
    Default,
    /// Flat accent-tinted colour.
    Solid,
    /// `background.png` inside the profile folder.
    Custom,
}

/// Online services a profile can link (game libraries and Discord).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Discord,
    Steam,
    Xbox,
    Epic,
    Gog,
    Ea,
    Ubisoft,
    Amazon,
    Itch,
    Battlenet,
}

impl Service {
    pub fn display_name(self) -> &'static str {
        match self {
            Service::Discord => "Discord",
            Service::Steam => "Steam",
            Service::Xbox => "Xbox",
            Service::Epic => "Epic Games",
            Service::Gog => "GOG",
            Service::Ea => "EA",
            Service::Ubisoft => "Ubisoft",
            Service::Amazon => "Amazon Games",
            Service::Itch => "itch.io",
            Service::Battlenet => "Battle.net",
        }
    }
}

/// An account on some service linked to a profile. Tokens are stored separately (encrypted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedAccount {
    pub service: Service,
    /// Service-side id, e.g. a Discord user id or SteamID64.
    pub account_id: String,
    pub display_name: String,
}

/// Settings of one library add-on installed on a profile (like Playnite's library extension settings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibrarySettings {
    pub import_installed: bool,
    pub import_uninstalled: bool,
    /// Extra per-library values (e.g. an API key), by field id.
    pub fields: std::collections::BTreeMap<String, String>,
}

impl Default for LibrarySettings {
    fn default() -> Self {
        Self { import_installed: true, import_uninstalled: true, fields: Default::default() }
    }
}

/// A user profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: Uuid,
    pub name: String,
    /// Accent colour as `#RRGGBB`.
    pub accent: String,
    #[serde(default)]
    pub avatar: Avatar,
    #[serde(default)]
    pub background: Background,
    #[serde(default)]
    pub accounts: Vec<LinkedAccount>,
    /// Installed library add-ons and their settings, by provider key.
    #[serde(default)]
    pub libraries: std::collections::BTreeMap<String, LibrarySettings>,
    /// Shown on the profile page.
    #[serde(default)]
    pub bio: String,
    /// Games shown on the profile page (game ids); empty shows the most played.
    #[serde(default)]
    pub showcase: Vec<String>,
}

/// Default accent (Xbox green).
pub const DEFAULT_ACCENT: &str = "#107C10";

/// Validates `#RRGGBB` and returns it normalised to upper case.
pub fn parse_accent(s: &str) -> Option<String> {
    let hex = s.strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", hex.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_keys_round_trip() {
        for p in Provider::ALL {
            assert_eq!(Provider::from_key(p.key()), Some(p));
        }
        assert_eq!(Provider::from_key("origin"), None);
    }

    #[test]
    fn accent_validation() {
        assert_eq!(parse_accent("#107c10").as_deref(), Some("#107C10"));
        assert_eq!(parse_accent("107C10"), None);
        assert_eq!(parse_accent("#12345"), None);
        assert_eq!(parse_accent("#GGGGGG"), None);
    }

    #[test]
    fn launch_spec_json_shape() {
        let spec = LaunchSpec::Uri { uri: "steam://rungameid/440".into() };
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(json, r#"{"kind":"uri","uri":"steam://rungameid/440"}"#);
        assert_eq!(serde_json::from_str::<LaunchSpec>(&json).unwrap(), spec);
    }
}
