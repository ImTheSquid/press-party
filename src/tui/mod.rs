pub mod app;
pub mod data;
pub mod events;
pub mod render;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui_image::picker::Picker;

use app::App;

pub fn run(root: PathBuf, recurse: bool) -> Result<()> {
    // Query the terminal for its graphics protocol and font cell size BEFORE we
    // enter the alternate screen / raw mode. Falls back to a unicode half-block
    // renderer with a guessed cell size if the terminal won't answer.
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());

    let mut app = App::new(root, recurse, picker)?;
    let mut terminal = setup_terminal()?;
    install_panic_hook();

    let result = event_loop(&mut terminal, &mut app);

    let restore = restore_terminal(&mut terminal);
    result.and(restore)
}

type Term = Terminal<CrosstermBackend<io::Stdout>>;

fn event_loop(terminal: &mut Term, app: &mut App) -> Result<()> {
    let mut was_modal = false;
    while !app.should_quit {
        app.sync_preview();
        terminal.draw(|f| render::draw(f, app))?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                events::handle_key(app, key);
            }
        }

        // Closing a popup leaves a hole over the image: graphics protocols like
        // iTerm2/sixel emit the whole picture anchored to one cell and only
        // resend when that cell changes, so the region the popup overwrote is
        // never repainted. Force a full redraw and re-encode on the close edge.
        let now_modal = app.modal_open();
        if was_modal && !now_modal {
            // Invalidate the back buffer so the next draw repaints every cell,
            // re-sending the image's cached escape sequence. No need to rebuild
            // the protocol — that would re-decode the original full-res file.
            terminal.clear()?;
        }
        was_modal = now_modal;
    }
    Ok(())
}

fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));
}
