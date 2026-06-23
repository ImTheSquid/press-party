//! `dump` / `show`: print a track's metadata, and optionally render its
//! embedded cover art inline in the terminal.

use std::path::Path;

use anyhow::Result;
use owo_colors::OwoColorize;

use crate::art::{self, Art};
use crate::format::{format_duration, format_size};
use crate::meta::Track;

pub fn run(paths: &[std::path::PathBuf], show_art: bool) -> Result<()> {
    for (i, path) in paths.iter().enumerate() {
        if i > 0 {
            println!();
        }
        match Track::read(path) {
            Ok(track) => print_track(&track, show_art),
            Err(e) => eprintln!("{}: {e}", path.display().red()),
        }
    }
    Ok(())
}

fn print_track(track: &Track, show_art: bool) {
    println!(
        "{}  {}",
        track.display_title().bold(),
        format!("[{}]", track.format).dimmed()
    );
    println!("  {}", track.path.display().dimmed());

    row("artist", track.artist.as_deref());
    row("album", track.album.as_deref());
    row("album-artist", track.album_artist.as_deref());
    row("genre", track.genre.as_deref());
    row_owned("year", track.year.map(|y| y.to_string()));
    row_owned("track", track.track_no.map(|t| t.to_string()));

    println!(
        "  {:<13} {}   {}",
        "length:".cyan(),
        format_duration(track.duration_secs),
        format_size(track.file_size).dimmed()
    );

    let art_label = if track.art_count == 0 {
        "none".dimmed().to_string()
    } else {
        format!("{} embedded", track.art_count).green().to_string()
    };
    println!("  {:<13} {}", "art:".cyan(), art_label);

    if show_art {
        print_embedded_art(track);
    }
}

fn row(name: &str, value: Option<&str>) {
    let label = format!("{name}:");
    match value.filter(|v| !v.is_empty()) {
        Some(v) => println!("  {:<13} {}", label.cyan(), v),
        None => println!("  {:<13} {}", label.cyan(), "—".dimmed()),
    }
}

fn row_owned(name: &str, value: Option<String>) {
    row(name, value.as_deref());
}

fn print_embedded_art(track: &Track) {
    if track.art_count == 0 {
        return;
    }
    match extract_cover(&track.path) {
        Ok(Some(cover)) => {
            if !art::terminal_supports_graphics() {
                println!(
                    "  {}",
                    "(terminal has no graphics protocol; rendering as blocks)".dimmed()
                );
            }
            if let Err(e) = art::print_inline(&cover, 16) {
                eprintln!("  {}: {e}", "art preview failed".yellow());
            }
        }
        Ok(None) => {}
        Err(e) => eprintln!("  {}: {e}", "could not read embedded art".yellow()),
    }
}

/// Pull the first embedded picture out of a file as an [`Art`].
pub fn extract_cover(path: &Path) -> Result<Option<Art>> {
    let tagged = lofty::read_from_path(path)?;
    use lofty::file::TaggedFileExt;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let Some(tag) = tag else {
        return Ok(None);
    };
    // Prefer an explicit front cover, else fall back to the first picture.
    let pic = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront)
        .or_else(|| tag.pictures().first());
    match pic {
        Some(p) => Ok(Some(Art::from_bytes(p.data().to_vec())?)),
        None => Ok(None),
    }
}
