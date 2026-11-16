//! Image loading off the UI thread, downscaled to display size to keep memory low.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use image::{DynamicImage, ImageFormat, ImageReader, imageops::FilterType};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// How large an image is decoded, by where it's shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Size {
    /// Square and box-art tiles (largest is 390 design px).
    Tile,
    /// Wide Home tiles (416×216 design px).
    Wide,
    /// Full-screen backgrounds.
    Backdrop,
}

impl Size {
    fn max(self) -> (u32, u32) {
        match self {
            Size::Tile => (400, 600),
            Size::Wide => (832, 432),
            Size::Backdrop => (1280, 720),
        }
    }

    /// Decoded images of this size kept before the oldest are dropped.
    fn limit(self) -> usize {
        match self {
            Size::Tile => 400,
            Size::Wide => 24,
            Size::Backdrop => 8,
        }
    }
}

/// Stored custom artwork is downscaled to this before saving.
pub const STORED_COVER_MAX: (u32, u32) = (600, 900);
pub const STORED_BACKGROUND_MAX: (u32, u32) = (1920, 1080);
pub const AVATAR_SIZE: u32 = 256;

pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;
type Key = (PathBuf, Size);

/// Caches decoded images; misses are queued to a background decoder thread.
pub struct ImageCache {
    tx: mpsc::Sender<Key>,
    pending: HashSet<Key>,
    /// `None` = failed to load (don't retry). The `u64` is a use counter for eviction.
    cache: HashMap<Key, (Option<Image>, u64)>,
    tick: u64,
}

impl ImageCache {
    /// `on_loaded` runs on the decoder thread; it should post the result to the UI thread.
    /// Images given as `http(s)://` URLs are downloaded once into `download_dir`.
    pub fn new(
        download_dir: PathBuf,
        on_loaded: impl Fn(PathBuf, Size, Option<Pixels>) + Send + Sync + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Key>();
        // A few decoder threads share the queue so a big library fills in quickly.
        let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
        let on_loaded = std::sync::Arc::new(on_loaded);
        for n in 0..3 {
            let (rx, on_loaded, download_dir) = (rx.clone(), on_loaded.clone(), download_dir.clone());
            std::thread::Builder::new()
                .name(format!("image-decoder-{n}"))
                .spawn(move || {
                    loop {
                        let Ok((path, size)) = rx.lock().unwrap().recv() else { break };
                        let local = match fetch(&path, &download_dir) {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!("image {}: {e}", path.display());
                                on_loaded(path, size, None);
                                continue;
                            }
                        };
                        let pixels = decode_fit(&local, size.max())
                            .inspect_err(|e| tracing::warn!("image {}: {e}", path.display()))
                            .ok()
                            .map(to_pixels);
                        on_loaded(path, size, pixels);
                    }
                })
                .expect("spawn image decoder");
        }
        Self { tx, pending: HashSet::new(), cache: HashMap::new(), tick: 0 }
    }

    /// The image if already decoded; otherwise queues it and returns `None`.
    pub fn get(&mut self, path: &Path, size: Size) -> Option<Image> {
        self.tick += 1;
        let key = (path.to_path_buf(), size);
        if let Some((img, used)) = self.cache.get_mut(&key) {
            *used = self.tick;
            return img.clone();
        }
        if self.pending.insert(key.clone()) {
            let _ = self.tx.send(key);
        }
        None
    }

    /// Stores a decoded result (UI thread). Evicts the least recently used image of that size if full.
    /// Cached image without requesting a load (for tiles that are off screen).
    pub fn peek(&self, path: &Path, size: Size) -> Option<Image> {
        self.cache.get(&(path.to_path_buf(), size)).and_then(|(img, _)| img.clone())
    }

    pub fn insert(&mut self, path: PathBuf, size: Size, pixels: Option<Pixels>) {
        let key = (path, size);
        self.pending.remove(&key);
        if self.cache.keys().filter(|(_, s)| *s == size).count() >= size.limit()
            && let Some(oldest) = self
                .cache
                .iter()
                .filter(|((_, s), _)| *s == size)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
        {
            self.cache.remove(&oldest);
        }
        self.tick += 1;
        self.cache.insert(key, (pixels.map(Image::from_rgba8), self.tick));
    }

    /// Drops cached entries for a file that was replaced.
    pub fn invalidate(&mut self, path: &Path) {
        self.cache.retain(|(p, _), _| p != path);
    }
}

/// Returns a local file for `path`, downloading it first if it's an http(s) URL.
fn fetch(path: &Path, dir: &Path) -> Result<PathBuf, String> {
    let s = path.to_string_lossy();
    if !(s.starts_with("https://") || s.starts_with("http://")) {
        return Ok(path.to_path_buf());
    }
    use sha2::{Digest, Sha256};
    let name: String = Sha256::digest(s.as_bytes()).iter().take(16).map(|b| format!("{b:02x}")).collect();
    let file = dir.join(name);
    if file.is_file() {
        return Ok(file);
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let bytes = ureq::get(&*s)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(20 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    let tmp = file.with_extension("part");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
    Ok(file)
}

fn decode(path: &Path) -> image::ImageResult<DynamicImage> {
    ImageReader::open(path)?.with_guessed_format()?.decode()
}

/// Decodes and shrinks (never enlarges) to fit within `max`, keeping aspect ratio.
pub fn decode_fit(path: &Path, max: (u32, u32)) -> image::ImageResult<DynamicImage> {
    let img = decode(path)?;
    Ok(if img.width() > max.0 || img.height() > max.1 { img.thumbnail(max.0, max.1) } else { img })
}

fn to_pixels(img: DynamicImage) -> Pixels {
    let rgba = img.into_rgba8();
    SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height())
}

/// Copies a user-chosen image into Win2Console's data folder as a downscaled PNG.
pub fn store_image(src: &Path, dst: &Path, max: (u32, u32)) -> image::ImageResult<()> {
    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir)?;
    }
    decode_fit(src, max)?.save_with_format(dst, ImageFormat::Png)
}

/// Downscaled PNG bytes of an image (for profile backgrounds, written by the profile store).
pub fn png_fit(src: &Path, max: (u32, u32)) -> image::ImageResult<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    decode_fit(src, max)?.write_to(&mut out, ImageFormat::Png)?;
    Ok(out.into_inner())
}

/// Center-crops to a square avatar and encodes it as PNG.
pub fn avatar_png(src: &Path) -> image::ImageResult<Vec<u8>> {
    let img = decode(src)?.resize_to_fill(AVATAR_SIZE, AVATAR_SIZE, FilterType::Triangle);
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)?;
    Ok(out.into_inner())
}
