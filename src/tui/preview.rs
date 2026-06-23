//! Background image decoder. Reading the encoded bytes (a file, or the picture
//! embedded in a track) is cheap I/O and happens on the main thread so we can
//! hash the content and decide instantly whether the preview even needs to
//! change. The expensive part — decoding a 4000×4000 JPEG and downscaling it —
//! runs on a worker thread so it never stutters the event loop.

use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};

use image::DynamicImage;

/// Longest-side pixel cap for preview images. The preview pane is at most a few
/// hundred pixels wide, so anything larger is wasted work in every re-encode.
const PREVIEW_MAX_PX: u32 = 900;

/// What to preview: a standalone image file (the cover to assign) or the cover
/// already embedded in a track (when inspecting the TRACKS column).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    ImageFile(PathBuf),
    EmbeddedArt(PathBuf),
}

/// Result of synchronously reading a source's encoded bytes.
pub enum SourceRead {
    /// Encoded image bytes plus a content hash for cheap equality checks.
    Image(Vec<u8>, u64),
    /// Resolved fine, but there's no image (a track with no embedded cover).
    NoImage,
    /// The source could not be read or parsed.
    Error,
}

/// A decoded, downscaled image ready to build a protocol from, tagged with its
/// source and content hash so the main thread can drop stale results.
pub enum Decoded {
    Ok(Source, DynamicImage, u64),
    Err(Source),
}

struct Request {
    source: Source,
    bytes: Vec<u8>,
    hash: u64,
}

pub struct PreviewLoader {
    req_tx: Sender<Request>,
    res_rx: Receiver<Decoded>,
    _handle: JoinHandle<()>,
}

impl PreviewLoader {
    pub fn new() -> Self {
        let (req_tx, req_rx) = channel::<Request>();
        let (res_tx, res_rx) = channel::<Decoded>();
        let handle = thread::spawn(move || worker(req_rx, res_tx));
        Self {
            req_tx,
            res_rx,
            _handle: handle,
        }
    }

    /// Hand already-read bytes to the worker to decode. Cheap; never blocks.
    pub fn request(&self, source: Source, bytes: Vec<u8>, hash: u64) {
        let _ = self.req_tx.send(Request {
            source,
            bytes,
            hash,
        });
    }

    /// Non-blocking drain of one ready result, if any.
    pub fn try_recv(&self) -> Option<Decoded> {
        self.res_rx.try_recv().ok()
    }
}

impl Default for PreviewLoader {
    fn default() -> Self {
        Self::new()
    }
}

fn worker(req_rx: Receiver<Request>, res_tx: Sender<Decoded>) {
    while let Ok(mut req) = req_rx.recv() {
        // Coalesce: if the user scrolled past several items while we were
        // decoding, skip straight to the most recent request and drop the rest.
        while let Ok(newer) = req_rx.try_recv() {
            req = newer;
        }
        let decoded = match image::load_from_memory(&req.bytes) {
            Ok(img) => Decoded::Ok(req.source, downscale(img), req.hash),
            Err(_) => Decoded::Err(req.source),
        };
        if res_tx.send(decoded).is_err() {
            break; // main thread is gone; shut the worker down
        }
    }
}

/// Read a source's encoded bytes and hash them. Cheap relative to decoding.
pub fn read_source(source: &Source) -> SourceRead {
    match source {
        Source::ImageFile(path) => match std::fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => {
                let h = hash_bytes(&bytes);
                SourceRead::Image(bytes, h)
            }
            _ => SourceRead::Error,
        },
        Source::EmbeddedArt(path) => embedded_cover(path),
    }
}

fn embedded_cover(path: &Path) -> SourceRead {
    use lofty::file::TaggedFileExt;
    use lofty::picture::PictureType;
    let Ok(tagged) = lofty::read_from_path(path) else {
        return SourceRead::Error;
    };
    let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
        return SourceRead::NoImage;
    };
    let pic = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first());
    match pic {
        Some(p) => {
            let bytes = p.data().to_vec();
            let h = hash_bytes(&bytes);
            SourceRead::Image(bytes, h)
        }
        None => SourceRead::NoImage,
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn downscale(img: DynamicImage) -> DynamicImage {
    use image::GenericImageView;
    let (w, h) = img.dimensions();
    if w <= PREVIEW_MAX_PX && h <= PREVIEW_MAX_PX {
        img
    } else {
        // `thumbnail` preserves aspect ratio and uses a fast linear filter.
        img.thumbnail(PREVIEW_MAX_PX, PREVIEW_MAX_PX)
    }
}
