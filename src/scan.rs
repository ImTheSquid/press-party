//! Filesystem scanning: walk a directory and split what we find into audio
//! files (things we can tag) and image files (candidate album art).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Extensions lofty can read/write tags for that we care about here.
pub const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "m4a", "m4b", "mp4", "aac", "wav", "wave", "aiff", "aif", "aifc", "ogg", "oga",
    "opus", "wv", "ape",
];

/// Image extensions we treat as candidate cover art.
pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff"];

fn ext_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

pub fn is_audio(path: &Path) -> bool {
    ext_lower(path).is_some_and(|e| AUDIO_EXTS.contains(&e.as_str()))
}

pub fn is_image(path: &Path) -> bool {
    ext_lower(path).is_some_and(|e| IMAGE_EXTS.contains(&e.as_str()))
}

#[derive(Default)]
pub struct Scan {
    pub audio: Vec<PathBuf>,
    pub images: Vec<PathBuf>,
}

/// Recursively walk `root`, collecting audio and image files. Hidden entries
/// (dotfiles) and unreadable directories are skipped silently; symlinks are not
/// followed to avoid cycles.
pub fn scan_dir(root: &Path, recurse: bool) -> Result<Scan> {
    let mut scan = Scan::default();
    walk(root, recurse, &mut scan)
        .with_context(|| format!("scanning {}", root.display()))?;
    scan.audio.sort();
    scan.images.sort();
    Ok(scan)
}

fn walk(dir: &Path, recurse: bool, scan: &mut Scan) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()), // unreadable dir — skip, don't abort the whole walk
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if recurse {
                walk(&path, recurse, scan)?;
            }
        } else if is_audio(&path) {
            scan.audio.push(path);
        } else if is_image(&path) {
            scan.images.push(path);
        }
    }
    Ok(())
}
