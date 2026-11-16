//! GIFs and images linked in chat messages: Tenor / Giphy pages (resolved through their `og:image`) and
//! direct image links (e.g. Discord's CDN). Decoded off the UI thread, shrunk, and animated by frame time.

use std::collections::HashMap;
use std::time::Instant;

use image::{AnimationDecoder, RgbaImage};

/// Largest size a chat image is decoded at (design pixels).
const MAX: u32 = 320;
const MAX_FRAMES: usize = 200;

enum Entry {
    Loading,
    Failed,
    /// Frames with their end time (ms from the start of the loop).
    Ready(Vec<(slint::Image, u32)>),
}

pub struct Gifs {
    cache: HashMap<String, Entry>,
    epoch: Instant,
}

/// The first image / GIF link in `text`, and the text without it.
pub fn find_link(text: &str) -> Option<(String, String)> {
    let url = text.split_whitespace().find(|w| is_media_link(w))?;
    let rest = text.replacen(url, "", 1).trim().to_string();
    Some((url.to_string(), rest))
}

fn is_media_link(word: &str) -> bool {
    let Some(rest) = word.strip_prefix("https://").or_else(|| word.strip_prefix("http://")) else {
        return false;
    };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = path.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    (host.ends_with("tenor.com") && path.starts_with("view/"))
        || (host.ends_with("giphy.com") && (path.starts_with("gifs/") || path.starts_with("media/")))
        || [".gif", ".png", ".jpg", ".jpeg", ".webp"].iter().any(|ext| path.ends_with(ext))
}

impl Gifs {
    pub fn new() -> Self {
        Self { cache: HashMap::new(), epoch: Instant::now() }
    }

    /// The current frame for `url`; starts loading it the first time (`None` until it's ready or if it failed).
    pub fn frame(&mut self, url: &str) -> Option<slint::Image> {
        match self.cache.get(url) {
            Some(Entry::Ready(frames)) => Some(pick(frames, self.epoch.elapsed().as_millis() as u64)),
            Some(_) => None,
            None => {
                self.cache.insert(url.to_string(), Entry::Loading);
                let url = url.to_string();
                std::thread::spawn(move || {
                    let result = fetch(&url);
                    if let Err(e) = &result {
                        tracing::warn!("chat image {url}: {e}");
                    }
                    crate::app::post(move |app| app.gif_loaded(url, result.ok()));
                });
                None
            }
        }
    }

    /// Whether `url` has more than one frame.
    pub fn animated(&self, url: &str) -> bool {
        matches!(self.cache.get(url), Some(Entry::Ready(f)) if f.len() > 1)
    }

    pub fn insert(&mut self, url: String, frames: Option<Vec<(RgbaImage, u32)>>) {
        let entry = match frames {
            Some(frames) if !frames.is_empty() => {
                let mut end = 0;
                Entry::Ready(
                    frames
                        .into_iter()
                        .map(|(img, delay)| {
                            end += delay;
                            let buf = slint::SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height());
                            (slint::Image::from_rgba8(buf), end)
                        })
                        .collect(),
                )
            }
            _ => Entry::Failed,
        };
        self.cache.insert(url, entry);
    }
}

fn pick(frames: &[(slint::Image, u32)], now_ms: u64) -> slint::Image {
    let total = frames.last().map_or(1, |f| f.1.max(1)) as u64;
    let t = (now_ms % total) as u32;
    frames.iter().find(|f| t < f.1).unwrap_or(&frames[0]).0.clone()
}

fn get(url: &str) -> Result<Vec<u8>, String> {
    ureq::get(url)
        .header("User-Agent", "Mozilla/5.0 Win2Console")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(20 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

/// Downloads and decodes `url` (following a page's `og:image`) into frames with their delays (ms).
fn fetch(url: &str) -> Result<Vec<(RgbaImage, u32)>, String> {
    let mut bytes = get(url)?;
    if bytes.iter().take(512).any(|&b| b == b'<') && !bytes.starts_with(b"GIF8") {
        let page = String::from_utf8_lossy(&bytes);
        let image = og_image(&page).ok_or("no image on the page")?;
        bytes = get(&image)?;
    }
    if bytes.starts_with(b"GIF8") {
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let mut frames = Vec::new();
        for frame in decoder.into_frames().take(MAX_FRAMES) {
            let frame = frame.map_err(|e| e.to_string())?;
            let (num, den) = frame.delay().numer_denom_ms();
            let ms = num / den.max(1);
            // Browsers treat tiny delays as 100 ms; so do we.
            let ms = if ms <= 10 { 100 } else { ms };
            frames.push((shrink(frame.into_buffer()), ms));
        }
        return Ok(frames);
    }
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    Ok(vec![(shrink(img.into_rgba8()), 1000)])
}

fn og_image(page: &str) -> Option<String> {
    page.split("<meta").find_map(|tag| {
        let tag = tag.split('>').next()?;
        if !(tag.contains("\"og:image\"") || tag.contains("\"og:image:url\"")) {
            return None;
        }
        let content = tag.split("content=\"").nth(1)?.split('"').next()?;
        Some(content.replace("&amp;", "&"))
    })
}

fn shrink(img: RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    if w <= MAX && h <= MAX {
        return img;
    }
    let k = MAX as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1));
    image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links() {
        let (url, rest) = find_link("lol https://tenor.com/view/cat-dance-gif-123").unwrap();
        assert_eq!(url, "https://tenor.com/view/cat-dance-gif-123");
        assert_eq!(rest, "lol");
        assert!(find_link("https://cdn.discordapp.com/attachments/1/2/a.GIF?ex=1").is_some());
        assert!(find_link("see https://example.com/page").is_none());
    }

    #[test]
    fn reads_og_image() {
        let page = r#"<meta class="dynamic" property="og:image" content="https://media1.tenor.com/m/x/cat.gif">"#;
        assert_eq!(og_image(page).as_deref(), Some("https://media1.tenor.com/m/x/cat.gif"));
    }
}
