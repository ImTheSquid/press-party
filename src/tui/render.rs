use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::{Resize, StatefulImage};

use crate::format::{format_duration, format_size};

use super::app::{App, EDIT_LABELS, Focus, InputMode, StatusLevel};

pub fn draw(f: &mut Frame, app: &mut App) {
    let outer = Layout::vertical([
        Constraint::Length(1), // top bar
        Constraint::Min(0),    // body
        Constraint::Length(2), // status
    ])
    .split(f.area());

    draw_top_bar(f, outer[0], app);

    let body = Layout::vertical([
        Constraint::Percentage(48), // columns
        Constraint::Percentage(52), // preview
    ])
    .split(outer[1]);

    let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(body[0]);
    draw_image_column(f, cols[0], app);
    draw_track_column(f, cols[1], app);

    draw_preview(f, body[1], app);
    draw_status(f, outer[2], app);

    match app.mode {
        InputMode::Edit => draw_edit(f, app),
        InputMode::Confirm => draw_confirm(f, app),
        InputMode::Help => draw_help(f),
        _ => {}
    }
}

fn draw_top_bar(f: &mut Frame, area: Rect, app: &App) {
    let title = Span::styled("press-party", Style::new().bold().cyan());
    let sel = app.trk_col.selected.len();
    let mid = format!(
        "  {} tracks  {} images  selected={}  backup={}",
        app.tracks.len(),
        app.images.len(),
        sel,
        if app.backup { "ON" } else { "off" },
    );
    let line = Line::from(vec![title, Span::styled(mid, Style::new().fg(Color::Gray))]);
    f.render_widget(Paragraph::new(line), area);
}

fn column_block(label: &str, count: usize, focused: bool) -> Block<'static> {
    let border_style = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" {label} ({count}) "))
}

fn draw_search_bar(f: &mut Frame, area: Rect, query: &str, active: bool) {
    let caret = if active { "_" } else { "" };
    let text = format!(" / {query}{caret}");
    let style = if active {
        Style::new().bold()
    } else {
        Style::new().dim()
    };
    f.render_widget(Paragraph::new(text).style(style), area);
}

