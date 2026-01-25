use std::time::Duration;

use ratatui::{
    prelude::*,
    text::{Span, Text},
    widgets::block::Title,
    widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
};

use crate::{
    config::Theme,
    player::{PlayState, Player},
    util::fmt_time,
};

use super::input::UiState;

pub(crate) fn draw_ui(f: &mut Frame, player: &Player, ui: &UiState, theme: &Theme) {
    let area = f.area();

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let title = title_line(player, ui);
    let title_widget = Paragraph::new(title)
        .style(
            Style::default()
                .fg(theme.title_accent)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Terminal Music Player"),
        );
    f.render_widget(title_widget, root[0]);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(root[1]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(mid[0]);

    // Playlist
    let items: Vec<ListItem> = player
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let (prefix, prefix_style) = if i == player.current {
                ("▶ ", Style::default().fg(theme.playing_indicator))
            } else {
                ("  ", Style::default())
            };

            let name_style = if i == player.current {
                Style::default()
                    .fg(theme.current_track_accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::styled(t.display_name.clone(), name_style),
            ]))
        })
        .collect();

    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(player.selected));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.library_accent))
                .title(Title::from(Line::styled(
                    "Library",
                    Style::default()
                        .fg(theme.library_accent)
                        .add_modifier(Modifier::BOLD),
                ))),
        )
        .highlight_style(
            Style::default()
                .fg(theme.library_accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("» ");

    f.render_stateful_widget(list, left[0], &mut state);

    let (box_title, box_border, box_style, box_text) = if ui.move_mode {
        let input = if ui.move_query.is_empty() {
            "Type a timestamp (e.g. 1:30)".to_string()
        } else {
            ui.move_query.clone()
        };

        let text = if let Some(err) = &ui.move_error {
            Text::from(vec![
                Line::styled(err.clone(), Style::default().fg(theme.error)),
                Line::raw(input),
            ])
        } else {
            Text::from(input)
        };

        (
            "Move",
            theme.move_accent,
            Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD),
            text,
        )
    } else {
        let text = if ui.search_mode {
            if ui.search_query.is_empty() {
                Text::from("Type to search…".to_string())
            } else {
                Text::from(ui.search_query.clone())
            }
        } else {
            Text::from("Press S to search".to_string())
        };

        let style = if ui.search_mode {
            Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_muted)
        };

        ("Search", theme.search_accent, style, text)
    };

    let input_widget = Paragraph::new(box_text)
        .style(box_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(box_border))
                .title(Title::from(Line::styled(
                    box_title,
                    Style::default().fg(box_border).add_modifier(Modifier::BOLD),
                ))),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(input_widget, left[1]);

    // Now playing
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(mid[1]);

    let now_playing = now_playing_lines(player, ui, theme);
    let now_widget = Paragraph::new(Text::from(now_playing))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.now_accent))
                .title(Title::from(Line::styled(
                    "Now",
                    Style::default().fg(theme.now_accent).add_modifier(Modifier::BOLD),
                ))),
        );
    f.render_widget(now_widget, right[0]);

    let (ratio, label) = progress(player);
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.progress_accent))
                .title(Title::from(Line::styled(
                    "Progress",
                    Style::default()
                        .fg(theme.progress_accent)
                        .add_modifier(Modifier::BOLD),
                ))),
        )
        .gauge_style(
            Style::default()
                .fg(theme.progress_accent)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(ratio)
        .label(Span::styled(
            label,
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD),
        ));
    f.render_widget(gauge, right[1]);

    let hints = hints_lines(player, ui, theme);
    let help_widget = Paragraph::new(Text::from(hints))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.hints_accent))
                .title(Title::from(Line::styled(
                    "Hints",
                    Style::default()
                        .fg(theme.hints_accent)
                        .add_modifier(Modifier::BOLD),
                ))),
        );
    f.render_widget(help_widget, right[2]);

    if ui.show_help {
        draw_help_overlay(f, player, ui, theme);
    }
}

