use std::collections::HashSet;
use std::path::PathBuf;

use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::meta::Plan;

use super::data::{ImageRow, TrackRow, image_visible, load_rows, rescan, track_visible};
use super::preview::{Decoded, PreviewLoader, Source, SourceRead, read_source};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The IMAGES column — pick one cover to assign.
    Images,
    /// The TRACKS column — multi-select the destinations.
    Tracks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search(Focus),
    Edit,
    Confirm,
    Help,
}

/// The editable text fields, in display order. Index maps to a `Changes` field
/// in `events::changes_from_edit`.
pub const EDIT_LABELS: [&str; 7] = [
    "title",
    "artist",
    "album",
    "album-artist",
    "genre",
    "year",
    "track",
];

/// In-flight text-tag edit. Targets are captured when the panel opens so the
/// selection can change underneath without affecting the pending edit.
#[derive(Clone, Debug)]
pub struct EditState {
    pub targets: Vec<String>,
    pub values: [String; 7],
    pub focus: usize,
    pub error: Option<String>,
    /// True when editing more than one track; blank fields are left unchanged.
    pub batch: bool,
}

impl EditState {
    pub fn focused_value_mut(&mut self) -> &mut String {
        &mut self.values[self.focus]
    }
}

#[derive(Clone, Debug)]
pub struct ColumnState {
    pub query: String,
    pub visible: Vec<usize>,
    pub cursor: usize,
    pub selected: HashSet<String>,
}

impl Default for ColumnState {
    fn default() -> Self {
        Self {
            query: String::new(),
            visible: Vec::new(),
            cursor: 0,
            selected: HashSet::new(),
        }
    }
}

impl ColumnState {
    pub fn clamp_cursor(&mut self) {
        if self.visible.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.visible.len() {
            self.cursor = self.visible.len() - 1;
        }
    }
    pub fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.cursor = 0;
            return;
        }
        let n = self.visible.len() as isize;
        let mut c = self.cursor as isize + delta;
        if c < 0 {
            c = 0;
        }
        if c >= n {
            c = n - 1;
        }
        self.cursor = c as usize;
    }
    pub fn jump_top(&mut self) {
        self.cursor = 0;
    }
    pub fn jump_bottom(&mut self) {
        if !self.visible.is_empty() {
            self.cursor = self.visible.len() - 1;
        }
    }
}

#[derive(Default)]
pub struct StatusLine {
    pub text: String,
    pub level: StatusLevel,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum StatusLevel {
    #[default]
    Info,
    Warn,
    Err,
    Ok,
}

impl StatusLine {
    pub fn info(&mut self, msg: impl Into<String>) {
        self.text = msg.into();
        self.level = StatusLevel::Info;
    }
    pub fn ok(&mut self, msg: impl Into<String>) {
        self.text = msg.into();
        self.level = StatusLevel::Ok;
    }
    pub fn warn(&mut self, msg: impl Into<String>) {
        self.text = msg.into();
        self.level = StatusLevel::Warn;
    }
    pub fn err(&mut self, msg: impl Into<String>) {
        self.text = msg.into();
        self.level = StatusLevel::Err;
    }
}

/// A built batch awaiting confirmation. `summary` describes the change (e.g.
/// `cover: art.jpg` or `title, artist`) for the confirm modal header.
pub struct PendingBatch {
    pub summary: String,
    pub plans: Vec<Plan>,
    pub failures: Vec<(String, String)>, // track title, error
}

/// Cached terminal-graphics protocol for the currently previewed image. Rebuilt
/// only when the preview source changes.
pub struct Preview {
    pub source: Source,
    pub protocol: StatefulProtocol,
    /// Content hash of the displayed image, so switching to a different source
    /// with identical content (e.g. another track off the same album) can keep
    /// the current image instead of clearing and re-decoding it.
    pub hash: u64,
}

pub struct App {
    pub root: PathBuf,
    pub recurse: bool,

    pub tracks: Vec<TrackRow>,
    pub images: Vec<ImageRow>,

    pub img_col: ColumnState,
    pub trk_col: ColumnState,
    pub focus: Focus,
    pub mode: InputMode,
    pub backup: bool,

