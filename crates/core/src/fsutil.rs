use std::io::{self, Write};
use std::path::Path;

/// Writes `bytes` to `path` atomically: write a sibling temp file, flush, then rename over the target.
/// A crash mid-write leaves the previous file intact.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}
