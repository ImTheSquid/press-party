//! Cached row models for the two TUI columns: tracks (taggable audio) on one
//! side, images (candidate cover art) on the other. Built once at startup and
//! after each apply batch.

use std::path::{Path, PathBuf};

use crate::meta::Track;
use crate::scan::Scan;

/// A single audio file shown in the TRACKS column.
#[derive(Clone, Debug)]
pub struct TrackRow {
    /// Stable identity for the selection set: the path as a string.
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub format: String,
    pub art_count: usize,
    pub duration_secs: u64,
    pub search_blob: String,
}

impl TrackRow {
    fn from_track(t: Track) -> Self {
        let title = t.display_title().to_string();
        let artist = t.artist.clone().unwrap_or_default();
        let search_blob = format!("{} {}", title.to_lowercase(), artist.to_lowercase());
        Self {
            id: t.path.to_string_lossy().into_owned(),
            path: t.path,
            title,
            artist,
            format: t.format,
            art_count: t.art_count,
            duration_secs: t.duration_secs,
            search_blob,
        }
    }
}

/// A single image file shown in the IMAGES column.
#[derive(Clone, Debug)]
pub struct ImageRow {
    pub path: PathBuf,
    pub name: String,
    pub dims: Option<(u32, u32)>,
    pub size: u64,
    pub search_blob: String,
}

impl ImageRow {
    fn from_path(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Header-only read; cheap and never decodes the full image.
        let dims = image::image_dimensions(&path).ok();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let search_blob = name.to_lowercase();
        Self {
            path,
            name,
            dims,
            size,
            search_blob,
        }
    }
}

/// Read every scanned file into row form. Tag-read failures are skipped (with
/// the count returned) rather than aborting the whole load.
pub fn load_rows(scan: &Scan) -> (Vec<TrackRow>, Vec<ImageRow>, usize) {
    let mut tracks = Vec::new();
    let mut skipped = 0usize;
    for path in &scan.audio {
        match Track::read(path) {
            Ok(t) => tracks.push(TrackRow::from_track(t)),
            Err(_) => skipped += 1,
        }
    }
    let images = scan.images.iter().cloned().map(ImageRow::from_path).collect();
    (tracks, images, skipped)
}

pub fn track_visible(rows: &[TrackRow], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, r)| q.is_empty() || r.search_blob.contains(&q))
        .map(|(i, _)| i)
        .collect()
}

pub fn image_visible(rows: &[ImageRow], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, r)| q.is_empty() || r.search_blob.contains(&q))
        .map(|(i, _)| i)
        .collect()
}

pub fn rescan(root: &Path, recurse: bool) -> anyhow::Result<Scan> {
    crate::scan::scan_dir(root, recurse)
}