    pub picker: Picker,
    pub preview: Option<Preview>,
    /// Source the background loader is currently decoding, if any. Drives the
    /// "loading…" placeholder and a tighter poll interval while in flight.
    pub preview_loading: Option<Source>,
    /// Sources that failed to decode, so we don't request them again each frame.
    pub preview_failed: HashSet<Source>,
    /// Sources that resolved with no image (e.g. a track with no embedded art).
    pub preview_empty: HashSet<Source>,
    pub loader: PreviewLoader,

    pub status: StatusLine,
    pub pending: Option<PendingBatch>,
    pub edit: Option<EditState>,
    pub unresolved_errors: bool,
    pub quit_pending: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(root: PathBuf, recurse: bool, picker: Picker) -> anyhow::Result<Self> {
        let scan = rescan(&root, recurse)?;
        let (tracks, images, skipped) = load_rows(&scan);
        let mut app = App {
            root,
            recurse,
            tracks,
            images,
            img_col: ColumnState::default(),
            trk_col: ColumnState::default(),
            focus: Focus::Images,
            mode: InputMode::Normal,
            backup: true,
            picker,
            preview: None,
            preview_loading: None,
            preview_failed: HashSet::new(),
            preview_empty: HashSet::new(),
            loader: PreviewLoader::new(),
            status: StatusLine::default(),
            pending: None,
            edit: None,
            unresolved_errors: false,
            quit_pending: false,
            should_quit: false,
        };
        app.recompute_visible();
        let mut msg = format!("{} tracks, {} images.", app.tracks.len(), app.images.len());
        if skipped > 0 {
            msg.push_str(&format!(" ({skipped} unreadable, skipped)"));
        }
        app.status.info(msg);
        Ok(app)
    }

    pub fn recompute_visible(&mut self) {
        self.img_col.visible = image_visible(&self.images, &self.img_col.query);
        self.img_col.clamp_cursor();
        self.trk_col.visible = track_visible(&self.tracks, &self.trk_col.query);
        self.trk_col.clamp_cursor();
    }

    pub fn reload(&mut self) -> anyhow::Result<()> {
        let scan = rescan(&self.root, self.recurse)?;
        let (tracks, images, _) = load_rows(&scan);
        self.tracks = tracks;
        self.images = images;
        let existing: HashSet<&str> = self.tracks.iter().map(|r| r.id.as_str()).collect();
        self.trk_col.selected.retain(|id| existing.contains(id.as_str()));
        self.preview = None;
        self.preview_loading = None;
        self.preview_failed.clear();
        self.preview_empty.clear();
        self.recompute_visible();
        Ok(())
    }

    pub fn focused_column_mut(&mut self) -> &mut ColumnState {
        match self.focus {
            Focus::Images => &mut self.img_col,
            Focus::Tracks => &mut self.trk_col,
        }
    }

    pub fn current_image(&self) -> Option<&ImageRow> {
        self.img_col
            .visible
            .get(self.img_col.cursor)
            .and_then(|&i| self.images.get(i))
    }

    pub fn current_track(&self) -> Option<&TrackRow> {
        self.trk_col
            .visible
            .get(self.trk_col.cursor)
            .and_then(|&i| self.tracks.get(i))
    }

    /// The tracks an apply would touch: the explicit multi-selection, or the
    /// cursor row when nothing is explicitly selected.
    pub fn target_track_ids(&self) -> Vec<String> {
        if !self.trk_col.selected.is_empty() {
            let mut v: Vec<String> = self.trk_col.selected.iter().cloned().collect();
            v.sort();
            v
        } else if let Some(t) = self.current_track() {
            vec![t.id.clone()]
        } else {
            Vec::new()
        }
    }

    /// True while a popup (edit / confirm / help) is covering the UI.
    pub fn modal_open(&self) -> bool {
        matches!(
            self.mode,
            InputMode::Edit | InputMode::Confirm | InputMode::Help
        )
    }

    /// Look up the on-disk path for a track id.
    pub fn track_path(&self, id: &str) -> Option<std::path::PathBuf> {
        self.tracks
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.path.clone())
    }

