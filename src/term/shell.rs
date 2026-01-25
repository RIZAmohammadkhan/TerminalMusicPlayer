use std::{
    env,
    io::{self, Write},
};

use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use super::AppTerminal;

pub(crate) fn hide_to_shell_toggleable(terminal: &mut AppTerminal) -> Result<()> {
    // We keep raw mode enabled and act like a minimal terminal multiplexer (tmux-like):
    // forward keys to a PTY-backed shell, but intercept Ctrl+Alt+x to return to Trix.

    // Leave the TUI so the normal terminal screen is visible.
    {
        let backend = terminal.backend_mut();
        crossterm::execute!(backend, terminal::LeaveAlternateScreen, cursor::Show)
            .context("leave alternate screen")?;
        let _ = std::io::Write::flush(backend);
    }

    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("open pty")?;

    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let cmd = CommandBuilder::new(shell);
    let mut child = pair.slave.spawn_command(cmd).context("spawn shell")?;

    // Print a small hint on the real terminal.
    {
        let mut out = io::stdout();
        writeln!(
            out,
            "\nTrix hidden. Press Ctrl+Alt+x again to return (or type 'exit').\n"
        )?;
        out.flush().ok();
    }

    let mut pty_writer = pair.master.take_writer().context("pty take writer")?;
    let mut pty_reader = pair.master.try_clone_reader().context("pty clone reader")?;

    // Pump PTY output to stdout.
    let out_thread = std::thread::spawn(move || {
        let mut out = io::stdout();
        let mut buf = [0u8; 8192];
        loop {
            match std::io::Read::read(&mut pty_reader, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = std::io::Write::write_all(&mut out, &buf[..n]);
                    let _ = std::io::Write::flush(&mut out);
                }
                Err(_) => break,
            }
        }
    });

    // Forward user input events into the PTY.
    loop {
        // If the shell exited, return to the TUI.
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }

        let ev = match event::read() {
            Ok(v) => v,
            Err(e) => {
                let _ = child.kill();
                let _ = out_thread.join();
                return Err(anyhow::Error::new(e)).context("read terminal event while hidden");
            }
        };

        match ev {
            Event::Key(key) => {
                if key.kind == KeyEventKind::Press
                    && ((key.code == KeyCode::Char('x')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.modifiers.contains(KeyModifiers::ALT))
                        || (key.code == KeyCode::Backspace
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.modifiers.contains(KeyModifiers::ALT)))
                {
                    // Toggle back: terminate the shell and return.
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }

                if let Some(bytes) = key_event_to_pty_bytes(key) {
                    let _ = std::io::Write::write_all(&mut pty_writer, &bytes);
                    let _ = std::io::Write::flush(&mut pty_writer);
                }
            }
            Event::Paste(s) => {
                let _ = std::io::Write::write_all(&mut pty_writer, s.as_bytes());
                let _ = std::io::Write::flush(&mut pty_writer);
            }
            Event::Resize(c, r) => {
                let _ = pair.master.resize(PtySize {
                    rows: r,
                    cols: c,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
            _ => {}
        }
    }

    let _ = out_thread.join();

    // Restore the TUI.
    {
        let backend = terminal.backend_mut();
        crossterm::execute!(backend, terminal::EnterAlternateScreen, cursor::Hide)
            .context("enter alternate screen")?;
        let _ = std::io::Write::flush(backend);
    }
    terminal.clear().ok();
    Ok(())
}

fn key_event_to_pty_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    // Ignore release events.
    if matches!(key.kind, KeyEventKind::Release) {
        return None;
    }

    let mut out: Vec<u8> = Vec::new();

    // ALT sends an ESC prefix in most terminals.
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(c) => {
            // Control chords map into 0x01..0x1A for a..z (plus a few common extras).
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                let lower = c.to_ascii_lowercase();
                if ('a'..='z').contains(&lower) {
                    out.push((lower as u8) & 0x1f);
                } else if lower == '@' {
                    out.push(0);
                } else {
                    return None;
                }
            } else {
                if alt {
                    out.push(0x1b);
                }
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                out.extend_from_slice(s.as_bytes());
            }
        }
        KeyCode::Enter => out.push(b'\r'),
        KeyCode::Tab => out.push(b'\t'),
        KeyCode::Backspace => out.push(0x7f),
        KeyCode::Esc => out.push(0x1b),

        // Common xterm sequences.
        KeyCode::Up => out.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => out.extend_from_slice(b"\x1b[B"),
        KeyCode::Right => out.extend_from_slice(b"\x1b[C"),
        KeyCode::Left => out.extend_from_slice(b"\x1b[D"),
        KeyCode::Home => out.extend_from_slice(b"\x1b[H"),
        KeyCode::End => out.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => out.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => out.extend_from_slice(b"\x1b[6~"),
        KeyCode::Delete => out.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => out.extend_from_slice(b"\x1b[2~"),

        KeyCode::F(n) => {
            // F1..F4 are special; others are CSI sequences.
            match n {
                1 => out.extend_from_slice(b"\x1bOP"),
                2 => out.extend_from_slice(b"\x1bOQ"),
                3 => out.extend_from_slice(b"\x1bOR"),
                4 => out.extend_from_slice(b"\x1bOS"),
                5 => out.extend_from_slice(b"\x1b[15~"),
                6 => out.extend_from_slice(b"\x1b[17~"),
                7 => out.extend_from_slice(b"\x1b[18~"),
                8 => out.extend_from_slice(b"\x1b[19~"),
                9 => out.extend_from_slice(b"\x1b[20~"),
                10 => out.extend_from_slice(b"\x1b[21~"),
                11 => out.extend_from_slice(b"\x1b[23~"),
                12 => out.extend_from_slice(b"\x1b[24~"),
                _ => return None,
            }
        }
        _ => return None,
    }

    Some(out)
}
