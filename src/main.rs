use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;

use press_party::art::Art;
use press_party::meta::{Changes, apply_plan, build_plan};
use press_party::{dump, scan, tui};

#[derive(Parser)]
#[command(
    name = "press-party",
    version,
    about = "Assign metadata and album art to music — batch-friendly, with in-terminal image preview"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interactive two-column TUI: pick an image on the left, multi-select
    /// tracks on the right, and assign the cover in one keystroke. Shows the
    /// image inline if your terminal supports a graphics protocol.
    Tui {
        /// Directory to scan for tracks and images. Defaults to the cwd.
        dir: Option<PathBuf>,
        /// Only scan the top level; don't descend into subdirectories.
        #[arg(long)]
        no_recurse: bool,
    },

    /// Assign one image as cover art to many tracks. The headline batch op.
    ///
    /// Each FILE may be an audio file or a directory (scanned for audio).
    Art {
        /// Image file to embed as the front cover.
        image: PathBuf,
        /// Tracks (or directories of tracks) to receive the cover.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Print the plan without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Don't back up files before writing.
        #[arg(long)]
        no_backup: bool,
        /// Don't descend into subdirectories of directories given as FILES.
        #[arg(long)]
        no_recurse: bool,
    },

    /// Set arbitrary text tags (and optionally cover art) on one or more files.
    Set {
        /// Tracks (or directories of tracks) to modify.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        artist: Option<String>,
        #[arg(long)]
        album: Option<String>,
        #[arg(long = "album-artist")]
        album_artist: Option<String>,
        #[arg(long)]
        genre: Option<String>,
        #[arg(long)]
        year: Option<u32>,
        #[arg(long)]
        track: Option<u32>,
        /// Image file to embed as the front cover.
        #[arg(long)]
        art: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        no_backup: bool,
        /// Don't descend into subdirectories of directories given as FILES.
        #[arg(long)]
        no_recurse: bool,
    },

    /// Print metadata for tracks. With --art, render embedded cover art inline.
    Dump {
        /// Tracks (or directories of tracks) to inspect.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Render embedded cover art in the terminal.
        #[arg(long)]
        art: bool,
        /// Don't descend into subdirectories of directories given as FILES.
        #[arg(long)]
        no_recurse: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Tui { dir, no_recurse } => {
            let root = dir.unwrap_or(std::env::current_dir()?);
            tui::run(root, !no_recurse)
        }
        Cmd::Art {
            image,
            files,
            dry_run,
            no_backup,
            no_recurse,
        } => run_art(&image, &files, dry_run, !no_backup, !no_recurse),
        Cmd::Set {
            files,
            title,
            artist,
            album,
            album_artist,
            genre,
            year,
            track,
            art,
            dry_run,
            no_backup,
            no_recurse,
        } => {
            let art = art.as_deref().map(Art::load).transpose()?;
            let changes = Changes {
                title,
                artist,
                album,
                album_artist,
                genre,
                year,
                track_no: track,
                art,
            };
            run_set(&files, changes, dry_run, !no_backup, !no_recurse)
        }
        Cmd::Dump {
            files,
            art,
            no_recurse,
        } => {
            let paths = expand(&files, !no_recurse)?;
            if paths.is_empty() {
                bail!("no audio files found");
            }
            dump::run(&paths, art)
        }
    }
}

/// Expand a list of files/dirs into concrete audio-file paths.
fn expand(inputs: &[PathBuf], recurse: bool) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for input in inputs {
        if input.is_dir() {
            let scan = scan::scan_dir(input, recurse)?;
            out.extend(scan.audio);
        } else if input.exists() {
            if scan::is_audio(input) {
                out.push(input.clone());
            } else {
                eprintln!(
                    "{}: not a recognized audio file, skipping",
                    input.display().yellow()
                );
            }
        } else {
            bail!("no such file: {}", input.display());
        }
    }
    out.dedup();
    Ok(out)
}

fn run_art(
    image: &PathBuf,
    files: &[PathBuf],
    dry_run: bool,
    backup: bool,
    recurse: bool,
) -> Result<()> {
    let art = Art::load(image)?;
    let changes = Changes::art_only(art);
    run_set(files, changes, dry_run, backup, recurse)
}

fn run_set(
    files: &[PathBuf],
    changes: Changes,
    dry_run: bool,
    backup: bool,
    recurse: bool,
) -> Result<()> {
    if changes.is_empty() {
        bail!("nothing to set — pass at least one field (e.g. --title, --art)");
    }
    let paths = expand(files, recurse)?;
    if paths.is_empty() {
        bail!("no audio files found");
    }

    let mut plans = Vec::new();
    let mut failed = 0;
    for path in &paths {
        match build_plan(path, changes.clone()) {
            Ok(p) => plans.push(p),
            Err(e) => {
                eprintln!("{}: {e}", path.display().red());
                failed += 1;
            }
        }
    }

    for plan in &plans {
        println!("{}", plan.render());
        println!();
    }

    if dry_run {
        eprintln!(
            "{}",
            format!(
                "dry-run: {} file(s) would change, {failed} skipped. Pass without --dry-run to write.",
                plans.len()
            )
            .cyan()
        );
        return Ok(());
    }

    let mut ok = 0;
    let mut first_backup = None;
    for plan in &plans {
        match apply_plan(plan, backup) {
            Ok(b) => {
                ok += 1;
                if first_backup.is_none() {
                    first_backup = b;
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", plan.track.path.display().red());
                failed += 1;
            }
        }
    }

    let backup_note = match (&first_backup, backup) {
        (Some(p), _) => {
            let dir = p
                .parent()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            format!(" Backups in {dir}")
        }
        (None, false) => " (no backups — --no-backup)".to_string(),
        _ => String::new(),
    };
    eprintln!(
        "{}",
        format!("Wrote {ok} file(s), {failed} failed.{backup_note}").green()
    );
    Ok(())
}
