//! Core metadata model and the read / plan / apply pipeline.
//!
//! Mirrors rekord-ripper's `analysis` module: build a [`Plan`] describing the
//! exact change to a file, render it for preview, then `apply` it — backing up
//! the original first so a bad write is always recoverable.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::tag::{Accessor, ItemKey, Tag};

use crate::art::Art;

/// A snapshot of a track's current tag state, read once.
#[derive(Clone, Debug)]
pub struct Track {
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_no: Option<u32>,
    pub duration_secs: u64,
    pub format: String,
    pub art_count: usize,
    pub file_size: u64,
}

impl Track {
    pub fn read(path: &Path) -> Result<Self> {
        let tagged = lofty::read_from_path(path)
            .with_context(|| format!("reading tags from {}", path.display()))?;
        let format = file_type_name(tagged.file_type());
        let duration_secs = tagged.properties().duration().as_secs();
        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
        let (title, artist, album, album_artist, genre, year, track_no, art_count) = match tag {
            Some(t) => (
                t.title().map(cow_owned),
                t.artist().map(cow_owned),
                t.album().map(cow_owned),
                t.get_string(ItemKey::AlbumArtist).map(str::to_owned),
                t.genre().map(cow_owned),
                read_year(t),
                t.track(),
                t.picture_count() as usize,
            ),
            None => (None, None, None, None, None, None, None, 0),
        };

        Ok(Self {
            path: path.to_path_buf(),
            title,
            artist,
            album,
            album_artist,
            genre,
            year,
            track_no,
            duration_secs,
            format,
            art_count,
            file_size,
        })
    }

    pub fn display_title(&self) -> &str {
        match &self.title {
            Some(t) if !t.is_empty() => t.as_str(),
            _ => file_stem(&self.path),
        }
    }
}

fn cow_owned(c: std::borrow::Cow<'_, str>) -> String {
    c.into_owned()
}

/// Read the year across formats: `Year` where supported (ID3v2, Vorbis, MP4),
/// else the leading year of `RecordingDate` (e.g. RIFF INFO's `ICRD`).
fn read_year(tag: &lofty::tag::Tag) -> Option<u32> {
    tag.get_string(ItemKey::Year)
        .or_else(|| tag.get_string(ItemKey::RecordingDate))
        .and_then(parse_year)
}

fn parse_year(s: &str) -> Option<u32> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.get(0..4)?.parse().ok()
}

fn file_stem(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("(unknown)")
}

pub fn file_type_name(ft: lofty::file::FileType) -> String {
    use lofty::file::FileType::*;
    let s = match ft {
        Aac => "AAC",
        Aiff => "AIFF",
        Ape => "APE",
        Flac => "FLAC",
        Mpeg => "MP3",
        Mp4 => "M4A",
        Opus => "OPUS",
        Vorbis => "OGG",
        Speex => "SPEEX",
        Wav => "WAV",
        WavPack => "WV",
        _ => "AUDIO",
    };
    s.to_string()
}

/// The set of mutations to apply. `None` fields are left untouched; this is a
/// merge, not a replace, so a batch art assignment never clobbers titles.
#[derive(Clone, Default)]
pub struct Changes {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_no: Option<u32>,
    pub art: Option<Art>,
}

