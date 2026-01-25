use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use ratatui::prelude::Rect;

use crate::{
    player::Player,
    util::{fmt_time, parse_timestamp},
};

use super::render::{help_overlay_rect, help_wrapped_lines};

#[derive(Debug)]
pub(crate) struct UiState {
    pub(crate) volume_mode: bool,
    pub(crate) show_help: bool,
    pub(crate) help_scroll: u16,
    pub(crate) search_mode: bool,
    pub(crate) search_query: String,
    pub(crate) move_mode: bool,
    pub(crate) move_query: String,
    pub(crate) move_error: Option<String>,
    last_seek_key: Option<KeyCode>,
    last_seek_at: Instant,
    pub(crate) delete_confirm: Option<DeleteConfirm>,
    pub(crate) last_tick: Instant,
}

#[derive(Debug, Clone)]
pub(crate) struct DeleteConfirm {
    pub(crate) index: usize,
    pub(crate) started_at: Instant,
}

impl UiState {
    pub(crate) fn new() -> Self {
        Self {
            volume_mode: false,
            show_help: false,
            help_scroll: 0,
            search_mode: false,
            search_query: String::new(),
            move_mode: false,
            move_query: String::new(),
            move_error: None,
            last_seek_key: None,
            last_seek_at: Instant::now() - Duration::from_millis(500),
            delete_confirm: None,
            last_tick: Instant::now(),
        }
    }

    pub(crate) fn reset_transient(&mut self) {
        // Cancel transient UI modes so the user returns to a clean state.
        self.volume_mode = false;
        self.show_help = false;
        self.help_scroll = 0;
        self.search_mode = false;
        self.search_query.clear();
        self.move_mode = false;
        self.move_query.clear();
        self.move_error = None;
        self.delete_confirm = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiAction {
    None,
    Quit,
    HideToShell,
}

pub(crate) fn handle_key(key: KeyEvent, player: &mut Player, ui: &mut UiState) -> Result<UiAction> {
    // Some terminals report key holding as Repeat, others as rapid Press.
    // We treat Repeat as non-actionable and also apply a short cooldown for seek keys.
    let is_press = key.kind == KeyEventKind::Press;
    let is_repeat = key.kind == KeyEventKind::Repeat;

    // Hide-to-shell toggle: Ctrl+Alt+x hides/unhides.
    // Note: some terminals may report Ctrl+Alt+x as Backspace with CONTROL+ALT modifiers.
    if is_press
        && ((key.code == KeyCode::Char('x')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.modifiers.contains(KeyModifiers::ALT))
            || (key.code == KeyCode::Backspace
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && key.modifiers.contains(KeyModifiers::ALT)))
    {
        return Ok(UiAction::HideToShell);
    }

    // Ctrl+C quit
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        player.stop_playback();
        return Ok(UiAction::Quit);
    }

    // Search mode captures all typing so it doesn't trigger other bindings.
    if ui.search_mode {
        match key.code {
            KeyCode::Esc => {
                ui.search_mode = false;
                ui.search_query.clear();
            }
            KeyCode::Enter => {
                // Confirm selection (and play) then exit search.
                let _ = player.play_selected();
                ui.search_mode = false;
            }
            KeyCode::Backspace => {
                ui.search_query.pop();
                apply_search_selection(player, &ui.search_query);
            }
            KeyCode::Char(c) => {
                // Ignore control chords; accept everything else as input.
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    ui.search_query.push(c);
                    apply_search_selection(player, &ui.search_query);
                }
            }
            _ => {}
        }

        return Ok(UiAction::None);
    }