fn title_line(player: &Player, ui: &UiState) -> String {
    let state = match player.state {
        PlayState::Stopped => "stopped",
        PlayState::Playing => "playing",
        PlayState::Paused => "paused",
    };

    let vol = format!("{:.0}%", player.volume.display() * 100.0);
    let chord = if ui.volume_mode { " (v: volume mode)" } else { "" };

    let lp = if player.loop_current { " • Loop" } else { "" };
    let sh = if player.shuffle { " • Shuffle" } else { "" };
    let backend = player.volume.label();
    format!("State: {state} • Volume: {vol} [{backend}]{chord}{lp}{sh}")
}

fn now_playing_lines(player: &Player, _ui: &UiState, theme: &Theme) -> Vec<Line<'static>> {
    let name = player
        .current_track()
        .map(|t| t.display_name.as_str())
        .unwrap_or("(no tracks)");

    let title = player
        .now_meta
        .title
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(name);
    let artist = player
        .now_meta
        .artist
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("-");
    let album = player
        .now_meta
        .album
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("-");

    let key = key_style(theme);
    let title_style = Style::default()
        .fg(theme.song_title_accent)
        .add_modifier(Modifier::BOLD);

    vec![
        Line::from(vec![
            Span::styled("Track:", key),
            Span::raw(" "),
            Span::styled(title.to_string(), title_style),
        ]),
        Line::from(vec![
            Span::styled("Artist:", key),
            Span::raw(format!(" {artist}")),
        ]),
        Line::from(vec![
            Span::styled("Album:", key),
            Span::raw(format!("  {album}")),
        ]),
        Line::from(vec![
            Span::styled("Index:", key),
            Span::raw(format!(
                "  {} / {}",
                if player.has_tracks() { player.current + 1 } else { 0 },
                player.tracks.len()
            )),
        ]),
    ]
}

fn key_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.key_accent)
        .add_modifier(Modifier::BOLD)
}

fn heading_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.title_accent)
        .add_modifier(Modifier::BOLD)
}

fn hints_lines(player: &Player, ui: &UiState, theme: &Theme) -> Vec<Line<'static>> {
    let key = key_style(theme);

    if ui.search_mode {
        return vec![Line::from(vec![
            Span::styled("Enter", key),
            Span::raw(" play • "),
            Span::styled("Esc", key),
            Span::raw(" cancel • "),
            Span::styled("Backspace", key),
            Span::raw(" delete"),
        ])];
    }

    if ui.move_mode {
        return vec![Line::from(vec![
            Span::styled("Enter", key),
            Span::raw(" jump • "),
            Span::styled("Esc", key),
            Span::raw(" cancel • "),
            Span::styled("Backspace", key),
            Span::raw(" delete"),
        ])];
    }

    if let Some(confirm) = &ui.delete_confirm {
        if confirm.started_at.elapsed() <= Duration::from_millis(2500) {
            let name = player
                .tracks
                .get(confirm.index)
                .map(|t| t.display_name.as_str())
                .unwrap_or("(track)");
            return vec![Line::from(vec![
                Span::raw("Press "),
                Span::styled("D", key),
                Span::raw(" again to delete: "),
                Span::styled(name.to_string(), Style::default().fg(theme.song_title_accent)),
                Span::raw(" • "),
                Span::styled("Esc", key),
                Span::raw(" cancel"),
            ])];
        }
    }

    if ui.volume_mode {
        let what = if player.volume.is_system() {
            "system volume"
        } else {
            "volume"
        };
        return vec![Line::from(vec![
            Span::raw("Volume mode: "),
            Span::styled("↑/↓", key),
            Span::raw(format!(" change {what} • ")),
            Span::styled("v", key),
            Span::raw("/"),
            Span::styled("Esc", key),
            Span::raw(" exit"),
        ])];
    }

    vec![Line::from(vec![
        Span::raw("Press "),
        Span::styled("h", key),
        Span::raw(" for cheatsheet • "),
        Span::styled("F12", key),
        Span::raw(" hide/unhide • "),
        Span::styled("v", key),
        Span::raw(" volume mode • "),
        Span::styled("S", key),
        Span::raw(" search • "),
        Span::styled("m", key),
        Span::raw(" move • "),
        Span::styled("D", key),
        Span::raw(" delete"),
    ])]
}

