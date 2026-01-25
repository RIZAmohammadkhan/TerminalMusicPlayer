use std::io::{self, Write};

use anyhow::{Context, Result};
use crossterm::{cursor, terminal};
use ratatui::{backend::CrosstermBackend, Terminal};

pub(crate) type AppTerminal = Terminal<CrosstermBackend<io::Stdout>>;

pub(crate) mod shell;
pub(crate) use shell::hide_to_shell_toggleable;

pub(crate) fn init_terminal() -> Result<AppTerminal> {
    terminal::enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)
        .context("enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;
    terminal.clear().ok();
    Ok(terminal)
}

fn restore_terminal_minimal() {
    let _ = terminal::disable_raw_mode();
    let mut stdout = io::stdout();
    let _ = crossterm::execute!(stdout, terminal::LeaveAlternateScreen, cursor::Show);
    let _ = stdout.flush();
}

pub(crate) struct TerminalCleanup;

impl TerminalCleanup {
    pub(crate) fn install_panic_hook() {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal_minimal();
            default_hook(info);
        }));
    }
}

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        restore_terminal_minimal();
    }
}
