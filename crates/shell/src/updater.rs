//! Updates through Velopack and GitHub Releases. Installed copies check github.com/Mythorical/Win2Console;
//! development builds (not installed by the Setup program) can't update and say so.
//!
//! Automatic mode downloads in the background and installs when Win2Console closes; manual mode only
//! reports that an update exists until you choose to install it from Settings.

use std::path::{Path, PathBuf};

use velopack::sources::GithubSource;
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, VelopackApp};

pub const REPO: &str = "https://github.com/Mythorical/Win2Console";

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Not an installed copy (e.g. a development build).
    Unavailable,
    Idle,
    Checking,
    UpToDate,
    Available(String),
    Downloading(String),
    /// Downloaded; installs when Win2Console closes (or now, from Settings).
    Ready(String),
    Failed(String),
}

impl State {
    pub fn label(&self) -> String {
        match self {
            State::Unavailable => "Only installed copies can update".into(),
            State::Idle => format!("Version {}", env!("CARGO_PKG_VERSION")),
            State::Checking => "Checking for updates…".into(),
            State::UpToDate => format!("Up to date ({})", env!("CARGO_PKG_VERSION")),
            State::Available(v) => format!("Version {v} is available"),
            State::Downloading(v) => format!("Downloading version {v}…"),
            State::Ready(v) => format!("Version {v} installs when you close Win2Console"),
            State::Failed(e) => format!("Couldn't check for updates: {e}"),
        }
    }
}

/// Must run first in `main`: Velopack calls the app with special arguments while installing/updating.
pub fn run_hooks() {
    VelopackApp::build()
        .on_after_install_fast_callback(|_| install_playnite_bridge())
        .on_after_update_fast_callback(|_| install_playnite_bridge())
        .on_before_uninstall_fast_callback(|_| {
            let _ = cs_platform::startup::set(None);
        })
        .run();
}

fn manager() -> Result<UpdateManager, String> {
    UpdateManager::new(GithubSource::new(REPO, None, false), None, None).map_err(|e| e.to_string())
}

/// Looks for an update; with `download`, also downloads it. Blocking: call off the UI thread.
pub fn check(download: bool) -> (State, Option<UpdateInfo>) {
    let um = match manager() {
        Ok(um) => um,
        Err(_) => return (State::Unavailable, None),
    };
    let info = match um.check_for_updates() {
        Ok(UpdateCheck::UpdateAvailable(info)) => *info,
        Ok(_) => return (State::UpToDate, None),
        Err(e) => return (State::Failed(e.to_string()), None),
    };
    let version = info.TargetFullRelease.Version.clone();
    if !download {
        return (State::Available(version), Some(info));
    }
    match um.download_updates(&info, None) {
        Ok(()) => {
            // Install when Win2Console exits (no restart, no window): never interrupts a session.
            if let Err(e) = um.wait_exit_then_apply_updates(&info, true, false, Vec::<String>::new()) {
                return (State::Failed(e.to_string()), Some(info));
            }
            (State::Ready(version), Some(info))
        }
        Err(e) => (State::Failed(e.to_string()), Some(info)),
    }
}

/// Downloads (if needed) and installs now, restarting Win2Console. Blocking: call off the UI thread.
pub fn install_now(info: &UpdateInfo) -> Result<(), String> {
    let um = manager()?;
    um.download_updates(info, None).map_err(|e| e.to_string())?;
    um.apply_updates_and_restart(info).map_err(|e| e.to_string())
}

/// Copies the bundled Playnite bridge into Playnite's extensions (when Playnite is installed).
pub fn install_playnite_bridge() {
    let Some(src) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("playnite-bridge"))) else {
        return;
    };
    let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) else { return };
    if !appdata.join("Playnite").is_dir() || !src.is_dir() {
        return;
    }
    let dst = appdata.join("Playnite").join("Extensions").join("Win2ConsoleBridge");
    if let Err(e) = copy_dir(&src, &dst) {
        eprintln!("installing the Playnite bridge: {e}");
    }
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
    }
    Ok(())
}