fn progress(player: &Player) -> (f64, String) {
    let pos = player.position();
    if let Some(total) = player.total_duration {
        let ratio = if total.as_millis() == 0 {
            0.0
        } else {
            (pos.as_secs_f64() / total.as_secs_f64()).clamp(0.0, 1.0)
        };
        let label = format!("{} / {}", fmt_time(pos), fmt_time(total));
        (ratio, label)
    } else {
        (0.0, format!("{} / --:--", fmt_time(pos)))
    }
}

fn help_text(ui: &UiState) -> String {
    let vol_line = if ui.volume_mode {
        "v / Esc      exit volume mode"
    } else {
        "v           toggle volume mode"
    };

    [
        "General",
        "  h or ?      toggle this help",
        "  F12        hide/unhide (shell; press again to return)",
        "  q           quit",
        "  s           toggle shuffle order",
        "  S           search library (type to select)",
        "  m           move to timestamp (e.g. 1:30)",
        "  D           delete selected track (press twice)",
        "  ↑/↓         scroll (PgUp/PgDn, Home/End)",
        "",
        "Playback",
        "  Space       pause/resume",
        "  P / N       previous/next track",
        "  r           restart current track",
        "  l           loop selected/current",
        "",
        "Arrows",
        "  ← / →       seek -5s / +5s",
        "  ↑ / ↓       select track",
        "  (vol mode)  ↑ / ↓ changes volume",
        "",
        "Seek",
        "  p / n       -10s / +10s",
        "",
        "Volume",
        "  v           enter/exit volume mode",
        "  ↑ / ↓       volume (when in volume mode)",
        &format!("  {vol_line}"),
        "",
        "Library",
        "  Enter       play selected",
    ]
    .join("\n")
}

fn draw_help_overlay(f: &mut Frame, player: &Player, ui: &UiState, theme: &Theme) {
    let area = f.area();
    let overlay = help_overlay_rect(area);

    f.render_widget(Clear, overlay);

    let inner_w = overlay.width.saturating_sub(2);
    let lines = help_wrapped_lines(ui, inner_w);
    let total_lines = lines.len();
    let inner_h = overlay.height.saturating_sub(2) as usize;
    let max_scroll = total_lines.saturating_sub(inner_h);
    let scroll = ui.help_scroll.min(max_scroll.min(u16::MAX as usize) as u16);

    let base_header = if player.loop_current {
        "Cheatsheet • Loop ON"
    } else {
        "Cheatsheet"
    };

    let indicator = if total_lines == 0 || inner_h == 0 {
        String::new()
    } else {
        let start = (scroll as usize).saturating_add(1);
        let visible = inner_h.max(1);
        let end = (start.saturating_add(visible).saturating_sub(1)).min(total_lines);
        format!("{start}-{end}/{total_lines}")
    };

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title_top(base_header)
        .style(Style::default().fg(theme.text_primary));

    if !indicator.is_empty() {
        block = block.title_bottom(
            Line::styled(indicator, Style::default().fg(theme.text_muted))
                .alignment(Alignment::Right),
        );
    }

    let styled_lines: Vec<Line<'static>> =
        lines.into_iter().map(|l| stylize_help_line(l, theme)).collect();
    let p = Paragraph::new(Text::from(styled_lines))
        .block(block)
        .scroll((scroll, 0))
        .style(Style::default().fg(theme.text_primary));

    f.render_widget(p, overlay);
}

pub(crate) fn help_overlay_rect(area: Rect) -> Rect {
    // Use almost all terminal space, with a small margin, so the cheatsheet is
    // readable even on small terminals.
    area.inner(Margin {
        vertical: 1,
        horizontal: 2,
    })
}

