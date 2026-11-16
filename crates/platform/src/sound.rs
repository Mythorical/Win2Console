//! UI sounds (in-memory WAV through `PlaySound`; a new sound replaces the previous one) and one looping
//! background track through MCI (plays alongside the UI sounds).

#[cfg(windows)]
pub fn play_wav(wav: &'static [u8]) {
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    use windows::core::PCWSTR;
    // SAFETY: the WAV data is 'static, so it outlives the asynchronous playback.
    unsafe {
        let _ = PlaySoundW(PCWSTR(wav.as_ptr().cast()), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

#[cfg(windows)]
fn mci(command: &str) -> u32 {
    use windows::Win32::Media::Multimedia::mciSendStringW;
    use windows::core::PCWSTR;
    let wide = crate::wide(command);
    // SAFETY: valid null-terminated command; no return buffer or callback window.
    unsafe { mciSendStringW(PCWSTR(wide.as_ptr()), None, None) }
}

/// Opens (if needed) and loops `path` (wav or mp3) at `volume` (0–1000).
#[cfg(windows)]
pub fn music_play(path: &std::path::Path, volume: u32) {
    if mci("status cs_bgm mode") != 0 {
        let err = mci(&format!("open \"{}\" type mpegvideo alias cs_bgm", path.display()));
        if err != 0 {
            tracing::warn!("couldn't open background music (MCI error {err})");
            return;
        }
    }
    mci(&format!("setaudio cs_bgm volume to {volume}"));
    mci("play cs_bgm repeat");
}

#[cfg(windows)]
pub fn music_pause() {
    mci("pause cs_bgm");
}

#[cfg(not(windows))]
pub fn play_wav(_wav: &'static [u8]) {}
#[cfg(not(windows))]
pub fn music_play(_path: &std::path::Path, _volume: u32) {}
#[cfg(not(windows))]
pub fn music_pause() {}
