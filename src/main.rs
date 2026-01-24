#[cfg(not(target_os = "linux"))]
compile_error!("terminal-music-player is Linux-only. Build on Linux (target_os=\"linux\").");

use std::{
    cmp::min,
    env,
    fs,
    fs::File,
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    terminal,
};
use ratatui::{
    prelude::*,
    text::Text,
    widgets::block::Title,
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

use signal_hook::{consts::signal::*, flag as signal_flag};

mod volume;
use volume::VolumeControl;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Directory (or file) to play.
    ///
    /// If omitted: uses XDG music dir (if configured), else ~/Music, else the current directory.
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

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
    volume: VolumeControl,

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
            volume: VolumeControl::new(),
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

    fn refresh_volume(&mut self) {
        self.volume.refresh();
    }

    fn adjust_volume(&mut self, delta: f32) {
        self.volume.adjust(self.sink.as_ref(), delta);
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
        self.volume.apply_to_sink(&sink);

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

    fn stop_playback(&mut self) {
        if let Some(old) = self.sink.take() {
            old.stop();
        }
        self.state = PlayState::Stopped;
        self.started_at = None;
        self.paused_at = None;
        self.total_pause = Duration::ZERO;
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

    fn delete_selected(&mut self) -> Result<()> {
        if !self.has_tracks() {
            return Ok(());
        }

        let idx = self.selected;
        let path = self
            .tracks
            .get(idx)
            .context("No track selected")?
            .path
            .clone();

        let deleting_current = idx == self.current;
        let was_playing_or_paused = matches!(self.state, PlayState::Playing | PlayState::Paused);

        // Remove from disk first; if it fails, keep the entry.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("Failed to delete file: {}", path.display()));
            }
        }

        if deleting_current {
            if let Some(old) = self.sink.take() {
                old.stop();
            }
            self.state = PlayState::Stopped;
            self.loop_current = false;
            self.base_pos = Duration::ZERO;
            self.started_at = None;
            self.paused_at = None;
            self.total_pause = Duration::ZERO;
            self.total_duration = None;
            self.now_meta = TrackMeta::default();
        }

        self.tracks.remove(idx);

        if self.tracks.is_empty() {
            self.play_order.clear();
            self.play_pos = 0;
            self.current = 0;
            self.selected = 0;
            self.shuffle = false;
            return Ok(());
        }

        if !deleting_current {
            if idx < self.current {
                self.current = self.current.saturating_sub(1);
            }
        } else {
            self.current = idx.min(self.tracks.len().saturating_sub(1));
        }

        self.selected = self.selected.min(self.tracks.len().saturating_sub(1));

        if self.shuffle {
            self.play_order = make_shuffled_order(self.tracks.len(), self.current);
            self.play_pos = 0;
        } else {
            self.play_order = (0..self.tracks.len()).collect();
            self.play_pos = self.current;
        }

        if deleting_current && was_playing_or_paused {
            self.selected = self.current;
            self.start_track(Duration::ZERO)?;
        }

        Ok(())
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
    let total = decoder
        .total_duration()
        .or_else(|| probe_duration(path).ok());

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
    help_scroll: u16,
    search_mode: bool,
    search_query: String,
    delete_confirm: Option<DeleteConfirm>,
    last_tick: Instant,
}

#[derive(Debug, Clone)]
struct DeleteConfirm {
    index: usize,
    started_at: Instant,
}

fn main() -> Result<()> {
    TerminalCleanup::install_panic_hook();

    let args = Args::parse();

    let library_path = args.path.unwrap_or_else(default_library_path);

    // Handle SIGINT/SIGTERM/etc. so we can restore terminal state.
    let shutdown = Arc::new(AtomicBool::new(false));
    for sig in [SIGINT, SIGTERM, SIGHUP, SIGQUIT] {
        signal_flag::register(sig, Arc::clone(&shutdown))
            .with_context(|| format!("register signal handler for {sig}"))?;
    }

    let tracks = discover_tracks(&library_path)?;
    let mut player = Player::new(tracks, args.index)?;

    // Auto-start first track if any
    if player.has_tracks() {
        player.start_track(Duration::ZERO)?;
    }

    let mut terminal = init_terminal()?;
    let _cleanup = TerminalCleanup;
    let mut ui = UiState {
        volume_mode: false,
        show_help: false,
        help_scroll: 0,
        search_mode: false,
        search_query: String::new(),
        delete_confirm: None,
        last_tick: Instant::now(),
    };

    let tick_rate = Duration::from_millis(50);

    loop {
        if shutdown.load(Ordering::Relaxed) {
            player.stop_playback();
            break;
        }

        player.refresh_volume();
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
    }

    drop(terminal);
    Ok(())
}

fn handle_key(key: KeyEvent, player: &mut Player, ui: &mut UiState) -> Result<bool> {
    // Ctrl+C quit
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        player.stop_playback();
        return Ok(true);
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

        return Ok(false);
    }

    // Enter search mode.
    if key.code == KeyCode::Char('S') {
        ui.search_mode = true;
        ui.search_query.clear();
        ui.delete_confirm = None;
        return Ok(false);
    }

    // Quit
    if key.code == KeyCode::Char('q') {
        player.stop_playback();
        return Ok(true);
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
        return Ok(false);
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
        return Ok(false);
    }

    // Volume mode: toggle with `v`, adjust with Up/Down until toggled off.
    // Note: terminals don't provide a "v is held" modifier, so this is the most reliable UX.
    if ui.volume_mode {
        match key.code {
            KeyCode::Up => {
                player.adjust_volume(0.05);
                return Ok(false);
            }
            KeyCode::Down => {
                player.adjust_volume(-0.05);
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
        KeyCode::Char('s') => {
            player.toggle_shuffle();
        }
        KeyCode::Char('D') => {
            if !player.has_tracks() {
                return Ok(false);
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

fn restore_terminal_minimal() {
    let _ = terminal::disable_raw_mode();
    let mut stdout = io::stdout();
    let _ = crossterm::execute!(stdout, terminal::LeaveAlternateScreen, cursor::Show);
    let _ = stdout.flush();
}

struct TerminalCleanup;

impl TerminalCleanup {
    fn install_panic_hook() {
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

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

fn xdg_music_dir() -> Option<PathBuf> {
    let home = home_dir()?;
    let config = home.join(".config/user-dirs.dirs");
    let content = fs::read_to_string(config).ok()?;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some(rest) = line.strip_prefix("XDG_MUSIC_DIR=") else {
            continue;
        };

        let raw = rest.trim();
        let raw = raw.trim_matches('"');
        let expanded = raw.replace("$HOME", &home.to_string_lossy());
        let p = PathBuf::from(expanded);
        if p.is_dir() {
            return Some(p);
        }
    }

    None
}

fn default_music_dir() -> Option<PathBuf> {
    if let Some(p) = xdg_music_dir() {
        return Some(p);
    }

    let home = home_dir()?;
    for name in ["Music", "music"] {
        let p = home.join(name);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

fn default_library_path() -> PathBuf {
    default_music_dir().unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
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
        .style(
            Style::default()
                .fg(Color::Cyan)
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
                ("▶ ", Style::default().fg(Color::Green))
            } else {
                ("  ", Style::default())
            };

            let name_style = if i == player.current {
                Style::default()
                    .fg(Color::Cyan)
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
                .border_style(Style::default().fg(Color::Yellow))
                .title(Title::from(Line::styled(
                    "Library",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("» ");

    f.render_stateful_widget(list, left[0], &mut state);

    let search_text = if ui.search_mode {
        if ui.search_query.is_empty() {
            "Type to search…".to_string()
        } else {
            ui.search_query.clone()
        }
    } else {
        "Press S to search".to_string()
    };

    let search_style = if ui.search_mode {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let search_widget = Paragraph::new(search_text)
        .style(search_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Magenta))
                .title(Title::from(Line::styled(
                    "Search",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ))),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(search_widget, left[1]);

    // Now playing
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(mid[1]);

    let now_playing = now_playing_lines(player, ui);
    let now_widget = Paragraph::new(Text::from(now_playing))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Title::from(Line::styled(
                    "Now",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))),
        );
    f.render_widget(now_widget, right[0]);

    let (ratio, label) = progress(player);
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Green))
                .title(Title::from(Line::styled(
                    "Progress",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ))),
        )
        .gauge_style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(ratio)
        .label(label);
    f.render_widget(gauge, right[1]);

    let hints = hints_lines(player, ui);
    let help_widget = Paragraph::new(Text::from(hints))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Blue))
                .title(Title::from(Line::styled(
                    "Hints",
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                ))),
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

    let vol = format!("{:.0}%", player.volume.display() * 100.0);
    let chord = if ui.volume_mode {
        " (v: volume mode)"
    } else {
        ""
    };

    let lp = if player.loop_current { " • Loop" } else { "" };
    let sh = if player.shuffle { " • Shuffle" } else { "" };
    let backend = player.volume.label();
    format!("State: {state} • Volume: {vol} [{backend}]{chord}{lp}{sh}")
}

fn now_playing_lines(player: &Player, _ui: &UiState) -> Vec<Line<'static>> {
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

    let key = key_style();
    let title_style = Style::default()
        .fg(Color::Yellow)
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
                if player.has_tracks() {
                    player.current + 1
                } else {
                    0
                },
                player.tracks.len()
            )),
        ]),
    ]
}