    /// Build an [`EditState`] for the current target tracks. A single target is
    /// prefilled from its existing tags; a multi-target batch starts blank so
    /// untouched fields are left unchanged.
    pub fn make_edit_state(&self) -> Option<EditState> {
        let targets = self.target_track_ids();
        if targets.is_empty() {
            return None;
        }
        let batch = targets.len() > 1;
        let mut values: [String; 7] = Default::default();
        if !batch {
            if let Some(path) = self.track_path(&targets[0]) {
                if let Ok(t) = crate::meta::Track::read(&path) {
                    values[0] = t.title.unwrap_or_default();
                    values[1] = t.artist.unwrap_or_default();
                    values[2] = t.album.unwrap_or_default();
                    values[3] = t.album_artist.unwrap_or_default();
                    values[4] = t.genre.unwrap_or_default();
                    values[5] = t.year.map(|y| y.to_string()).unwrap_or_default();
                    values[6] = t.track_no.map(|n| n.to_string()).unwrap_or_default();
                }
            }
        }
        Some(EditState {
            targets,
            values,
            focus: 0,
            error: None,
            batch,
        })
    }

    /// What the preview pane should show, driven by focus: the cursored image
    /// file when on the IMAGES column, or the cursored track's embedded cover
    /// when on the TRACKS column.
    pub fn preview_target(&self) -> Option<Source> {
        match self.focus {
            Focus::Images => self.current_image().map(|i| Source::ImageFile(i.path.clone())),
            Focus::Tracks => self.current_track().map(|t| Source::EmbeddedArt(t.path.clone())),
        }
    }

    /// React to a change in the preview target. Reading the encoded bytes is
    /// cheap, so we do it synchronously and hash the content: if it's identical
    /// to what's already on screen (the common "whole album shares one cover"
    /// case) we keep the current image untouched; otherwise we clear it
    /// immediately — so the stale image never lingers — and decode async.
    pub fn sync_preview(&mut self) {
        let Some(target) = self.preview_target() else {
            self.preview = None;
            self.preview_loading = None;
            return;
        };
        // Already showing exactly this source, or already decoding it: nothing
        // to do.
        if self.preview.as_ref().map(|p| &p.source) == Some(&target)
            || self.preview_loading.as_ref() == Some(&target)
        {
            return;
        }
        // Known to have no image (or be unreadable): make sure nothing is shown.
        // Crucially this clears whatever cover was on screen before — e.g. when
        // tabbing from the IMAGES column onto an art-less track we've already
        // visited — instead of leaving the previous image stuck.
        if self.preview_failed.contains(&target) || self.preview_empty.contains(&target) {
            self.preview = None;
            self.preview_loading = None;
            return;
        }

        match read_source(&target) {
            SourceRead::Image(bytes, hash) => {
                if self.preview.as_ref().map(|p| p.hash) == Some(hash) {
                    // Identical content already displayed — keep it, just point
                    // the preview at the new source so it counts as shown.
                    if let Some(p) = self.preview.as_mut() {
                        p.source = target;
                    }
                    self.preview_loading = None;
                } else {
                    // Different content: clear now, decode on the worker.
                    self.preview = None;
                    self.preview_loading = Some(target.clone());
                    self.loader.request(target, bytes, hash);
                }
            }
            SourceRead::NoImage => {
                self.preview = None;
                self.preview_loading = None;
                self.preview_empty.insert(target);
            }
            SourceRead::Error => {
                self.preview = None;
                self.preview_loading = None;
                self.preview_failed.insert(target);
            }
        }
    }

    /// Resolution of the current preview target, for the placeholder text.
    pub fn current_preview_failed(&self) -> bool {
        self.preview_target()
            .is_some_and(|t| self.preview_failed.contains(&t))
    }

    pub fn current_preview_empty(&self) -> bool {
        self.preview_target()
            .is_some_and(|t| self.preview_empty.contains(&t))
    }

    /// Drain finished decodes from the loader. Adopts a result only if it's
    /// still for the current target; stale results (cursor moved on) are
    /// dropped. Returns true if the preview changed and a redraw is warranted.
    pub fn poll_preview(&mut self) -> bool {
        let current = self.preview_target();
        let mut changed = false;
        while let Some(decoded) = self.loader.try_recv() {
            match decoded {
                Decoded::Ok(source, img, hash) => {
                    if current.as_ref() == Some(&source) {
                        let protocol = self.picker.new_resize_protocol(img);
                        self.preview = Some(Preview {
                            source,
                            protocol,
                            hash,
                        });
                        self.preview_loading = None;
                        changed = true;
                    }
                }
                Decoded::Err(source) => {
                    if self.preview_loading.as_ref() == Some(&source) {
                        self.preview_loading = None;
                    }
                    if current.as_ref() == Some(&source) {
                        self.preview = None;
                        changed = true;
                    }
                    self.preview_failed.insert(source);
                }
            }
        }
        changed
    }
}
