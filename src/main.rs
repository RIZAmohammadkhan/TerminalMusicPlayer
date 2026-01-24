use std::{
    cmp::min,
    fs::File,
    io::{self, BufReader},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    terminal,
};
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use symphonia::core::{
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey},
    probe::Hint,
    units::Time,
};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Directory (or file) to play. If omitted, uses current directory.
    #[arg(value_name = "PATH", default_value = ".")]
    path: PathBuf,

    /// Start at this track index (0-based)
    #[arg(long, default_value_t = 0)]
    index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug)]
struct Track {
    path: PathBuf,
    display_name: String,
}

#[derive(Clone, Debug, Default)]
struct TrackMeta {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration: Option<Duration>,
}

struct Player {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Option<Sink>,

    tracks: Vec<Track>,
    current: usize,
    selected: usize,

    // Playback order (either sequential or shuffled). Values are indices into `tracks`.
    play_order: Vec<usize>,
    play_pos: usize,
    shuffle: bool,

    state: PlayState,
    volume: f32, // 0.0..=1.5

    base_pos: Duration,
    started_at: Option<Instant>,
    paused_at: Option<Instant>,
    total_pause: Duration,
    total_duration: Option<Duration>,

    now_meta: TrackMeta,

    loop_current: bool,
}

impl Player {
    fn new(tracks: Vec<Track>, start_index: usize) -> Result<Self> {
        let (stream, handle) =
            OutputStream::try_default().context("Failed to open audio output")?;

        let start_index = min(start_index, tracks.len().saturating_sub(1));
        let play_order: Vec<usize> = (0..tracks.len()).collect();
        Ok(Self {
            _stream: stream,
            handle,
            sink: None,
            tracks,
            current: start_index,
            selected: start_index,

            play_order,
            play_pos: start_index,
            shuffle: false,
            state: PlayState::Stopped,
            volume: 1.0,
            base_pos: Duration::ZERO,
            started_at: None,
            paused_at: None,
            total_pause: Duration::ZERO,
            total_duration: None,

            now_meta: TrackMeta::default(),

            loop_current: false,
        })
    }

    fn sync_play_pos(&mut self) {
        if let Some(pos) = self.play_order.iter().position(|&i| i == self.current) {
            self.play_pos = pos;
        }
    }

