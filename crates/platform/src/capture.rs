//! Screen capture for screenshots and the in-game Guide's backdrop (primary monitor, via GDI).
//! Works for windowed and borderless games; exclusive-fullscreen games may capture as black.

use std::io;

/// A captured image, RGBA8, top-down.
pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[cfg(windows)]
pub fn primary_screen() -> io::Result<Capture> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    // SAFETY: GDI objects are created, used and released in order below.
    unsafe {
        let (w, h) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        if w <= 0 || h <= 0 {
            return Err(io::Error::other("no screen"));
        }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bitmap.into());
        let copied = BitBlt(mem, 0, 0, w, h, Some(screen), 0, 0, SRCCOPY | CAPTUREBLT);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(mem, bitmap, 0, h as u32, Some(bgra.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        copied.map_err(|e| io::Error::other(e.to_string()))?;
        if lines == 0 {
            return Err(io::Error::other("couldn't read the screen"));
        }
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        Ok(Capture { width: w as u32, height: h as u32, rgba: bgra })
    }
}

#[cfg(not(windows))]
pub fn primary_screen() -> io::Result<Capture> {
    Err(crate::unsupported())
}
