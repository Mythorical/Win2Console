//! Volume control through Windows Core Audio: the default output device's master volume, and the volume
//! of a set of processes' audio sessions (the running game).

use std::io;

#[cfg(windows)]
mod imp {
    use std::io;

    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume,
        MMDeviceEnumerator, eConsole, eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows::core::Interface;

    fn err(e: windows::core::Error) -> io::Error {
        io::Error::other(e.message())
    }

    pub fn device() -> io::Result<IMMDevice> {
        // SAFETY: COM is initialised for this thread (repeated init is harmless), then plain COM calls.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(err)?;
            enumerator.GetDefaultAudioEndpoint(eRender, eConsole).map_err(err)
        }
    }

    pub fn endpoint() -> io::Result<IAudioEndpointVolume> {
        // SAFETY: COM activation on a valid device.
        unsafe { device()?.Activate(CLSCTX_ALL, None).map_err(err) }
    }

    /// Volume controls of every audio session belonging to one of `pids`.
    pub fn sessions(pids: &[u32]) -> io::Result<Vec<ISimpleAudioVolume>> {
        // SAFETY: COM calls on interfaces obtained above.
        unsafe {
            let manager: IAudioSessionManager2 = device()?.Activate(CLSCTX_ALL, None).map_err(err)?;
            let list = manager.GetSessionEnumerator().map_err(err)?;
            let mut out = Vec::new();
            for i in 0..list.GetCount().map_err(err)? {
                let Ok(control) = list.GetSession(i) else { continue };
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
                if control2.GetProcessId().is_ok_and(|pid| pids.contains(&pid))
                    && let Ok(volume) = control.cast::<ISimpleAudioVolume>()
                {
                    out.push(volume);
                }
            }
            Ok(out)
        }
    }
}

/// Master volume (0.0–1.0) and mute of the default output device.
#[cfg(windows)]
pub fn master() -> io::Result<(f32, bool)> {
    let v = imp::endpoint()?;
    // SAFETY: COM calls on a valid interface.
    unsafe {
        let level = v.GetMasterVolumeLevelScalar().map_err(|e| io::Error::other(e.message()))?;
        let muted = v.GetMute().map_err(|e| io::Error::other(e.message()))?.as_bool();
        Ok((level, muted))
    }
}

#[cfg(windows)]
pub fn set_master(level: f32, muted: bool) -> io::Result<()> {
    let v = imp::endpoint()?;
    // SAFETY: COM calls on a valid interface; no event context GUID.
    unsafe {
        v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null())
            .map_err(|e| io::Error::other(e.message()))?;
        v.SetMute(muted, std::ptr::null()).map_err(|e| io::Error::other(e.message()))
    }
}

/// Volume (0.0–1.0) and mute of the given processes' audio, or `None` if they have no audio session yet.
#[cfg(windows)]
pub fn app(pids: &[u32]) -> io::Result<Option<(f32, bool)>> {
    let sessions = imp::sessions(pids)?;
    let Some(first) = sessions.first() else { return Ok(None) };
    // SAFETY: COM calls on a valid interface.
    unsafe {
        let level = first.GetMasterVolume().map_err(|e| io::Error::other(e.message()))?;
        let muted = first.GetMute().map_err(|e| io::Error::other(e.message()))?.as_bool();
        Ok(Some((level, muted)))
    }
}

#[cfg(windows)]
pub fn set_app(pids: &[u32], level: f32, muted: bool) -> io::Result<()> {
    for s in imp::sessions(pids)? {
        // SAFETY: COM calls on a valid interface.
        unsafe {
            s.SetMasterVolume(level.clamp(0.0, 1.0), std::ptr::null()).map_err(|e| io::Error::other(e.message()))?;
            s.SetMute(muted, std::ptr::null()).map_err(|e| io::Error::other(e.message()))?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn master() -> io::Result<(f32, bool)> {
    Err(crate::unsupported())
}
#[cfg(not(windows))]
pub fn set_master(_level: f32, _muted: bool) -> io::Result<()> {
    Err(crate::unsupported())
}
#[cfg(not(windows))]
pub fn app(_pids: &[u32]) -> io::Result<Option<(f32, bool)>> {
    Err(crate::unsupported())
}
#[cfg(not(windows))]
pub fn set_app(_pids: &[u32], _level: f32, _muted: bool) -> io::Result<()> {
    Err(crate::unsupported())
}
