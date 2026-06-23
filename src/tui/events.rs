use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::art::Art;
use crate::meta::{Changes, apply_plan, build_plan};

use super::app::{App, EditState, Focus, InputMode, PendingBatch};

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return;
    }
    let is_quit_key = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc);
    if !is_quit_key {
        app.quit_pending = false;
    }

    let mode = app.mode.clone();
    match mode {
        InputMode::Normal => handle_normal(app, key),
        InputMode::Search(focus) => handle_search(app, key, focus),
        InputMode::Edit => handle_edit(app, key),
        InputMode::Confirm => handle_confirm(app, key),
        InputMode::Help => app.mode = InputMode::Normal,
    }
}

fn has_pending_work(app: &App) -> bool {
    !app.trk_col.selected.is_empty() || app.unresolved_errors
}

fn try_quit(app: &mut App) {
    if app.quit_pending || !has_pending_work(app) {
        app.should_quit = true;
        return;
    }
    app.quit_pending = true;
    let mut bits = Vec::new();
    if !app.trk_col.selected.is_empty() {
        bits.push(format!("{} track(s) selected", app.trk_col.selected.len()));
    }
    if app.unresolved_errors {
        bits.push("unresolved errors".into());
    }
    app.status
        .warn(format!("{}. Press 'q' again to quit.", bits.join(", ")));
}

fn handle_normal(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
            app.focus = match app.focus {
                Focus::Images => Focus::Tracks,
                Focus::Tracks => Focus::Images,
            };
        }
        (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => try_quit(app),
        (KeyCode::Char('?'), _) => app.mode = InputMode::Help,
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => app.focused_column_mut().move_by(-1),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => app.focused_column_mut().move_by(1),
        (KeyCode::PageUp, _) => app.focused_column_mut().move_by(-10),
        (KeyCode::PageDown, _) => app.focused_column_mut().move_by(10),
        (KeyCode::Char('g'), _) => app.focused_column_mut().jump_top(),
        (KeyCode::Char('G'), _) => app.focused_column_mut().jump_bottom(),
        (KeyCode::Char('/'), _) => app.mode = InputMode::Search(app.focus),
        (KeyCode::Char(' '), _) => {
            if app.focus == Focus::Tracks {
                if let Some(id) = app
                    .trk_col
                    .visible
                    .get(app.trk_col.cursor)
                    .and_then(|&i| app.tracks.get(i))
                    .map(|r| r.id.clone())
                {
                    if !app.trk_col.selected.remove(&id) {
                        app.trk_col.selected.insert(id);
                    }
                }
            }
        }
        (KeyCode::Char('c'), _) => {
            app.trk_col.selected.clear();
            app.status.info("cleared selection");
        }
        (KeyCode::Char('b'), _) => {
            app.backup = !app.backup;
            app.status
                .info(format!("backup = {}", if app.backup { "ON" } else { "off" }));
        }
        (KeyCode::Char('e'), _) => match app.make_edit_state() {
            Some(state) => {
                let label = if state.batch {
                    format!("editing {} tracks", state.targets.len())
                } else {
                    "editing 1 track".to_string()
                };
                app.edit = Some(state);
                app.mode = InputMode::Edit;
                app.status.info(label);
            }
            None => app.status.err("no tracks to edit"),
        },
        (KeyCode::Char('R'), _) => match app.reload() {
            Ok(()) => app
                .status
                .ok(format!("Rescanned: {} tracks, {} images.", app.tracks.len(), app.images.len())),
            Err(e) => app.status.err(format!("rescan failed: {e}")),
        },
        (KeyCode::Enter, _) => build_pending(app),
        _ => {}
    }
}

fn handle_search(app: &mut App, key: KeyEvent, focus: Focus) {
    let col = match focus {
        Focus::Images => &mut app.img_col,
        Focus::Tracks => &mut app.trk_col,
    };
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) | (KeyCode::Enter, _) => {
            app.mode = InputMode::Normal;
            return;
        }
        (KeyCode::Backspace, _) => {
            col.query.pop();
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => col.query.clear(),
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => col.query.push(c),
        (KeyCode::Up, _) => col.move_by(-1),
        (KeyCode::Down, _) => col.move_by(1),
        _ => return,
    }
    app.recompute_visible();
}

fn handle_edit(app: &mut App, key: KeyEvent) {
    let Some(edit) = app.edit.as_mut() else {
        app.mode = InputMode::Normal;
        return;
    };
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            app.edit = None;
            app.mode = InputMode::Normal;
            app.status.info("edit cancelled");
        }
        (KeyCode::Tab, _) | (KeyCode::Down, _) => {
            edit.focus = (edit.focus + 1) % edit.values.len();
        }
        (KeyCode::BackTab, _) | (KeyCode::Up, _) => {
            edit.focus = (edit.focus + edit.values.len() - 1) % edit.values.len();
        }
        (KeyCode::Backspace, _) => {
            edit.focused_value_mut().pop();
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => edit.focused_value_mut().clear(),
        (KeyCode::Enter, _) => build_edit_pending(app),
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
            edit.focused_value_mut().push(c);
        }
        _ => {}
    }
}

