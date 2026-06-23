//! Album-art handling: load an image from disk (or from bytes embedded in a
//! track), detect its MIME type, build a lofty `Picture`, and render it inline
//! in terminals that support a graphics protocol (Kitty / iTerm2 / Sixel), with
//! a unicode half-block fallback everywhere else.

use std::path::Path;

use anyhow::{Context, Result, bail};
use lofty::picture::{MimeType, Picture, PictureType};
use viuer::KittySupport;

/// An image loaded into memory, ready to embed as cover art or preview.
#[derive(Clone)]
pub struct Art {
    pub bytes: Vec<u8>,
    pub mime: MimeType,
    /// Decoded pixel dimensions, if the image crate could parse them.
    pub dimensions: Option<(u32, u32)>,
}

impl Art {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("reading image {}", path.display()))?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() {
            bail!("image is empty");
        }
        let mime = detect_mime(&bytes);
        let dimensions = image::load_from_memory(&bytes)
            .ok()
            .map(|img| (img.width(), img.height()));
        Ok(Self {
            bytes,
            mime,
            dimensions,
        })
    }

    /// Build a front-cover `Picture` for embedding into a tag.
    pub fn to_cover(&self) -> Picture {
        Picture::unchecked(self.bytes.clone())
            .pic_type(PictureType::CoverFront)
            .mime_type(self.mime.clone())
            .build()
    }

    pub fn mime_str(&self) -> &str {
        mime_str(&self.mime)
    }
}

/// Sniff the MIME type from magic bytes. Falls back to a sensible default
/// rather than failing, since embedding tolerates an approximate type.
pub fn detect_mime(bytes: &[u8]) -> MimeType {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        MimeType::Jpeg
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        MimeType::Png
    } else if bytes.starts_with(b"GIF8") {
        MimeType::Gif
    } else if bytes.starts_with(b"BM") {
        MimeType::Bmp
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        MimeType::Unknown("image/webp".to_string())
    } else if bytes.starts_with(&[0x49, 0x49, 0x2A, 0x00])
        || bytes.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
    {
        MimeType::Tiff
    } else {
        MimeType::Unknown("application/octet-stream".to_string())
    }
}

pub fn mime_str(mime: &MimeType) -> &str {
    match mime {
        MimeType::Png => "image/png",
        MimeType::Jpeg => "image/jpeg",
        MimeType::Tiff => "image/tiff",
        MimeType::Bmp => "image/bmp",
        MimeType::Gif => "image/gif",
        MimeType::Unknown(s) => s.as_str(),
        _ => "image/unknown",
    }
}

/// True if the terminal advertises a real graphics protocol (Kitty or iTerm2).
/// When false, viuer still renders via unicode half-blocks.
pub fn terminal_supports_graphics() -> bool {
    viuer::get_kitty_support() != KittySupport::None || viuer::is_iterm_supported()
}

/// Print an image inline using the best protocol the terminal supports.
/// `max_height` is in terminal rows; width auto-scales to preserve aspect.
pub fn print_inline(art: &Art, max_height: u32) -> Result<()> {
    let img = image::load_from_memory(&art.bytes).context("decoding image for display")?;
    let conf = viuer::Config {
        transparent: true,
        absolute_offset: false,
        height: Some(max_height),
        ..Default::default()
    };
    viuer::print(&img, &conf).context("rendering image in terminal")?;
    Ok(())
}