    fn toggle_shuffle(&mut self) {
        if !self.has_tracks() {
            self.shuffle = false;
            self.play_order.clear();
            self.play_pos = 0;
            return;
        }

        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.play_order = make_shuffled_order(self.tracks.len(), self.current);
            self.play_pos = 0;
        } else {
            self.play_order = (0..self.tracks.len()).collect();
            self.play_pos = self.current;
        }
    }

    fn has_tracks(&self) -> bool {
        !self.tracks.is_empty()
    }

    fn current_track(&self) -> Option<&Track> {
        self.tracks.get(self.current)
    }

    fn selected_track(&self) -> Option<&Track> {
        self.tracks.get(self.selected)
    }

    fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.5);
        if let Some(sink) = &self.sink {
            sink.set_volume(self.volume);
        }
    }

    fn play_selected(&mut self) -> Result<()> {
        if !self.has_tracks() {
            return Ok(());
        }
        self.current = self.selected;
        self.sync_play_pos();
        self.start_track(Duration::ZERO)
    }

    fn restart_current(&mut self) -> Result<()> {
        self.start_track(Duration::ZERO)
    }

    fn toggle_loop_selected(&mut self) -> Result<()> {
        if !self.has_tracks() {
            return Ok(());
        }

        // If the user selected a different track, start looping that track.
        if self.selected != self.current {
            self.current = self.selected;
            self.loop_current = true;
            return self.start_track(Duration::ZERO);
        }

        // Toggle loop for the current track. Restart playback to apply the source mode.
        let pos = self.position();
        self.loop_current = !self.loop_current;
        self.start_track(pos)
    }

    fn start_track(&mut self, start_pos: Duration) -> Result<()> {
        let track = self
            .current_track()
            .context("No track selected")?
            .path
            .clone();

        if let Some(old) = self.sink.take() {
            old.stop();
        }

        let sink = Sink::try_new(&self.handle).context("Failed to create audio sink")?;
        sink.set_volume(self.volume);

        let meta = probe_track_meta(&track).unwrap_or_default();
        let (source, total_duration) = open_source(&track, start_pos, self.loop_current)
            .with_context(|| format!("Failed to open track: {}", track.display()))?;

        self.now_meta = meta.clone();
        self.total_duration = total_duration.or(meta.duration);
        self.base_pos = start_pos;
        self.started_at = Some(Instant::now());
        self.paused_at = None;
        self.total_pause = Duration::ZERO;

        sink.append(source);
        sink.play();

        self.sink = Some(sink);
        self.state = PlayState::Playing;
        Ok(())
    }

    fn toggle_pause(&mut self) {
        let Some(sink) = &self.sink else {
            return;
        };

        match self.state {
            PlayState::Playing => {
                sink.pause();
                self.state = PlayState::Paused;
                self.paused_at = Some(Instant::now());
            }
            PlayState::Paused => {
                sink.play();
                self.state = PlayState::Playing;
                if let Some(paused_at) = self.paused_at.take() {
                    self.total_pause += paused_at.elapsed();
                }
            }
            PlayState::Stopped => {
                // no-op
            }
        }
    }

    fn is_track_finished(&self) -> bool {
        match (&self.sink, self.state) {
            (Some(sink), PlayState::Playing) => sink.empty(),
            _ => false,
        }
    }

    fn next_track(&mut self) -> Result<()> {
        if !self.has_tracks() {
            return Ok(());
        }
        self.play_pos = (self.play_pos + 1) % self.play_order.len();
        self.current = self.play_order[self.play_pos];
        self.selected = self.current;
        self.start_track(Duration::ZERO)
    }

    fn prev_track(&mut self) -> Result<()> {
        if !self.has_tracks() {
            return Ok(());
        }
        self.play_pos = (self.play_pos + self.play_order.len() - 1) % self.play_order.len();
        self.current = self.play_order[self.play_pos];
        self.selected = self.current;
        self.start_track(Duration::ZERO)
    }

    fn seek_relative(&mut self, delta: i64) -> Result<()> {
        if !self.has_tracks() {
            return Ok(());
        }

        let current_pos = self.position();
        let mut target = if delta.is_negative() {
            current_pos.saturating_sub(Duration::from_millis(delta.unsigned_abs() as u64))
        } else {
            current_pos.saturating_add(Duration::from_millis(delta as u64))
        };

        if let Some(total) = self.total_duration {
            if target > total {
                target = total;
            }
        }

        self.start_track(target)
    }

    fn position(&self) -> Duration {
        let pos = match self.state {
            PlayState::Stopped => Duration::ZERO,
            PlayState::Paused => {
                if let (Some(started_at), Some(paused_at)) = (self.started_at, self.paused_at) {
                    self.base_pos
                        + paused_at
                            .saturating_duration_since(started_at)
                            .saturating_sub(self.total_pause)
                } else {
                    self.base_pos
                }
            }
            PlayState::Playing => {
                if let Some(started_at) = self.started_at {
                    self.base_pos + started_at.elapsed().saturating_sub(self.total_pause)
                } else {
                    self.base_pos
                }
            }
        };

        // When looping, keep the UI position within the track length.
        if self.loop_current {
            if let Some(total) = self.total_duration {
                if total > Duration::ZERO {
                    let ms = pos.as_millis() as u64;
                    let total_ms = total.as_millis() as u64;
                    return Duration::from_millis(ms % total_ms);
                }
            }
        }

        pos
    }

    fn select_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn select_down(&mut self) {
        if self.selected + 1 < self.tracks.len() {
            self.selected += 1;
        }
    }
}

fn open_source(
    path: &Path,
    start_pos: Duration,
    loop_enabled: bool,
) -> Result<(Box<dyn Source<Item = f32> + Send>, Option<Duration>)> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let decoder = rodio::Decoder::new(reader)?;
    let total = decoder.total_duration().or_else(|| probe_duration(path).ok());

    // Always call skip_duration so both branches have the same type.
    let source = decoder.skip_duration(start_pos).convert_samples();

    let source: Box<dyn Source<Item = f32> + Send> = if loop_enabled {
        Box::new(source.repeat_infinite())
    } else {
        Box::new(source)
    };

    Ok((source, total))
}

