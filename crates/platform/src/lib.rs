//! Thin wrappers over the Windows APIs Win2Console needs.
//!
//! Every function has a non-Windows fallback (usually a no-op or `Unsupported` error) so the crates
//! that depend on this one still build and unit-test on Linux.

pub mod audio;
pub mod capture;
pub mod power;
pub mod process;
pub mod registry;
pub mod secret;
pub mod sound;
pub mod startup;
pub mod time;
pub mod window;

#[cfg(windows)]
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(not(windows))]
pub(crate) fn unsupported() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, "only supported on Windows")
}