pub(crate) fn help_wrapped_lines(ui: &UiState, width: u16) -> Vec<String> {
    let raw = help_text(ui);
    let max_width = width.max(1) as usize;

    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }

        let indent_len = line.chars().take_while(|c| *c == ' ').count();
        let indent = " ".repeat(indent_len);
        let content = &line[indent_len..];

        // If the line already fits, keep it as-is.
        if unicode_width::UnicodeWidthStr::width(line) <= max_width {
            out.push(line.to_string());
            continue;
        }

        // If this looks like a "keys  description" help entry, preserve the keys column
        // so it can be styled properly even when wrapped.
        let mut split_at: Option<usize> = None;
        let mut run = 0usize;
        for (i, ch) in content.char_indices() {
            if ch == ' ' {
                run += 1;
                if run >= 2 {
                    split_at = Some(i + 1 - run);
                    break;
                }
            } else {
                run = 0;
            }
        }

        if let Some(at) = split_at {
            let left = content[..at].trim_end();
            let right = content[at..].trim();
            let left_w = unicode_width::UnicodeWidthStr::width(left);
            let pad = 2usize;
            let avail_desc = max_width
                .saturating_sub(indent_len)
                .saturating_sub(left_w)
                .saturating_sub(pad)
                .max(1);

            let mut first = true;
            let mut current = String::new();
            for word in right.split_whitespace() {
                let word_w = unicode_width::UnicodeWidthStr::width(word);
                if current.is_empty() {
                    current.push_str(word);
                } else {
                    let cur_w = unicode_width::UnicodeWidthStr::width(current.as_str());
                    if cur_w + 1 + word_w <= avail_desc {
                        current.push(' ');
                        current.push_str(word);
                    } else {
                        if first {
                            out.push(format!("{indent}{left}  {current}"));
                            first = false;
                        } else {
                            out.push(format!(
                                "{}{}",
                                " ".repeat(indent_len + left_w + pad),
                                current
                            ));
                        }
                        current.clear();
                        current.push_str(word);
                    }
                }
            }

            if !current.is_empty() {
                if first {
                    out.push(format!("{indent}{left}  {current}"));
                } else {
                    out.push(format!(
                        "{}{}",
                        " ".repeat(indent_len + left_w + pad),
                        current
                    ));
                }
            }
            continue;
        }

        // Fallback: generic wrapping.
        let avail = max_width.saturating_sub(indent_len).max(1);
        let mut current = String::new();
        for word in content.split_whitespace() {
            let word_w = unicode_width::UnicodeWidthStr::width(word);
            if current.is_empty() {
                if word_w <= avail {
                    current.push_str(word);
                } else {
                    // Hard-break very long words.
                    let mut chunk = String::new();
                    for ch in word.chars() {
                        let next_w = unicode_width::UnicodeWidthStr::width(chunk.as_str())
                            + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                        if next_w > avail && !chunk.is_empty() {
                            out.push(format!("{indent}{chunk}"));
                            chunk.clear();
                        }
                        chunk.push(ch);
                    }
                    if !chunk.is_empty() {
                        current = chunk;
                    }
                }
            } else {
                let cur_w = unicode_width::UnicodeWidthStr::width(current.as_str());
                if cur_w + 1 + word_w <= avail {
                    current.push(' ');
                    current.push_str(word);
                } else {
                    out.push(format!("{indent}{current}"));
                    current.clear();
                    current.push_str(word);
                }
            }
        }
        if !current.is_empty() {
            out.push(format!("{indent}{current}"));
        }
    }

    out
}

fn stylize_help_line(line: String, theme: &Theme) -> Line<'static> {
    if line.trim().is_empty() {
        return Line::from(Span::raw(""));
    }

    // Section heading (no indentation)
    if !line.starts_with(' ') {
        return Line::from(Span::styled(line, heading_style(theme)));
    }

    let key = key_style(theme);
    let indent_len = line.chars().take_while(|c| *c == ' ').count();
    let indent = " ".repeat(indent_len);
    let trimmed = line[indent_len..].to_string();

    // Split "keys" column from description column by the first run of >= 2 spaces.
    let mut split_at: Option<usize> = None;
    let mut run = 0usize;
    for (i, ch) in trimmed.char_indices() {
        if ch == ' ' {
            run += 1;
            if run >= 2 {
                split_at = Some(i + 1 - run);
                break;
            }
        } else {
            run = 0;
        }
    }

    if let Some(at) = split_at {
        let left = trimmed[..at].trim_end().to_string();
        let right = trimmed[at..].trim().to_string();
        let left_style = if left.trim_start().starts_with('(') {
            Style::default().fg(theme.text_muted)
        } else {
            key
        };

        let mut spans = vec![Span::raw(indent), Span::styled(left, left_style)];
        if !right.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::raw(right));
        }
        Line::from(spans)
    } else {
        Line::from(vec![Span::raw(indent), Span::raw(trimmed)])
    }
}