#[derive(Debug)]
struct UiState {
    volume_mode: bool,
    show_help: bool,
    last_tick: Instant,
    last_anim: Instant,
    spinner_i: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let tracks = discover_tracks(&args.path)?;
    let mut player = Player::new(tracks, args.index)?;

    // Auto-start first track if any
    if player.has_tracks() {
        player.start_track(Duration::ZERO)?;
    }

    let mut terminal = init_terminal()?;
    let mut ui = UiState {
        volume_mode: false,
        show_help: false,
        last_tick: Instant::now(),
        last_anim: Instant::now(),
        spinner_i: 0,
    };

    let tick_rate = Duration::from_millis(50);

    loop {
        terminal.draw(|f| draw_ui(f, &player, &ui))?;

        // Auto-advance
        if !player.loop_current && player.is_track_finished() {
            let _ = player.next_track();
        }

        let timeout = tick_rate
            .checked_sub(ui.last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if handle_key(key, &mut player, &mut ui)? {
                    break;
                }
            }
        }

        if ui.last_tick.elapsed() >= tick_rate {
            ui.last_tick = Instant::now();
        }

        // Slow animation: update separately from tick rate.
        if ui.last_anim.elapsed() >= Duration::from_millis(400) {
            ui.last_anim = Instant::now();
            ui.spinner_i = ui.spinner_i.wrapping_add(1);
        }
    }

    restore_terminal(terminal)?;
    Ok(())
}

fn handle_key(key: KeyEvent, player: &mut Player, ui: &mut UiState) -> Result<bool> {
    // Quit
    if key.code == KeyCode::Char('q') {
        return Ok(true);
    }

    // Ctrl+C quit
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(true);
    }

    // Help overlay toggle.
    if matches!(key.code, KeyCode::Char('h') | KeyCode::Char('?')) {
        ui.show_help = !ui.show_help;
        return Ok(false);
    }

    // While help is open, keep playback running but ignore most keys.
    if ui.show_help {
        if key.code == KeyCode::Esc {
            ui.show_help = false;
        }
        return Ok(false);
    }

    // Volume mode: toggle with `v`, adjust with Up/Down until toggled off.
    // Note: terminals don't provide a "v is held" modifier, so this is the most reliable UX.
    if ui.volume_mode {
        match key.code {
            KeyCode::Up => {
                player.set_volume(player.volume + 0.05);
                return Ok(false);
            }
            KeyCode::Down => {
                player.set_volume(player.volume - 0.05);
                return Ok(false);
            }
            KeyCode::Esc | KeyCode::Char('v') => {
                ui.volume_mode = false;
                return Ok(false);
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
            let _ = player.seek_relative(-10_000);
        }
        KeyCode::Char('n') => {
            // 10s forward
            let _ = player.seek_relative(10_000);
        }
        KeyCode::Char('P') => {
            let _ = player.prev_track();
        }
        KeyCode::Char('N') => {
            let _ = player.next_track();
        }
        KeyCode::Left => {
            let _ = player.seek_relative(-5_000);
        }
        KeyCode::Right => {
            let _ = player.seek_relative(5_000);
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
        KeyCode::Char('x') => {
            player.toggle_shuffle();
        }

        // Nice-to-have navigation
        KeyCode::Up => {
            if ui.volume_mode {
                player.set_volume(player.volume + 0.05);
            } else {
                player.select_up();
            }
        }
        KeyCode::Down => {
            if ui.volume_mode {
                player.set_volume(player.volume - 0.05);
            } else {
                player.select_down();
            }
        }
        KeyCode::Enter => {
            player.play_selected()?;
        }
        KeyCode::Char(' ') => {
            player.toggle_pause();
        }
        _ => {}
    }

    Ok(false)
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    terminal::enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, terminal::EnterAlternateScreen)
        .context("enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;
    terminal.clear().ok();
    Ok(terminal)
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    terminal::disable_raw_mode().ok();
    crossterm::execute!(terminal.backend_mut(), terminal::LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    Ok(())
}

fn discover_tracks(path: &Path) -> Result<Vec<Track>> {
    let mut files: Vec<PathBuf> = Vec::new();

    if path.is_file() {
        files.push(path.to_path_buf());
    } else {
        for entry in WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let p = entry.path();
            if is_audio_file(p) {
                files.push(p.to_path_buf());
            }
        }
    }

    files.sort();

    let tracks = files
        .into_iter()
        .map(|p| Track {
            display_name: p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string()),
            path: p,
        })
        .collect();

    Ok(tracks)
}