fn draw_image_column(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Images;
    let block = column_block("IMAGES", app.img_col.visible.len(), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    let active = matches!(app.mode, InputMode::Search(Focus::Images));
    draw_search_bar(f, parts[0], &app.img_col.query, active);

    let dim = Style::new().add_modifier(Modifier::DIM);
    let mut items: Vec<ListItem> = Vec::with_capacity(app.img_col.visible.len());
    for (pos, &idx) in app.img_col.visible.iter().enumerate() {
        let row = &app.images[idx];
        let marked = pos == app.img_col.cursor;
        let mark = if marked {
            Span::styled("✓ ", Style::new().fg(Color::Green).bold())
        } else {
            Span::raw("  ")
        };
        let dims = row
            .dims
            .map(|(w, h)| format!("{w}×{h}"))
            .unwrap_or_else(|| "?".into());
        let line = Line::from(vec![
            mark,
            Span::styled(row.name.clone(), Style::new().bold()),
            Span::styled(
                format!("  {dims}  {}", format_size(row.size)),
                Style::new().fg(Color::DarkGray),
            ),
        ]);
        let style = if !focused && !marked { dim } else { Style::new() };
        items.push(ListItem::new(line).style(style));
    }

    let mut state = ListState::default();
    if !app.img_col.visible.is_empty() {
        state.select(Some(app.img_col.cursor));
    }
    let hl = highlight_style(focused);
    f.render_stateful_widget(List::new(items).highlight_style(hl), parts[1], &mut state);
}

fn draw_track_column(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Tracks;
    let block = column_block("TRACKS", app.trk_col.visible.len(), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    let active = matches!(app.mode, InputMode::Search(Focus::Tracks));
    draw_search_bar(f, parts[0], &app.trk_col.query, active);

    let dim = Style::new().add_modifier(Modifier::DIM);
    let mut items: Vec<ListItem> = Vec::with_capacity(app.trk_col.visible.len() * 2);
    for (pos, &idx) in app.trk_col.visible.iter().enumerate() {
        let row = &app.tracks[idx];
        let selected = app.trk_col.selected.contains(&row.id);
        let is_target =
            selected || (app.trk_col.selected.is_empty() && pos == app.trk_col.cursor);
        let style = if !focused && !is_target { dim } else { Style::new() };

        let mark = if selected {
            Span::styled("✓ ", Style::new().fg(Color::Green).bold())
        } else {
            Span::raw("  ")
        };
        let artist = if row.artist.is_empty() {
            "—".to_string()
        } else {
            row.artist.clone()
        };
        let line1 = Line::from(vec![
            mark,
            Span::styled(row.title.clone(), Style::new().bold()),
            Span::styled(format!("  —  {artist}"), Style::new().fg(Color::Gray)),
            Span::styled(format!("  [{}]", row.format), Style::new().fg(Color::DarkGray)),
        ]);
        let art_span = if row.art_count > 0 {
            Span::styled(
                format!("{} art", row.art_count),
                Style::new().fg(Color::Green),
            )
        } else {
            Span::styled("no art", Style::new().fg(Color::Yellow))
        };
        let line2 = Line::from(vec![
            Span::raw("    "),
            art_span,
            Span::raw("   "),
            Span::styled(
                format_duration(row.duration_secs),
                Style::new().fg(Color::Magenta),
            ),
        ]);
        items.push(ListItem::new(line1).style(style));
        items.push(ListItem::new(line2).style(style));
    }

    let mut state = ListState::default();
    if !app.trk_col.visible.is_empty() {
        state.select(Some(app.trk_col.cursor * 2));
    }
    let hl = highlight_style(focused);
    f.render_stateful_widget(List::new(items).highlight_style(hl), parts[1], &mut state);
}

fn highlight_style(focused: bool) -> Style {
    if focused {
        Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new()
    }
}

fn draw_preview(f: &mut Frame, area: Rect, app: &mut App) {
    // The preview reflects focus: the image to assign, or the cursored track's
    // embedded cover. Title says which so the source is never ambiguous.
    let title = match app.focus {
        Focus::Images => " PREVIEW ".to_string(),
        Focus::Tracks => match app.current_track() {
            Some(t) => format!(" PREVIEW · {} (embedded) ", t.title),
            None => " PREVIEW ".to_string(),
        },
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::DarkGray))
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let split = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(inner);
    let image_area = split[0];
    let info_area = split[1];

    // Info side (immutable borrow) first.
    draw_preview_info(f, info_area, app);

    // Image side (mutable borrow of the cached protocol).
    let loading = app.preview_loading.is_some();
    let failed = app.current_preview_failed();
    let empty = app.current_preview_empty();
    let on_tracks = app.focus == Focus::Tracks;
    if let Some(preview) = app.preview.as_mut() {
        let widget = StatefulImage::default().resize(Resize::Fit(None));
        f.render_stateful_widget(widget, image_area, &mut preview.protocol);
    } else {
        // No image to draw. A graphics-protocol bitmap (iTerm2/kitty/sixel) from
        // a previous frame persists until its cells are overwritten, so wipe the
        // whole area with Clear — a short placeholder Paragraph alone would leave
        // most cells untouched and the old cover would linger.
        f.render_widget(Clear, image_area);
        let msg = if failed {
            "failed to load image"
        } else if empty {
            "no embedded art"
        } else if loading {
            "loading…"
        } else if on_tracks {
            "no track selected"
        } else {
            "no image selected"
        };
        let para = Paragraph::new(msg)
            .style(Style::new().fg(Color::DarkGray))
            .wrap(Wrap { trim: true });
        f.render_widget(para, image_area);
    }
}

fn draw_preview_info(f: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    match app.focus {
        // On IMAGES: describe the image file being previewed (the one to assign).
        Focus::Images => match app.current_image() {
            Some(img) => {
                lines.push(Line::styled(
                    img.name.clone(),
                    Style::new().bold().fg(Color::Cyan),
                ));
                let dims = img
                    .dims
                    .map(|(w, h)| format!("{w}×{h}"))
                    .unwrap_or_else(|| "unknown".into());
                lines.push(Line::from(format!("{dims}   {}", format_size(img.size))));
            }
            None => lines.push(Line::styled("no image", Style::new().fg(Color::DarkGray))),
        },
        // On TRACKS: describe the cursored track whose embedded cover we show.
        Focus::Tracks => match app.current_track() {
            Some(t) => {
                lines.push(Line::styled(
                    t.title.clone(),
                    Style::new().bold().fg(Color::Cyan),
                ));
                let art = if t.art_count > 0 {
                    format!("{} embedded", t.art_count)
                } else {
                    "no cover".to_string()
                };
                lines.push(Line::from(format!("[{}]   {art}", t.format)));
            }
            None => lines.push(Line::styled("no track", Style::new().fg(Color::DarkGray))),
        },
    }
    lines.push(Line::from(""));

    let targets = app.target_track_ids();
    let label = if app.trk_col.selected.is_empty() {
        "cursor track"
    } else {
        "selected"
    };
    lines.push(Line::from(vec![
        Span::styled("assign → ", Style::new().fg(Color::Gray)),
        Span::styled(
            format!("{} {label}", targets.len()),
            Style::new().bold().fg(Color::Green),
        ),
    ]));
    if let Some(t) = app.current_track() {
        lines.push(Line::styled(
            format!("• {}", t.title),
            Style::new().fg(Color::Gray),
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "enter → assign cover",
        Style::new().fg(Color::DarkGray),
    ));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    let hints = "tab focus  / search  space select  e edit  c clear  b backup  R rescan  enter assign  ? help  q quit";
    f.render_widget(
        Paragraph::new(hints).style(Style::new().fg(Color::DarkGray)),
        parts[0],
    );
    let style = match app.status.level {
        StatusLevel::Info => Style::new().fg(Color::Gray),
        StatusLevel::Ok => Style::new().fg(Color::Green),
        StatusLevel::Warn => Style::new().fg(Color::Yellow),
        StatusLevel::Err => Style::new().fg(Color::Red).bold(),
    };
    f.render_widget(Paragraph::new(app.status.text.as_str()).style(style), parts[1]);
}

fn draw_confirm(f: &mut Frame, app: &App) {
    let Some(batch) = app.pending.as_ref() else {
        return;
    };
    let area = popup_area(f.area(), 80, 70);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Yellow))
        .title(" CONFIRM ");

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("changing: ", Style::new().fg(Color::Gray)),
        Span::styled(batch.summary.clone(), Style::new().bold().cyan()),
    ]));
    lines.push(Line::from(""));
    for p in &batch.plans {
        lines.push(Line::from(vec![
            Span::raw("  → "),
            Span::styled(p.track.display_title().to_string(), Style::new().bold()),
        ]));
    }
    if !batch.failures.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled("SKIPPED:", Style::new().fg(Color::Yellow).bold()));
        for (id, err) in &batch.failures {
            lines.push(Line::from(format!("  {id}: {err}")));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!(
            "[y/enter] assign to {}     [n/esc] cancel    backup={}",
            batch.plans.len(),
            if app.backup { "ON" } else { "off" }
        ),
        Style::new().fg(Color::Cyan).bold(),
    ));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false }).block(block);
    f.render_widget(para, area);
}

