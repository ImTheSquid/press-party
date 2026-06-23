use std::collections::HashSet;
use std::path::PathBuf;

use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::art::Art;
use crate::meta::Plan;

use super::data::{ImageRow, TrackRow, image_visible, load_rows, rescan, track_visible};

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
/// only when the image-column cursor moves to a different file.
pub struct Preview {
    pub path: PathBuf,
    pub protocol: StatefulProtocol,
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

    /// Rebuild the cached preview protocol if the cursored image changed.
    pub fn sync_preview(&mut self) {
        let Some(img) = self.current_image() else {
            self.preview = None;
            return;
        };
        let path = img.path.clone();
        if self.preview.as_ref().map(|p| &p.path) == Some(&path) {
            return;
        }
        match Art::load(&path).and_then(|art| {
            image::load_from_memory(&art.bytes).map_err(anyhow::Error::from)
        }) {
            Ok(dyn_img) => {
                let protocol = self.picker.new_resize_protocol(downscale_for_preview(dyn_img));
                self.preview = Some(Preview { path, protocol });
            }
            Err(_) => {
                self.preview = None;
            }
        }
    }
}

/// Longest-side pixel cap for preview images. The preview pane is at most a few
/// hundred pixels wide, so a 4000×4000 cover is pure waste — downscaling once
/// here keeps every later re-encode (e.g. on modal close) cheap.
const PREVIEW_MAX_PX: u32 = 900;

fn downscale_for_preview(img: image::DynamicImage) -> image::DynamicImage {
    use image::GenericImageView;
    let (w, h) = img.dimensions();
    if w <= PREVIEW_MAX_PX && h <= PREVIEW_MAX_PX {
        return img;
    }
    // `thumbnail` preserves aspect ratio and uses a fast linear filter.
    img.thumbnail(PREVIEW_MAX_PX, PREVIEW_MAX_PX)
}
