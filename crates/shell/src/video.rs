//! Custom startup video (`%APPDATA%\Win2Console\startup.mp4` or `.webm`), played full-window in a
//! WebView2 child view; it reports when playback ends (or fails) so the dashboard can take over.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::ComponentHandle;
use wry::{WebContext, WebView, WebViewBuilder, dpi};

pub struct StartupVideo {
    view: Rc<WebView>,
    _context: WebContext,
    _poll: slint::Timer,
}

/// The user's startup video, if there is one.
pub fn find(root: &Path) -> Option<PathBuf> {
    ["startup.mp4", "startup.webm"].into_iter().map(|f| root.join(f)).find(|p| p.is_file())
}

impl StartupVideo {
    pub fn open(
        window: &slint::Window,
        cache: &Path,
        video: &Path,
        on_end: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        // A tiny page next to the cache so it can load the file:// video.
        let url = |p: &Path| format!("file:///{}", p.display().to_string().replace('\\', "/").replace(' ', "%20"));
        let page = cache.join("startup.html");
        std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
        std::fs::write(
            &page,
            format!(
                r#"<!doctype html><html><body style="margin:0;background:#000;overflow:hidden" onclick="window.skip=true">
<video src="{}" autoplay playsinline style="width:100vw;height:100vh;object-fit:cover"></video></body></html>"#,
                url(video)
            ),
        )
        .map_err(|e| e.to_string())?;
        let mut context = WebContext::new(Some(cache.join("webview-video")));
        let size = window.size().to_logical(window.scale_factor());
        let view = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(url(&page))
            .with_focused(false)
            .with_devtools(false)
            .with_autoplay(true)
            .with_background_color((0, 0, 0, 255))
            .with_bounds(wry::Rect {
                position: dpi::LogicalPosition::new(0.0, 0.0).into(),
                size: dpi::LogicalSize::new(size.width as f64, size.height as f64).into(),
            })
            .build_as_child(&window.window_handle())
            .map_err(|e| e.to_string())?;
        let _ = view.focus_parent();
        // Page-to-app messages don't arrive from this view, so ask the page whether it's done instead.
        let view = Rc::new(view);
        let on_end = Arc::new(on_end);
        let poll = slint::Timer::default();
        let polled = view.clone();
        poll.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || {
            let on_end = on_end.clone();
            let _ = polled.evaluate_script_with_callback(
                "(function(){var v=document.querySelector('video');return !!(window.skip||!v||v.ended||v.error)})()",
                move |done| {
                    if done == "true" {
                        on_end();
                    }
                },
            );
        });
        Ok(Self { view, _context: context, _poll: poll })
    }
}

/// The startup video in its own full-screen window on top of Win2Console's.
pub struct StartupWindow {
    window: crate::ui::VideoWindow,
    video: Option<StartupVideo>,
}

impl StartupWindow {
    pub fn new() -> Result<Self, String> {
        let window = crate::ui::VideoWindow::new().map_err(|e| e.to_string())?;
        // Keys typed while the video has focus (Enter skips it) go to the app as usual.
        window.on_key(|text| crate::with_app(|app| app.on_key(&text)).unwrap_or(false));
        window.window().set_fullscreen(true);
        window.show().map_err(|e| e.to_string())?;
        Ok(Self { window, video: None })
    }

    /// Starts the video once the window exists and is full screen (call until it returns true).
    pub fn attach(&mut self, cache: &Path, video: &Path, on_end: impl Fn() + Send + Sync + 'static) -> Result<bool, String> {
        if self.video.is_some() {
            return Ok(true);
        }
        if self.window.window().size().width < 400 {
            return Ok(false);
        }
        self.video = Some(StartupVideo::open(self.window.window(), cache, video, on_end)?);
        Ok(true)
    }
}

impl StartupWindow {
    /// Fades the video to black over `secs`.
    pub fn fade_out(&self, secs: f32) {
        if let Some(v) = &self.video {
            let ms = (secs * 1000.0) as u32;
            let _ = v.view.evaluate_script(&format!(
                "var v=document.querySelector('video');v.style.transition='opacity {ms}ms ease-in';v.style.opacity=0;"
            ));
        }
    }
}

impl Drop for StartupWindow {
    fn drop(&mut self) {
        self.video = None;
        let _ = self.window.hide();
    }
}

impl Drop for StartupVideo {
    fn drop(&mut self) {
        let _ = self.view.set_visible(false);
    }
}