    // Move mode captures all typing so it doesn't trigger other bindings.
    if ui.move_mode {
        match key.code {
            KeyCode::Esc => {
                ui.move_mode = false;
                ui.move_query.clear();
                ui.move_error = None;
            }
            KeyCode::Enter => {
                match parse_timestamp(&ui.move_query) {
                    Ok(target) => {
                        if let Some(total) = player.total_duration {
                            if target > total {
                                ui.move_error = Some(format!(
                                    "Timestamp is past track length ({}).",
                                    fmt_time(total)
                                ));
                                return Ok(UiAction::None);
                            }
                        }

                        if let Err(e) = player.start_track(target) {
                            ui.move_error = Some(format!("Failed to seek: {e}"));
                            return Ok(UiAction::None);
                        }

                        ui.move_mode = false;
                        ui.move_query.clear();
                        ui.move_error = None;
                    }
                    Err(msg) => {
                        ui.move_error = Some(msg);
                    }
                }
            }
            KeyCode::Backspace => {
                ui.move_query.pop();
                ui.move_error = None;
            }
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    ui.move_query.push(c);
                    ui.move_error = None;
                }
            }
            _ => {}
        }

        return Ok(UiAction::None);
    }

    // Enter search mode.
    if key.code == KeyCode::Char('S') {
        ui.search_mode = true;
        ui.search_query.clear();
        ui.move_mode = false;
        ui.move_query.clear();
        ui.move_error = None;
        ui.delete_confirm = None;
        return Ok(UiAction::None);
    }

    // Enter move mode.
    if key.code == KeyCode::Char('m') {
        ui.move_mode = true;
        ui.move_query.clear();
        ui.move_error = None;
        ui.search_mode = false;
        ui.search_query.clear();
        ui.delete_confirm = None;
        return Ok(UiAction::None);
    }

    // Quit
    if key.code == KeyCode::Char('q') {
        player.stop_playback();
        return Ok(UiAction::Quit);
    }

    // Cancel pending delete confirmation.
    if key.code == KeyCode::Esc {
        ui.delete_confirm = None;
    }

    // Help overlay toggle.
    if matches!(key.code, KeyCode::Char('h') | KeyCode::Char('?')) {
        ui.show_help = !ui.show_help;
        if ui.show_help {
            ui.help_scroll = 0;
        }
        return Ok(UiAction::None);
    }

    // While help is open, keep playback running; allow scrolling + close.
    if ui.show_help {
        // Determine current page size and scroll limits based on terminal size.
        let (page, max_scroll) = if let Ok((cols, rows)) = terminal::size() {
            let area = Rect {
                x: 0,
                y: 0,
                width: cols,
                height: rows,
            };
            let overlay = help_overlay_rect(area);
            let inner_h = overlay.height.saturating_sub(2) as usize;
            let inner_w = overlay.width.saturating_sub(2);
            let lines = help_wrapped_lines(ui, inner_w);
            let max_scroll = lines.len().saturating_sub(inner_h);
            (
                (inner_h.saturating_sub(1).max(1)).min(u16::MAX as usize) as u16,
                max_scroll.min(u16::MAX as usize) as u16,
            )
        } else {
            (10, 0)
        };

        match key.code {
            KeyCode::Esc => {
                ui.show_help = false;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                ui.help_scroll = ui.help_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                ui.help_scroll = ui.help_scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                ui.help_scroll = ui.help_scroll.saturating_sub(page);
            }
            KeyCode::PageDown => {
                ui.help_scroll = ui.help_scroll.saturating_add(page);
            }
            KeyCode::Home => {
                ui.help_scroll = 0;
            }
            KeyCode::End => {
                ui.help_scroll = max_scroll;
            }
            _ => {}
        }

        ui.help_scroll = ui.help_scroll.min(max_scroll);
        return Ok(UiAction::None);
    }

    // Volume mode: toggle with `v`, adjust with Up/Down until toggled off.
    // Note: terminals don't provide a "v is held" modifier, so this is the most reliable UX.
    if ui.volume_mode {
        match key.code {
            KeyCode::Up => {
                player.adjust_volume(0.05);
                return Ok(UiAction::None);
            }
            KeyCode::Down => {
                player.adjust_volume(-0.05);
                return Ok(UiAction::None);
            }
            KeyCode::Esc | KeyCode::Char('v') => {
                ui.volume_mode = false;
                return Ok(UiAction::None);
            }
            _ => {
                // fall through to allow other keys while in volume mode
            }
        }
    }

    match key.code {
        // Requested bindings
        KeyCode::Char('p') => {
            // 10s back
            if is_press {
                let _ = player.seek_relative(-10_000);
            }
        }
        KeyCode::Char('n') => {
            // 10s forward
            if is_press {
                let _ = player.seek_relative(10_000);
            }
        }
        KeyCode::Char('P') => {
            let _ = player.prev_track();
        }
        KeyCode::Char('N') => {
            let _ = player.next_track();
        }
        KeyCode::Left => {
            if is_repeat {
                return Ok(UiAction::None);
            }

            // Ignore continuous holds even if the terminal reports them as Press.
            let cooldown = Duration::from_millis(180);
            if ui.last_seek_key == Some(KeyCode::Left) && ui.last_seek_at.elapsed() < cooldown {
                return Ok(UiAction::None);
            }

            if is_press {
                ui.last_seek_key = Some(KeyCode::Left);
                ui.last_seek_at = Instant::now();
                let _ = player.seek_relative(-5_000);
            }
        }
        KeyCode::Right => {
            if is_repeat {
                return Ok(UiAction::None);
            }

            let cooldown = Duration::from_millis(180);
            if ui.last_seek_key == Some(KeyCode::Right) && ui.last_seek_at.elapsed() < cooldown {
                return Ok(UiAction::None);
            }

            if is_press {
                ui.last_seek_key = Some(KeyCode::Right);
                ui.last_seek_at = Instant::now();
                let _ = player.seek_relative(5_000);
            }
        }
        KeyCode::Char('v') => {
            ui.volume_mode = !ui.volume_mode;
        }
        KeyCode::Char('r') => {
            let _ = player.restart_current();
        }
        KeyCode::Char('l') => {
            let _ = player.toggle_loop_selected();
        }
        KeyCode::Char('s') => {
            player.toggle_shuffle();
        }
        KeyCode::Char('D') => {
            if !player.has_tracks() {
                return Ok(UiAction::None);
            }

            let ttl = Duration::from_millis(2500);
            let idx = player.selected;

            if let Some(confirm) = &ui.delete_confirm {
                if confirm.index == idx && confirm.started_at.elapsed() <= ttl {
                    ui.delete_confirm = None;
                    player.delete_selected()?;
                } else {
                    ui.delete_confirm = Some(DeleteConfirm {
                        index: idx,
                        started_at: Instant::now(),
                    });
                }
            } else {
                ui.delete_confirm = Some(DeleteConfirm {
                    index: idx,
                    started_at: Instant::now(),
                });
            }
        }

        // Nice-to-have navigation
        KeyCode::Up => {
            if ui.volume_mode {
                player.adjust_volume(0.05);
            } else {
                player.select_up();
                ui.delete_confirm = None;
            }
        }
        KeyCode::Down => {
            if ui.volume_mode {
                player.adjust_volume(-0.05);
            } else {
                player.select_down();
                ui.delete_confirm = None;
            }
        }
        KeyCode::Enter => {
            player.play_selected()?;
            ui.delete_confirm = None;
        }
        KeyCode::Char(' ') => {
            player.toggle_pause();
        }
        _ => {}
    }

    Ok(UiAction::None)
}

fn apply_search_selection(player: &mut Player, query: &str) {
    let q = query.trim();
    if q.is_empty() {
        return;
    }

    let q = q.to_ascii_lowercase();
    if let Some((idx, _)) = player
        .tracks
        .iter()
        .enumerate()
        .find(|(_, t)| t.display_name.to_ascii_lowercase().contains(&q))
    {
        player.selected = idx;
    }
}
