use std::{
    env,
    io::{self, Read, Write},
    os::fd::AsRawFd,
    os::fd::BorrowedFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    cursor,
    terminal,
};
use nix::poll::{poll, PollFd, PollFlags};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use signal_hook::{consts::signal::SIGWINCH, flag as signal_flag};

use super::AppTerminal;

pub(crate) fn hide_to_shell_toggleable(terminal: &mut AppTerminal) -> Result<()> {
    // We keep raw mode enabled and act like a minimal terminal multiplexer (tmux-like):
    // forward *raw stdin bytes* to a PTY-backed shell, but intercept F12 to return to Trix.
    // This avoids lossy key mapping and makes the subshell feel like a real terminal.

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
            "\nTrix hidden. Press F12 again to return (or type 'exit').\n"
        )?;
        out.flush().ok();
    }

    let mut pty_writer = pair.master.take_writer().context("pty take writer")?;
    let mut pty_reader = pair.master.try_clone_reader().context("pty clone reader")?;

    // Track window-size changes so the shell gets correct $COLUMNS/$LINES behavior.
    let winch = Arc::new(AtomicBool::new(false));
    signal_flag::register(SIGWINCH, Arc::clone(&winch)).ok();

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

    // Forward raw user input bytes into the PTY.
    // Intercept F12 (commonly sent as ESC [ 2 4 ~) to return.
    let stdin_fd = io::stdin().as_raw_fd();
    // Safety: stdin_fd is a valid FD for the life of this function.
    let stdin_borrowed = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
    let mut poll_fds = [PollFd::new(stdin_borrowed, PollFlags::POLLIN)];
    let mut stdin = io::stdin();

    let mut pending_esc = false;
    let mut pending_esc_since: Option<Instant> = None;
    let mut esc_buf: Vec<u8> = Vec::new();
    let mut stdin_buf = [0u8; 4096];

    // Most xterm-compatible terminals send F12 as ESC [ 2 4 ~.
    // We treat this as the hide/unhide toggle while the subshell is active.
    const F12_SEQ: &[u8] = b"[24~";

    loop {
        // If the shell exited, return to the TUI.
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }

        // Apply resize if we saw a SIGWINCH.
        if winch.swap(false, Ordering::Relaxed) {
            if let Ok((cols, rows)) = terminal::size() {
                let _ = pair.master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }

        // If the user pressed ESC alone, don't wait forever for a following byte.
        if pending_esc {
            if let Some(since) = pending_esc_since {
                if since.elapsed() >= Duration::from_millis(40) {
                    // Flush a bare ESC or an incomplete escape sequence.
                    if esc_buf.is_empty() {
                        let _ = pty_writer.write_all(&[0x1b]);
                    } else {
                        let _ = pty_writer.write_all(&[0x1b]);
                        let _ = pty_writer.write_all(&esc_buf);
                        esc_buf.clear();
                    }
                    let _ = pty_writer.flush();
                    pending_esc = false;
                    pending_esc_since = None;
                }
            }
        }

        // Poll stdin so we can also periodically check child exit + SIGWINCH.
        match poll(&mut poll_fds, 50u16) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => {
                let _ = child.kill();
                let _ = out_thread.join();
                return Err(anyhow::Error::new(e)).context("poll stdin while hidden");
            }
        }

        let ready = poll_fds
            .get(0)
            .and_then(|fd| fd.revents())
            .map(|ev| ev.contains(PollFlags::POLLIN))
            .unwrap_or(false);

        if !ready {
            continue;
        }

        let n = match stdin.read(&mut stdin_buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = child.kill();
                let _ = out_thread.join();
                return Err(anyhow::Error::new(e)).context("read stdin bytes while hidden");
            }
        };

        for &b in &stdin_buf[..n] {
            if pending_esc {
                esc_buf.push(b);

                // Check for F12 sequence (ESC + [24~).
                if esc_buf.len() <= F12_SEQ.len() && esc_buf == F12_SEQ[..esc_buf.len()] {
                    if esc_buf.len() == F12_SEQ.len() {
                        // Toggle back: terminate the shell and return.
                        let _ = child.kill();
                        let _ = child.wait();
                        pending_esc = false;
                        pending_esc_since = None;
                        esc_buf.clear();
                        break;
                    }

                    // Still matching the prefix; keep waiting for more bytes.
                    pending_esc_since = Some(Instant::now());
                    continue;
                }

                // Not a recognized sequence: forward ESC + buffered bytes to the PTY.
                let _ = pty_writer.write_all(&[0x1b]);
                let _ = pty_writer.write_all(&esc_buf);
                esc_buf.clear();
                pending_esc = false;
                pending_esc_since = None;
                continue;
            }

            if b == 0x1b {
                pending_esc = true;
                pending_esc_since = Some(Instant::now());
                esc_buf.clear();
                continue;
            }

            let _ = pty_writer.write_all(&[b]);
        }
        let _ = pty_writer.flush();
    }

    let _ = out_thread.join();

    // Restore the TUI.
    {
        let backend = terminal.backend_mut();
        crossterm::execute!(backend, terminal::EnterAlternateScreen, terminal::Clear(terminal::ClearType::All), cursor::Hide)
            .context("enter alternate screen")?;
        let _ = std::io::Write::flush(backend);
    }
    terminal.clear().ok();
    Ok(())
}
