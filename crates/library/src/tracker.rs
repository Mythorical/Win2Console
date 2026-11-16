//! Detects when a launched game is running and when it exits, for playtime and "close game".
//!
//! - Exe launches: the game counts as running while the launched PID, or any process with the
//!   same executable path, is alive.
//! - URI launches (Steam, Epic): the launcher starts the game, so we wait (up to `start_timeout`)
//!   for any process whose executable is inside the game's install folder, then track until none remain.
//!
//! [`Tracker`] is a pure state machine; [`spawn`] drives it on a background thread.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cs_core::{Game, LaunchSpec};
use cs_platform::process::{ProcessSource, normalize_dir, normalize_path};

pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
pub const START_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackerEvent {
    /// The game's process was found.
    Started,
    /// The game exited after running for `secs`.
    Ended { secs: i64 },
    /// Nothing appeared within the start timeout (launch failed, or the launcher is showing an update/login).
    NotDetected,
    /// Tracking was cancelled (e.g. Win2Console is exiting). Playtime so far is reported.
    Cancelled { secs: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting { since: Instant },
    Running { since: Instant },
    Done,
}

/// What identifies the game's processes.
#[derive(Debug, Clone)]
pub struct Target {
    pid: Option<u32>,
    /// Normalised exact image path (exe launches).
    exe: Option<String>,
    /// Normalised folder prefix (URI launches).
    dir: Option<String>,
}

impl Target {
    pub fn for_game(game: &Game, launched_pid: Option<u32>) -> Self {
        match &game.launch {
            LaunchSpec::Exe { path, .. } => Self { pid: launched_pid, exe: Some(normalize_path(path)), dir: None },
            LaunchSpec::Uri { .. } => {
                Self { pid: launched_pid, exe: None, dir: game.install_dir.as_deref().map(normalize_dir) }
            }
        }
    }

    /// PIDs currently belonging to the game.
    pub fn pids(&self, procs: &dyn ProcessSource) -> Vec<u32> {
        let mut out: Vec<u32> = procs
            .processes()
            .into_iter()
            .filter(|(_, path)| {
                self.exe.as_deref().is_some_and(|e| path == e)
                    || self.dir.as_deref().is_some_and(|d| path.starts_with(d))
            })
            .map(|(pid, _)| pid)
            .collect();
        if let Some(pid) = self.pid.filter(|&p| !out.contains(&p) && procs.is_alive(p)) {
            out.push(pid);
        }
        out
    }
}

pub struct Tracker {
    target: Target,
    phase: Phase,
    start_timeout: Duration,
}

impl Tracker {
    pub fn new(target: Target, now: Instant, start_timeout: Duration) -> Self {
        Self { target, phase: Phase::Waiting { since: now }, start_timeout }
    }

    /// Advances the state machine; returns an event when the state changes.
    pub fn poll(&mut self, procs: &dyn ProcessSource, now: Instant) -> Option<TrackerEvent> {
        let alive = !self.target.pids(procs).is_empty();
        match self.phase {
            Phase::Waiting { .. } if alive => {
                self.phase = Phase::Running { since: now };
                Some(TrackerEvent::Started)
            }
            Phase::Waiting { since } if now.duration_since(since) >= self.start_timeout => {
                self.phase = Phase::Done;
                Some(TrackerEvent::NotDetected)
            }
            Phase::Running { since } if !alive => {
                self.phase = Phase::Done;
                Some(TrackerEvent::Ended { secs: now.duration_since(since).as_secs() as i64 })
            }
            _ => None,
        }
    }

    /// Ends tracking early, reporting playtime so far.
    pub fn cancel(&mut self, now: Instant) -> TrackerEvent {
        let secs = match self.phase {
            Phase::Running { since } => now.duration_since(since).as_secs() as i64,
            _ => 0,
        };
        self.phase = Phase::Done;
        TrackerEvent::Cancelled { secs }
    }

    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }

    pub fn target(&self) -> &Target {
        &self.target
    }
}

/// Handle to a background tracking thread.
pub struct TrackingHandle {
    cancel: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    target: Target,
}

impl TrackingHandle {
    /// PIDs of the running game (for "close game").
    pub fn pids(&self, procs: &dyn ProcessSource) -> Vec<u32> {
        self.target.pids(procs)
    }