fn draw_edit(f: &mut Frame, app: &App) {
    let Some(edit) = app.edit.as_ref() else {
        return;
    };
    let area = popup_area(f.area(), 70, 70);
    f.render_widget(Clear, area);

    let scope = if edit.batch {
        format!("EDIT {} TRACKS", edit.targets.len())
    } else {
        "EDIT TRACK".to_string()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Cyan))
        .title(format!(" {scope} "));

    let mut lines: Vec<Line> = Vec::new();
    if edit.batch {
        lines.push(Line::styled(
            "blank field = leave unchanged across all selected tracks",
            Style::new().fg(Color::DarkGray),
        ));
        lines.push(Line::from(""));
    }
    for (i, label) in EDIT_LABELS.iter().enumerate() {
        let focused = i == edit.focus;
        let caret = if focused { "_" } else { "" };
        let label_style = if focused {
            Style::new().bold().fg(Color::Cyan)
        } else {
            Style::new().fg(Color::Gray)
        };
        let value_style = if focused {
            Style::new().bold()
        } else {
            Style::new()
        };
        let shown = if edit.values[i].is_empty() && !focused {
            Span::styled("—", Style::new().fg(Color::DarkGray))
        } else {
            Span::styled(format!("{}{caret}", edit.values[i]), value_style)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{label:>13}: "), label_style),
            shown,
        ]));
    }
    lines.push(Line::from(""));
    if let Some(err) = &edit.error {
        lines.push(Line::styled(
            format!("⚠ {err}"),
            Style::new().fg(Color::Red).bold(),
        ));
    }
    lines.push(Line::styled(
        "tab/↑↓ field   ctrl-u clear   enter save   esc cancel",
        Style::new().fg(Color::DarkGray),
    ));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false }).block(block);
    f.render_widget(para, area);
}

fn draw_help(f: &mut Frame) {
    let area = popup_area(f.area(), 70, 70);
    f.render_widget(Clear, area);
    let body = "\
Tab / Shift-Tab    Switch focus between IMAGES and TRACKS
↑ ↓ / k j          Move cursor
PgUp / PgDn        Page
g / G              Jump top / bottom
/                  Search the focused column (Esc/Enter to leave)
Ctrl-U             Clear search query (in search mode)
Space              Toggle track selection (multi-select)
e                  Edit text tags (title/artist/album/…) for target tracks
c                  Clear track selection
b                  Toggle backup-before-write
R                  Rescan the directory from disk
Enter              Assign the cursored image as cover to target tracks
y / Enter          (Confirm) Apply
n / Esc / q        (Confirm) Cancel
?                  This help
q / Esc            Quit
";
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Cyan))
        .title(" HELP ");
    f.render_widget(Paragraph::new(body).block(block), area);
}

fn popup_area(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}