impl Changes {
    pub fn art_only(art: Art) -> Self {
        Self {
            art: Some(art),
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.album.is_none()
            && self.album_artist.is_none()
            && self.genre.is_none()
            && self.year.is_none()
            && self.track_no.is_none()
            && self.art.is_none()
    }
}

/// A validated, ready-to-apply change for a single file.
pub struct Plan {
    pub track: Track,
    pub changes: Changes,
}

pub fn build_plan(path: &Path, changes: Changes) -> Result<Plan> {
    if changes.is_empty() {
        bail!("nothing to change for {}", path.display());
    }
    let track = Track::read(path)?;
    Ok(Plan { track, changes })
}

impl Plan {
    /// One-line-per-field human preview, e.g. for `--dry-run` and CLI output.
    pub fn render(&self) -> String {
        let mut lines = vec![format!(
            "{}  [{}]",
            self.track.display_title(),
            self.track.format
        )];
        let c = &self.changes;
        field(&mut lines, "title", &self.track.title, &c.title);
        field(&mut lines, "artist", &self.track.artist, &c.artist);
        field(&mut lines, "album", &self.track.album, &c.album);
        field(
            &mut lines,
            "album-artist",
            &self.track.album_artist,
            &c.album_artist,
        );
        field(&mut lines, "genre", &self.track.genre, &c.genre);
        field_u32(&mut lines, "year", self.track.year, c.year);
        field_u32(&mut lines, "track", self.track.track_no, c.track_no);
        if let Some(art) = &c.art {
            let dims = art
                .dimensions
                .map(|(w, h)| format!("{w}×{h}"))
                .unwrap_or_else(|| "?".into());
            lines.push(format!(
                "    art: {} → {} ({}, {})",
                self.track.art_count,
                self.track.art_count.max(1),
                art.mime_str(),
                dims
            ));
        }
        lines.join("\n")
    }
}

fn field(lines: &mut Vec<String>, name: &str, before: &Option<String>, after: &Option<String>) {
    if let Some(new) = after {
        let old = before.as_deref().unwrap_or("—");
        lines.push(format!("    {name}: {old:?} → {new:?}"));
    }
}

fn field_u32(lines: &mut Vec<String>, name: &str, before: Option<u32>, after: Option<u32>) {
    if let Some(new) = after {
        match before {
            Some(old) => lines.push(format!("    {name}: {old} → {new}")),
            None => lines.push(format!("    {name}: — → {new}")),
        }
    }
}

/// Apply a plan to disk. Backs up the original file first (unless `backup` is
/// false) and returns the backup path that was written, if any.
pub fn apply_plan(plan: &Plan, backup: bool) -> Result<Option<PathBuf>> {
    let path = &plan.track.path;

    let backup_path = if backup {
        Some(backup_file(path)?)
    } else {
        None
    };

    let mut tagged = lofty::read_from_path(path)
        .with_context(|| format!("re-reading {} before write", path.display()))?;

    // Ensure there is a primary tag to write into; files with no tags at all
    // need one created of the format's native type.
    if tagged.primary_tag_mut().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .expect("primary tag present after insert");

    apply_changes(tag, &plan.changes);

    tagged
        .save_to_path(path, WriteOptions::default())
        .with_context(|| format!("writing tags to {}", path.display()))?;

    Ok(backup_path)
}

fn apply_changes(tag: &mut Tag, c: &Changes) {
    if let Some(v) = &c.title {
        tag.set_title(v.clone());
    }
    if let Some(v) = &c.artist {
        tag.set_artist(v.clone());
    }
    if let Some(v) = &c.album {
        tag.set_album(v.clone());
    }
    if let Some(v) = &c.album_artist {
        tag.insert_text(ItemKey::AlbumArtist, v.clone());
    }
    if let Some(v) = &c.genre {
        tag.set_genre(v.clone());
    }
    if let Some(v) = c.year {
        // Write both so it round-trips regardless of format: `Year` covers
        // ID3v2/Vorbis/MP4, `RecordingDate` covers RIFF INFO (`ICRD`) etc.
        tag.insert_text(ItemKey::Year, v.to_string());
        tag.insert_text(ItemKey::RecordingDate, v.to_string());
    }
    if let Some(v) = c.track_no {
        tag.set_track(v);
    }
    if let Some(art) = &c.art {
        // Replace any existing front cover so we don't accumulate duplicates.
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(art.to_cover());
    }
}

/// Copy `path` into the backup directory before mutation. The backup name
/// includes a timestamp-free short hash of the absolute path plus the original
/// filename, so files with the same name in different folders never collide.
pub fn backup_file(path: &Path) -> Result<PathBuf> {
    let dir = backup_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating backup dir {}", dir.display()))?;

    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    abs.hash(&mut hasher);
    let tag = format!("{:08x}", hasher.finish() & 0xffff_ffff);

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "track".into());
    let target = dir.join(format!("{tag}.{name}.bak"));

    std::fs::copy(path, &target)
        .with_context(|| format!("backing up {} to {}", path.display(), target.display()))?;
    Ok(target)
}

/// Platform data dir for backups. Created on first use by [`backup_file`].
pub fn backup_dir() -> Result<PathBuf> {
    let base = data_base_dir()?;
    Ok(base.join("press-party").join("backups"))
}

fn data_base_dir() -> Result<PathBuf> {
    use std::env::var;
    #[cfg(target_os = "windows")]
    {
        return var("LOCALAPPDATA")
            .map(PathBuf::from)
            .map_err(|_| anyhow::anyhow!("LOCALAPPDATA not set"));
    }
    #[cfg(target_os = "macos")]
    {
        return var("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support"))
            .map_err(|_| anyhow::anyhow!("HOME not set"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Ok(x) = var("XDG_DATA_HOME") {
            if !x.is_empty() {
                return Ok(PathBuf::from(x));
            }
        }
        return var("HOME")
            .map(|h| PathBuf::from(h).join(".local/share"))
            .map_err(|_| anyhow::anyhow!("HOME not set"));
    }
}