    /// Stops tracking; the thread emits `Cancelled` and exits.
    pub fn cancel(mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Tracks `game` on a background thread, calling `on_event` for each state change.
pub fn spawn(
    game: &Game,
    launched_pid: Option<u32>,
    procs: Arc<dyn ProcessSource>,
    on_event: impl Fn(TrackerEvent) + Send + 'static,
) -> TrackingHandle {
    let target = Target::for_game(game, launched_pid);
    let cancel = Arc::new(AtomicBool::new(false));
    let thread = {
        let (target, cancel) = (target.clone(), cancel.clone());
        let name = game.name.clone();
        std::thread::Builder::new()
            .name(format!("tracker:{name}"))
            .spawn(move || {
                let mut tracker = Tracker::new(target, Instant::now(), START_TIMEOUT);
                while !tracker.is_done() {
                    if cancel.load(Ordering::Relaxed) {
                        on_event(tracker.cancel(Instant::now()));
                        break;
                    }
                    if let Some(ev) = tracker.poll(procs.as_ref(), Instant::now()) {
                        tracing::info!("tracker[{name}]: {ev:?}");
                        on_event(ev);
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
            })
            .expect("spawn tracker thread")
    };
    TrackingHandle { cancel, thread: Some(thread), target }
}

/// Whether `path` (an exe) is inside `dir`. Exposed for the UI's "is this game running" checks.
pub fn is_inside(path: &Path, dir: &Path) -> bool {
    normalize_path(path).starts_with(&normalize_dir(dir))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use cs_core::{GameMedia, Provider};

    use super::*;

    #[derive(Default)]
    struct FakeProcs(Mutex<Vec<(u32, String)>>);

    impl FakeProcs {
        fn set(&self, procs: &[(u32, &str)]) {
            *self.0.lock().unwrap() = procs.iter().map(|&(p, s)| (p, s.to_lowercase())).collect();
        }
    }

    impl ProcessSource for FakeProcs {
        fn processes(&self) -> Vec<(u32, String)> {
            self.0.lock().unwrap().clone()
        }
        fn is_alive(&self, pid: u32) -> bool {
            self.0.lock().unwrap().iter().any(|(p, _)| *p == pid)
        }
    }

    fn uri_game() -> Game {
        Game {
            id: "steam:440".into(),
            provider: Provider::Steam,
            name: "TF2".into(),
            install_dir: Some(PathBuf::from(r"C:\Steam\steamapps\common\Team Fortress 2")),
            launch: LaunchSpec::Uri { uri: "steam://rungameid/440".into() },
            media: GameMedia::default(),
        }
    }

    fn exe_game() -> Game {
        Game {
            id: "custom:1".into(),
            provider: Provider::Custom,
            name: "Charmap".into(),
            install_dir: Some(PathBuf::from(r"C:\Windows\System32")),
            launch: LaunchSpec::Exe {
                path: r"C:\Windows\System32\charmap.exe".into(),
                args: vec![],
                working_dir: None,
            },
            media: GameMedia::default(),
        }
    }

    #[test]
    fn uri_game_lifecycle() {
        let procs = FakeProcs::default();
        let t0 = Instant::now();
        let mut tr = Tracker::new(Target::for_game(&uri_game(), None), t0, START_TIMEOUT);

        procs.set(&[(10, r"C:\Program Files (x86)\Steam\steam.exe")]);
        assert_eq!(tr.poll(&procs, t0 + Duration::from_secs(5)), None);

        procs.set(&[(10, "c:/x/steam.exe"), (20, r"C:\Steam\steamapps\common\Team Fortress 2\tf_win64.exe")]);
        assert_eq!(tr.poll(&procs, t0 + Duration::from_secs(10)), Some(TrackerEvent::Started));
        assert_eq!(tr.poll(&procs, t0 + Duration::from_secs(70)), None);

        procs.set(&[(10, "c:/x/steam.exe")]);
        assert_eq!(tr.poll(&procs, t0 + Duration::from_secs(130)), Some(TrackerEvent::Ended { secs: 120 }));
        assert!(tr.is_done());
    }

    #[test]
    fn uri_game_not_detected() {
        let procs = FakeProcs::default();
        let t0 = Instant::now();
        let mut tr = Tracker::new(Target::for_game(&uri_game(), None), t0, Duration::from_secs(60));
        assert_eq!(tr.poll(&procs, t0 + Duration::from_secs(59)), None);
        assert_eq!(tr.poll(&procs, t0 + Duration::from_secs(60)), Some(TrackerEvent::NotDetected));
    }

    #[test]
    fn exe_game_ignores_other_processes_in_same_folder() {
        let procs = FakeProcs::default();
        let t0 = Instant::now();
        let mut tr = Tracker::new(Target::for_game(&exe_game(), Some(42)), t0, START_TIMEOUT);

        procs.set(&[(42, r"C:\Windows\System32\charmap.exe"), (7, r"C:\Windows\System32\svchost.exe")]);
        assert_eq!(tr.poll(&procs, t0), Some(TrackerEvent::Started));
        assert_eq!(tr.target().pids(&procs), [42]);

        // Launched PID gone but svchost (same folder) still running: the game has ended.
        procs.set(&[(7, r"C:\Windows\System32\svchost.exe")]);
        assert_eq!(tr.poll(&procs, t0 + Duration::from_secs(3)), Some(TrackerEvent::Ended { secs: 3 }));
    }

    #[test]
    fn cancel_reports_partial_playtime() {
        let procs = FakeProcs::default();
        let t0 = Instant::now();
        let mut tr = Tracker::new(Target::for_game(&exe_game(), Some(42)), t0, START_TIMEOUT);
        procs.set(&[(42, r"C:\Windows\System32\charmap.exe")]);
        tr.poll(&procs, t0);
        assert_eq!(tr.cancel(t0 + Duration::from_secs(9)), TrackerEvent::Cancelled { secs: 9 });
        assert!(tr.is_done());
    }

    #[test]
    fn inside_check() {
        assert!(is_inside(Path::new("C:/Games/X/bin/x.exe"), Path::new(r"c:\games\x")));
        assert!(!is_inside(Path::new("C:/Games/X2/x.exe"), Path::new(r"C:\Games\X")));
    }
}