/// Translate the edit panel's text fields into a [`Changes`]. Blank fields are
/// left unchanged; `year`/`track` must parse as numbers.
fn changes_from_edit(edit: &EditState) -> Result<Changes, String> {
    let get = |i: usize| {
        let v = edit.values[i].trim();
        if v.is_empty() {
            None
        } else {
            Some(v.to_string())
        }
    };
    let parse_num = |i: usize, name: &str| -> Result<Option<u32>, String> {
        match get(i) {
            None => Ok(None),
            Some(s) => s
                .parse::<u32>()
                .map(Some)
                .map_err(|_| format!("{name} '{s}' is not a number")),
        }
    };
    Ok(Changes {
        title: get(0),
        artist: get(1),
        album: get(2),
        album_artist: get(3),
        genre: get(4),
        year: parse_num(5, "year")?,
        track_no: parse_num(6, "track")?,
        art: None,
    })
}

fn build_edit_pending(app: &mut App) {
    let Some(edit) = app.edit.as_ref() else {
        return;
    };
    let changes = match changes_from_edit(edit) {
        Ok(c) => c,
        Err(e) => {
            if let Some(ed) = app.edit.as_mut() {
                ed.error = Some(e);
            }
            return;
        }
    };
    if changes.is_empty() {
        if let Some(ed) = app.edit.as_mut() {
            ed.error = Some("nothing to change — fill at least one field".into());
        }
        return;
    }

    let summary = changes_summary(&changes);
    let targets = edit.targets.clone();
    let mut plans = Vec::new();
    let mut failures = Vec::new();
    for id in &targets {
        let Some(path) = app.track_path(id) else {
            continue;
        };
        let title = app
            .tracks
            .iter()
            .find(|t| &t.id == id)
            .map(|t| t.title.clone())
            .unwrap_or_else(|| id.clone());
        match build_plan(&path, changes.clone()) {
            Ok(plan) => plans.push(plan),
            Err(e) => failures.push((title, e.to_string())),
        }
    }
    if plans.is_empty() {
        if let Some(ed) = app.edit.as_mut() {
            ed.error = Some("no applicable tracks".into());
        }
        return;
    }
    app.pending = Some(PendingBatch {
        summary,
        plans,
        failures,
    });
    app.mode = InputMode::Confirm;
}

/// A short "title, artist, +cover" description of a change set.
fn changes_summary(c: &Changes) -> String {
    let mut parts = Vec::new();
    if c.title.is_some() {
        parts.push("title");
    }
    if c.artist.is_some() {
        parts.push("artist");
    }
    if c.album.is_some() {
        parts.push("album");
    }
    if c.album_artist.is_some() {
        parts.push("album-artist");
    }
    if c.genre.is_some() {
        parts.push("genre");
    }
    if c.year.is_some() {
        parts.push("year");
    }
    if c.track_no.is_some() {
        parts.push("track");
    }
    if c.art.is_some() {
        parts.push("cover");
    }
    parts.join(", ")
}

fn handle_confirm(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        (KeyCode::Char('y'), _) | (KeyCode::Enter, _) => apply_pending(app),
        (KeyCode::Char('n'), _) | (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => {
            app.pending = None;
            // If this confirm came from the edit panel, return there so the
            // user's typing isn't lost; otherwise back to normal.
            app.mode = if app.edit.is_some() {
                InputMode::Edit
            } else {
                InputMode::Normal
            };
        }
        _ => {}
    }
}

fn build_pending(app: &mut App) {
    let Some(img) = app.current_image() else {
        app.status.err("no image to assign");
        return;
    };
    let image_path = img.path.clone();
    let image_name = img.name.clone();

    let target_ids = app.target_track_ids();
    if target_ids.is_empty() {
        app.status.err("no tracks selected");
        return;
    }

    let art = match Art::load(&image_path) {
        Ok(a) => a,
        Err(e) => {
            app.status.err(format!("load image failed: {e}"));
            return;
        }
    };

    let mut plans = Vec::new();
    let mut failures = Vec::new();
    for id in &target_ids {
        let Some(row) = app.tracks.iter().find(|t| &t.id == id) else {
            continue;
        };
        match build_plan(&row.path, Changes::art_only(art.clone())) {
            Ok(plan) => plans.push(plan),
            Err(e) => failures.push((row.title.clone(), e.to_string())),
        }
    }

    if plans.is_empty() {
        app.status.err("no applicable tracks");
        return;
    }
    app.pending = Some(PendingBatch {
        summary: format!("cover: {image_name}"),
        plans,
        failures,
    });
    app.mode = InputMode::Confirm;
}

fn apply_pending(app: &mut App) {
    let Some(batch) = app.pending.take() else {
        app.mode = InputMode::Normal;
        return;
    };
    let total = batch.plans.len();
    let summary = batch.summary.clone();
    let mut errs: Vec<String> = Vec::new();
    let mut backup_hint = None;
    for plan in &batch.plans {
        match apply_plan(plan, app.backup) {
            Ok(path) => {
                if backup_hint.is_none() {
                    if let Some(p) = path {
                        backup_hint = Some(p);
                    }
                }
            }
            Err(e) => errs.push(format!("\"{}\": {e}", plan.track.display_title())),
        }
    }
    let ok = total - errs.len();

    app.trk_col.selected.clear();
    app.edit = None;
    app.mode = InputMode::Normal;
    if let Err(e) = app.reload() {
        app.status.warn(format!("reload after apply failed: {e}"));
    }

    let hint = backup_hint
        .map(|p| {
            let dir = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
            format!(" Backups in {dir}")
        })
        .unwrap_or_default();
    if errs.is_empty() {
        app.status.ok(format!("Wrote {summary} to {ok}/{total}.{hint}"));
    } else {
        app.unresolved_errors = true;
        let extra = if errs.len() > 1 {
            format!(" (+{} more)", errs.len() - 1)
        } else {
            String::new()
        };
        app.status
            .err(format!("Wrote {ok}/{total}. Failed → {}{extra}{hint}", errs[0]));
    }
}
