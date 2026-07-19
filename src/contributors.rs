//! Contributors wall data (spec 00 §4.2): REST contributors endpoint,
//! bot filtering, concurrent avatar download with base64 embedding.
//!
//! Determinism note: avatar fetches use `buffered` (ordered) concurrency —
//! `buffer_unordered` would make wall order depend on network timing.

use base64::Engine as _;
use futures::stream::{self, StreamExt};

use crate::Result;
use crate::github::GitHubClient;

/// The contributors API never returns beyond the first 500.
const API_MAX: usize = 500;
/// Concurrent avatar downloads.
const AVATAR_CONCURRENCY: usize = 8;

/// Pixel-normalize an avatar to PNG bytes (spec 00 §4.2): the avatars CDN
/// re-encodes per edge node, so raw bytes drift between requests and would
/// cause daily no-op commits. Decoding and re-encoding with fixed settings
/// makes output a pure function of the pixels.
pub fn normalize_png(bytes: &[u8]) -> Option<Vec<u8>> {
    use image::ImageEncoder as _;
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            &rgba,
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(out)
}

/// One wall cell: contributor identity + avatar payload (`None` → placeholder).
#[derive(Debug)]
pub struct WallEntry {
    pub login: String,
    pub contributions: u64,
    /// `data:<mime>;base64,...` URI, or `None` for the placeholder.
    pub avatar: Option<String>,
}

/// Fetch contributors (API order = contributions desc), filter bots, embed avatars.
pub async fn fetch_wall(
    client: &GitHubClient,
    repo: &str,
    max: usize,
    include_bots: bool,
    avatar_size: u32,
) -> Result<Vec<WallEntry>> {
    let max = if max > API_MAX {
        eprintln!("reposcope: --max clamped to {API_MAX} (contributors API ceiling)");
        API_MAX
    } else {
        max
    };
    let mut contributors = Vec::new();
    let mut page = 1u32;
    while contributors.len() < max {
        let batch = client.contributors_page(repo, page).await?;
        if batch.is_empty() {
            break;
        }
        let exhausted = batch.len() < 100;
        contributors.extend(batch);
        if exhausted {
            break;
        }
        page += 1;
    }
    contributors.retain(|c| c.login.is_some() && (include_bots || !c.is_bot()));
    contributors.truncate(max);

    let entries = stream::iter(contributors)
        .map(|c| async move {
            let login = c.login.unwrap_or_default();
            let avatar = match c.avatar_url {
                Some(u) => {
                    let url = format!("{u}&s={avatar_size}");
                    match client.fetch_bytes(&url).await {
                        Ok((_mime, bytes)) => match normalize_png(&bytes) {
                            Some(png) => Some(format!(
                                "data:image/png;base64,{}",
                                base64::engine::general_purpose::STANDARD.encode(png)
                            )),
                            None => {
                                eprintln!("reposcope: avatar for {login} is not a decodable image; using placeholder");
                                None
                            }
                        },
                        Err(e) => {
                            eprintln!(
                                "reposcope: avatar for {login} failed ({e}); using placeholder"
                            );
                            None
                        }
                    }
                }
                None => None,
            };
            WallEntry {
                login,
                contributions: c.contributions,
                avatar,
            }
        })
        .buffered(AVATAR_CONCURRENCY)
        .collect()
        .await;
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2x2 RGBA test pattern encoded as PNG.
    fn tiny_png() -> Vec<u8> {
        use image::ImageEncoder as _;
        let px: [u8; 16] = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&px, 2, 2, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    #[test]
    fn normalize_is_deterministic_and_idempotent() {
        let png = tiny_png();
        let a = normalize_png(&png).unwrap();
        assert_eq!(normalize_png(&png).unwrap(), a);
        // Normalizing normalized output is a fixed point — CDN re-encodes
        // (PNG→PNG, JPEG→PNG of same pixels) converge to stable bytes.
        assert_eq!(normalize_png(&a).unwrap(), a);
        assert!(normalize_png(b"not an image").is_none());
    }

    #[test]
    fn normalize_accepts_jpeg() {
        use image::ImageEncoder as _;
        // JPEG has no alpha channel — encode from RGB.
        let px: [u8; 12] = [128; 12];
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpg)
            .write_image(&px, 2, 2, image::ExtendedColorType::Rgb8)
            .unwrap();
        let out = normalize_png(&jpg).unwrap();
        assert!(out.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}
