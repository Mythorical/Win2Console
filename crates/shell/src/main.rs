//! Win2Console entry point: logging, storage, window, controller thread, clock.

// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod discord;
mod feeds;
mod files;
mod gifs;
mod images;
mod nav;
mod osk;
mod social;
mod sounds;
mod splash;
mod updater;
mod video;
mod weblogin;

use std::cell::RefCell;
use std::time::Duration;

use slint::ComponentHandle;

use crate::app::App;

pub mod ui {
    slint::include_modules!();
}

thread_local! {
    /// The app lives on the UI thread; background threads reach it through `app::post`.
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` with the app (UI thread only). Returns `None` before startup or if already borrowed.
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

fn main() {
    // Velopack runs install/update steps through the app itself; this returns straight away otherwise.
    updater::run_hooks();
    let paths = match cs_core::AppPaths::discover() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Win2Console: cannot find the user data folder: {e}");
            std::process::exit(1);
        }
    };
    let _ = paths.ensure();
    let _log_guard = init_logging(&paths);
    tracing::info!("Win2Console {} starting, data in {}", env!("CARGO_PKG_VERSION"), paths.root.display());

    if let Err(e) = run(paths) {
        tracing::error!("fatal: {e}");
        std::process::exit(1);
    }
}

fn run(paths: cs_core::AppPaths) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ui::AppWindow::new()?;
    let app = App::new(&ui, paths)?;
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    ui.on_key(|text| with_app(|app| app.on_key(&text)).unwrap_or(false));
    ui.on_pointer(|area, row, col, activate| {
        with_app(|app| app.on_pointer(&area, row, col, activate));
    });

    // Controller thread → UI thread.
    cs_input::spawn(|event| {
        // Mouse mode (on the desktop) drives the pointer straight from the controller thread.
        if let cs_input::InputEvent::Mouse(m) = event {
            use cs_platform::input;
            match m {
                cs_input::MouseEvent::Move(dx, dy) => input::mouse_move(dx, dy),
                cs_input::MouseEvent::Button { right, down } => input::mouse_button(right, down),
                cs_input::MouseEvent::Wheel(v, h) => input::wheel(v, h),
            }
            return;
        }
        app::post(move |app| match event {
            cs_input::InputEvent::Nav(action) => {
                // Using the controller hides the mouse pointer until the mouse moves again.
                app.set_cursor_hidden(true);
                app.on_nav(action);
            }
            cs_input::InputEvent::Controllers(list) => app.on_controllers(list),
            cs_input::InputEvent::Mouse(_) => {}
        });
    });

    // Moving the mouse brings the pointer back (keyboard input leaves it alone); focus pauses the music.
    {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
        // Windows also reports "moves" when nothing moved (e.g. after hiding the pointer): compare positions.
        let last = std::cell::Cell::new((f64::NAN, f64::NAN));
        ui.window().on_winit_window_event(move |_, event| {
            if let WindowEvent::Focused(focused) = *event {
                app::post(move |app| app.set_window_focused(focused));
            }
            if let WindowEvent::CursorMoved { position, .. } = event {
                let (x, y) = last.replace((position.x, position.y));
                if !x.is_nan() && ((position.x - x).abs() > 1.0 || (position.y - y).abs() > 1.0) {
                    app::post(|app| app.set_cursor_hidden(false));
                }
            }
            EventResult::Propagate
        });
    }

    // Ctrl+Shift+G (and Home while playing) opens the Guide from the keyboard, even while a game has focus.
    let hotkey = cs_platform::window::spawn_guide_hotkey(|| {
        app::post(|app| {
            app.on_nav(cs_input::NavAction::Guide);
        })
    });
    with_app(|app| app.hotkey = Some(hotkey));

    // Clock (updated every few seconds; cheap).
    let clock = slint::Timer::default();
    clock.start(slint::TimerMode::Repeated, Duration::from_secs(5), || {
        with_app(|app| app.tick_clock());
    });

    with_app(|app| {
        app.tick_clock();
        app.render();
        app.apply_branding();
        app.load_feeds();
        app.startup_update_check();
        app.refresh_start_with_windows();
        crate::sounds::write_index();
        app.start_achievement_watch();
        app.start_splash();
    });

    // Fullscreen unless WIN2CONSOLE_WINDOWED is set (handy for development).
    if std::env::var_os("WIN2CONSOLE_WINDOWED").is_none() {
        ui.window().set_fullscreen(true);
    }
    ui.run()?;

    with_app(|app| app.stop_tracking());
    APP.with(|cell| cell.borrow_mut().take());
    tracing::info!("Win2Console exiting");
    Ok(())
}

/// Daily rolling log files in `%APPDATA%\Win2Console\logs`, plus stderr in debug builds.
fn init_logging(paths: &cs_core::AppPaths) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::prelude::*;
    let file = tracing_appender::rolling::daily(paths.logs_dir(), "win2console.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    // WIN2CONSOLE_LOG=debug logs every button press and Guide action.
    let filter = match std::env::var("WIN2CONSOLE_LOG").as_deref() {
        Ok("debug") => tracing_subscriber::filter::LevelFilter::DEBUG,
        _ => tracing_subscriber::filter::LevelFilter::INFO,
    };
    let registry = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(writer).with_ansi(false).with_filter(filter));
    if cfg!(debug_assertions) {
        registry.with(tracing_subscriber::fmt::layer().with_filter(filter)).init();
    } else {
        registry.init();
    }
    guard
}