fn is_audio_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus"
    )
}

fn draw_ui(f: &mut Frame, player: &Player, ui: &UiState) {
    let area = f.area();

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let title = title_line(player, ui);
    let title_widget = Paragraph::new(title)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
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

    // Playlist
    let items: Vec<ListItem> = player
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let prefix = if i == player.current { "▶ " } else { "  " };
            let line = format!("{}{}", prefix, t.display_name);
            ListItem::new(line)
        })
        .collect();

    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(player.selected));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Library"),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("» ");

    f.render_stateful_widget(list, mid[0], &mut state);

    // Now playing
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(mid[1]);

    let now_playing = now_playing_text(player, ui);
    let now_widget = Paragraph::new(now_playing)
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Now"),
        );
    f.render_widget(now_widget, right[0]);

    let (ratio, label) = progress(player);
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Progress"),
        )
        .gauge_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        .ratio(ratio)
        .label(label);
    f.render_widget(gauge, right[1]);

    let hints = hints_text(ui);
    let help_widget = Paragraph::new(hints)
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::DarkGray))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Hints"),
        );
    f.render_widget(help_widget, right[2]);

    if ui.show_help {
        draw_help_overlay(f, player, ui);
    }
}

fn title_line(player: &Player, ui: &UiState) -> String {
    let state = match player.state {
        PlayState::Stopped => "stopped",
        PlayState::Playing => "playing",
        PlayState::Paused => "paused",
    };

    let vol = format!("{:.0}%", player.volume * 100.0);
    let chord = if ui.volume_mode {
        " (v: volume mode)"
    } else {
        ""
    };

    let lp = if player.loop_current { " • Loop" } else { "" };
    let sh = if player.shuffle { " • Shuffle" } else { "" };
    format!("State: {state} • Volume: {vol}{chord}{lp}{sh}")
}

fn now_playing_text(player: &Player, ui: &UiState) -> String {
    let name = player
        .current_track()
        .map(|t| t.display_name.as_str())
        .unwrap_or("(no tracks)");

    let spinner = spinner_text(player, ui);
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

    format!(
        "Track: {spinner} {title}\nArtist: {artist}\nAlbum:  {album}\nIndex:  {} / {}",
        if player.has_tracks() {
            player.current + 1
        } else {
            0
        },
        player.tracks.len()
    )
}

fn spinner_text(player: &Player, ui: &UiState) -> &'static str {
    // Requested animation: "." ".." ".." (repeat)
    const FRAMES: [&str; 3] = [".", "..", ".."];
    match player.state {
        PlayState::Playing => FRAMES[ui.spinner_i % FRAMES.len()],
        PlayState::Paused => "..",
        PlayState::Stopped => "",
    }
}

fn probe_duration(path: &Path) -> Result<Duration> {
    let meta = probe_track_meta(path)?;
    meta.duration.context("Duration unavailable")
}

