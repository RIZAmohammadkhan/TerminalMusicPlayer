use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, Event},
};
use signal_hook::{consts::signal::*, iterator::Signals};

use crate::{
    audio::AudioOutput,
    config::Config,
    library::{default_library_path, discover_tracks},
    player::Player,
    term::{hide_to_shell_toggleable, init_terminal, TerminalCleanup},
    ui::{draw_ui, handle_key, UiAction, UiState},
};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Directory (or file) to play.
    ///
    /// If omitted: uses XDG music dir (if configured), else ~/Music, else the current directory.
    #[arg(value_name = "PATH")]
    path: Option<std::path::PathBuf>,

    /// Start at this track index (0-based)
    #[arg(long, default_value_t = 0)]
    index: usize,
}

pub(crate) fn run() -> Result<()> {
    TerminalCleanup::install_panic_hook();

    let args = Args::parse();

    let config = Config::load();
    let theme = config.theme;

    let library_path = args.path.unwrap_or_else(default_library_path);

    // Low-latency audio output (small fixed buffers) so stop is immediate.
    let audio = AudioOutput::new_low_latency().context("Failed to initialize audio output")?;
    let audio_ctl = audio.control();

    // Handle SIGINT/SIGTERM/SIGHUP promptly.
    // Stop audio directly from the signal thread to avoid waiting for the UI tick.
    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let shutdown = Arc::clone(&shutdown);
        let audio_ctl = audio_ctl.clone();
        let mut signals = Signals::new([SIGINT, SIGTERM, SIGHUP, SIGQUIT])
            .context("Failed to create signal watcher")?;

        std::thread::spawn(move || {
            for sig in signals.forever() {
                shutdown.store(true, Ordering::Relaxed);
                audio_ctl.shutdown_now();
                // Terminal close should behave like a clean quit.
                // We don't call process::exit so terminal cleanup can still run.
                if sig == SIGHUP {
                    // nothing else to do here
                }
            }
        });
    }

    let tracks = discover_tracks(&library_path)?;
    let mut player = Player::new(tracks, args.index, audio)?;

    // Auto-start first track if any
    if player.has_tracks() {
        player.start_track(Duration::ZERO)?;
    }

    let mut terminal = init_terminal()?;
    let _cleanup = TerminalCleanup;

    let mut ui = UiState::new();

    let tick_rate = Duration::from_millis(50);
    loop {
        if shutdown.load(Ordering::Relaxed) {
            audio_ctl.shutdown_now();
            player.stop_playback();
            break;
        }

        player.refresh_volume();

        if terminal.draw(|f| draw_ui(f, &player, &ui, &theme)).is_err() {
            // Terminal likely closed (broken pipe / pty hangup). Treat as a clean quit.
            audio_ctl.shutdown_now();
            player.stop_playback();
            break;
        }

        // Auto-advance
        if !player.loop_current && player.is_track_finished() {
            let _ = player.next_track();
        }

        let timeout = tick_rate
            .checked_sub(ui.last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        let polled = match event::poll(timeout) {
            Ok(v) => v,
            Err(_) => {
                audio_ctl.shutdown_now();
                player.stop_playback();
                break;
            }
        };

        if polled {
            if shutdown.load(Ordering::Relaxed) {
                audio_ctl.shutdown_now();
                player.stop_playback();
                break;
            }

            let ev = match event::read() {
                Ok(v) => v,
                Err(_) => {
                    audio_ctl.shutdown_now();
                    player.stop_playback();
                    break;
                }
            };

            if let Event::Key(key) = ev {
                match handle_key(key, &mut player, &mut ui)? {
                    UiAction::None => {}
                    UiAction::Quit => break,
                    UiAction::HideToShell => {
                        ui.reset_transient();

                        // Temporarily hide the TUI and run a subshell in a PTY.
                        // Press F12 again (or exit the shell) to return.
                        // Audio playback continues.
                        if let Err(e) = hide_to_shell_toggleable(&mut terminal) {
                            // If the terminal was closed while hidden, treat it as a clean quit.
                            audio_ctl.shutdown_now();
                            player.stop_playback();
                            eprintln!("trix: hide failed: {e:#}");
                            break;
                        }
                    }
                }
            }
        }

        if ui.last_tick.elapsed() >= tick_rate {
            ui.last_tick = Instant::now();
        }
    }

    drop(terminal);
    Ok(())
}
