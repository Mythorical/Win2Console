//! Games the user adds by hand (any executable).

use std::path::PathBuf;

use cs_core::{Game, GameMedia, LaunchSpec, Provider};

/// Builds a custom game. The install dir is the exe's folder, so the process tracker can also follow
/// games whose exe is a small launcher that starts the real game.
pub fn new_custom_game(name: &str, exe: PathBuf, args: Vec<String>, working_dir: Option<PathBuf>) -> Game {
    let id = Game::make_id(Provider::Custom, &uuid_v4());
    Game {
        id,
        provider: Provider::Custom,
        name: name.trim().to_owned(),
        install_dir: exe.parent().map(PathBuf::from),
        launch: LaunchSpec::Exe { path: exe, args, working_dir },
        media: GameMedia::default(),
    }
}

/// Default display name for an exe: its file stem (`C:\Games\Celeste\Celeste.exe` → `Celeste`).
pub fn default_name(exe: &std::path::Path) -> String {
    exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_custom_game() {
        let g = new_custom_game(" Celeste ", PathBuf::from("C:/Games/Celeste/Celeste.exe"), vec![], None);
        assert!(g.id.starts_with("custom:"));
        assert_eq!(g.name, "Celeste");
        assert_eq!(g.install_dir, Some(PathBuf::from("C:/Games/Celeste")));
        assert_eq!(default_name(std::path::Path::new("C:/Games/Celeste/Celeste.exe")), "Celeste");
    }
}