fn probe_track_meta(path: &Path) -> Result<TrackMeta> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut meta = TrackMeta::default();

    // Gather tags from both the probe metadata and container metadata.
    if let Some(container_meta) = probed.metadata.get() {
        if let Some(rev) = container_meta.current() {
            apply_tags(&mut meta, rev);
        }
    }
    if let Some(rev) = probed.format.metadata().current() {
        apply_tags(&mut meta, rev);
    }

    // Duration (best-effort): use time_base*n_frames if present; else sample_rate*n_frames.
    if meta.duration.is_none() {
        if let Some(track) = probed
            .format
            .default_track()
            .or_else(|| probed.format.tracks().first())
        {
            let params = &track.codec_params;

            if let (Some(time_base), Some(n_frames)) = (params.time_base, params.n_frames) {
                let Time { seconds, frac, .. } = time_base.calc_time(n_frames);
                meta.duration = Some(Duration::from_secs(seconds) + Duration::from_secs_f64(frac));
            } else if let (Some(sample_rate), Some(n_frames)) = (params.sample_rate, params.n_frames)
            {
                let secs = n_frames as f64 / sample_rate as f64;
                if secs.is_finite() && secs > 0.0 {
                    meta.duration = Some(Duration::from_secs_f64(secs));
                }
            }
        }
    }

    Ok(meta)
}

fn apply_tags(meta: &mut TrackMeta, rev: &symphonia::core::meta::MetadataRevision) {
    for tag in rev.tags() {
        let value = tag.value.to_string();
        match tag.std_key {
            Some(StandardTagKey::TrackTitle) => {
                meta.title.get_or_insert(value);
            }
            Some(StandardTagKey::Artist) => {
                meta.artist.get_or_insert(value);
            }
            Some(StandardTagKey::Album) => {
                meta.album.get_or_insert(value);
            }
            _ => {
                // Fallbacks for common raw keys.
                match tag.key.to_ascii_lowercase().as_str() {
                    "title" => {
                        meta.title.get_or_insert(value);
                    }
                    "artist" => {
                        meta.artist.get_or_insert(value);
                    }
                    "album" => {
                        meta.album.get_or_insert(value);
                    }
                    _ => {
                        // ignore
                    }
                }
            }
        };
    }
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

fn hints_text(ui: &UiState) -> String {
    if ui.volume_mode {
        "Volume mode: ↑/↓ change volume • v/Esc exit".to_string()
    } else {
        "Press h for cheatsheet • v volume mode".to_string()
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
        "  q           quit",
        "  x           toggle shuffle order",
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

fn draw_help_overlay(f: &mut Frame, player: &Player, ui: &UiState) {
    let area = f.area();
    let overlay = centered_rect(78, 80, area);

    f.render_widget(Clear, overlay);

    let header = if player.loop_current {
        "Cheatsheet • Loop ON"
    } else {
        "Cheatsheet"
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(header)
        .style(Style::default().fg(Color::White));

    let text = help_text(ui);
    let p = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));

    f.render_widget(p, overlay);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn make_shuffled_order(len: usize, current: usize) -> Vec<usize> {
    if len == 0 {
        return Vec::new();
    }
    if len == 1 {
        return vec![0];
    }

    let mut rest: Vec<usize> = (0..len).filter(|&i| i != current).collect();

    // Fisher–Yates shuffle.
    for i in (1..rest.len()).rev() {
        let j = fastrand::usize(..=i);
        rest.swap(i, j);
    }

    let mut order = Vec::with_capacity(len);
    order.push(current);
    order.extend(rest);
    order
}

fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    let m = s / 60;
    let s = s % 60;
    format!("{m:02}:{s:02}")
}

trait SaturatingDurationSince {
    fn saturating_duration_since(self, earlier: Instant) -> Duration;
}

impl SaturatingDurationSince for Instant {
    fn saturating_duration_since(self, earlier: Instant) -> Duration {
        if self >= earlier {
            self.duration_since(earlier)
        } else {
            Duration::ZERO
        }
    }
}

trait SaturatingSub {
    fn saturating_sub(self, other: Duration) -> Duration;
}

impl SaturatingSub for Duration {
    fn saturating_sub(self, other: Duration) -> Duration {
        if self >= other {
            self - other
        } else {
            Duration::ZERO
        }
    }
}

trait SaturatingAdd {
    fn saturating_add(self, other: Duration) -> Duration;
}

impl SaturatingAdd for Duration {
    fn saturating_add(self, other: Duration) -> Duration {
        self.checked_add(other).unwrap_or(Duration::MAX)
    }
}