fn key_style() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD)
}

fn heading_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn hints_lines(player: &Player, ui: &UiState) -> Vec<Line<'static>> {
    let key = key_style();

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
                Span::styled(name.to_string(), Style::default().fg(Color::Yellow)),
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
        Span::styled("v", key),
        Span::raw(" volume mode • "),
        Span::styled("S", key),
        Span::raw(" search • "),
        Span::styled("D", key),
        Span::raw(" delete"),
    ])]
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
            } else if let (Some(sample_rate), Some(n_frames)) =
                (params.sample_rate, params.n_frames)
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
        "  s           toggle shuffle order",
        "  S           search library (type to select)",
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

fn draw_help_overlay(f: &mut Frame, player: &Player, ui: &UiState) {
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
        .title(Title::from(base_header))
        .style(Style::default().fg(Color::White));

    if !indicator.is_empty() {
        block = block.title(
            Title::from(Line::styled(
                indicator,
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(Alignment::Right),
        );
    }

    let styled_lines: Vec<Line<'static>> = lines.into_iter().map(stylize_help_line).collect();
    let p = Paragraph::new(Text::from(styled_lines))
        .block(block)
        .scroll((scroll, 0))
        .style(Style::default().fg(Color::White));

    f.render_widget(p, overlay);
}

fn help_overlay_rect(area: Rect) -> Rect {
    // Use almost all terminal space, with a small margin, so the cheatsheet is
    // readable even on small terminals.
    area.inner(Margin {
        vertical: 1,
        horizontal: 2,
    })
}

fn help_wrapped_lines(ui: &UiState, width: u16) -> Vec<String> {
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

fn stylize_help_line(line: String) -> Line<'static> {
    if line.trim().is_empty() {
        return Line::from(Span::raw(""));
    }

    // Section heading (no indentation)
    if !line.starts_with(' ') {
        return Line::from(Span::styled(line, heading_style()));
    }

    let key = key_style();
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
            Style::default().fg(Color::DarkGray)
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
