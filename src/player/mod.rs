use std::{
    cmp::min,
    fs,
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use rodio::Source;

use crate::{
    audio,
    library::Track,
    meta::{self, TrackMeta},
    audio::{AudioControl, AudioOutput, VolumeControl},
    util::{make_shuffled_order, SaturatingDurationSince},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayState {
    Stopped,
    Playing,
    Paused,
}

pub(crate) struct Player {
    audio: AudioOutput,
    audio_ctl: AudioControl,

    pub(crate) tracks: Vec<Track>,
    pub(crate) current: usize,
    pub(crate) selected: usize,

    // Playback order (either sequential or shuffled). Values are indices into `tracks`.
    play_order: Vec<usize>,
    play_pos: usize,
    pub(crate) shuffle: bool,

    pub(crate) state: PlayState,
    pub(crate) volume: VolumeControl,

    base_pos: Duration,
    started_at: Option<Instant>,
    paused_at: Option<Instant>,
    total_pause: Duration,
    pub(crate) total_duration: Option<Duration>,

    pub(crate) now_meta: TrackMeta,

    pub(crate) loop_current: bool,

    /// The directory from which the library was loaded. Used for YouTube downloads.
    pub(crate) library_path: PathBuf,
}

impl Player {
    pub(crate) fn new(
        tracks: Vec<Track>,
        start_index: usize,
        audio: AudioOutput,
        library_path: PathBuf,
    ) -> Result<Self> {
        let audio_ctl = audio.control();

        let start_index = min(start_index, tracks.len().saturating_sub(1));
        let play_order: Vec<usize> = (0..tracks.len()).collect();
        Ok(Self {
            audio,
            audio_ctl,
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
            library_path,
        })
    }

    fn sync_play_pos(&mut self) {
        if let Some(pos) = self.play_order.iter().position(|&i| i == self.current) {
            self.play_pos = pos;
        }
    }

    pub(crate) fn toggle_shuffle(&mut self) {
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

    pub(crate) fn has_tracks(&self) -> bool {
        !self.tracks.is_empty()
    }

    pub(crate) fn current_track(&self) -> Option<&Track> {
        self.tracks.get(self.current)
    }

    pub(crate) fn refresh_volume(&mut self) {
        self.volume.refresh();
        self.audio_ctl.set_gain(self.volume.app_gain_scalar());
    }

    pub(crate) fn adjust_volume(&mut self, delta: f32) {
        self.volume.adjust(delta);
        self.audio_ctl.set_gain(self.volume.app_gain_scalar());
    }

    pub(crate) fn play_selected(&mut self) -> Result<()> {
        if !self.has_tracks() {
            return Ok(());
        }
        self.current = self.selected;
        self.sync_play_pos();
        self.start_track(Duration::ZERO)
    }

    pub(crate) fn restart_current(&mut self) -> Result<()> {
        self.start_track(Duration::ZERO)
    }

    pub(crate) fn toggle_loop_selected(&mut self) -> Result<()> {
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

    pub(crate) fn start_track(&mut self, start_pos: Duration) -> Result<()> {
        let track = self
            .current_track()
            .context("No track selected")?
            .path
            .clone();

        // Prepare everything first. If decoding/seeking fails, keep the current sink playing.
        let meta = meta::probe_track_meta(&track).unwrap_or_default();
        let (source, total_duration) = open_source(&track, start_pos, self.loop_current)
            .with_context(|| format!("Failed to open track: {}", track.display()))?;

        // Ensure app gain is applied in the callback.
        self.audio_ctl.set_gain(self.volume.app_gain_scalar());
        self.audio_ctl.set_paused(false);
        self.audio_ctl
            .set_source(source, self.audio.channels, self.audio.sample_rate);

        self.now_meta = meta.clone();
        self.total_duration = total_duration.or(meta.duration);
        self.base_pos = start_pos;
        self.started_at = Some(Instant::now());
        self.paused_at = None;
        self.total_pause = Duration::ZERO;
        self.state = PlayState::Playing;
        Ok(())
    }

    pub(crate) fn toggle_pause(&mut self) {
        match self.state {
            PlayState::Playing => {
                self.audio_ctl.set_paused(true);
                self.state = PlayState::Paused;
                self.paused_at = Some(Instant::now());
            }
            PlayState::Paused => {
                self.audio_ctl.set_paused(false);
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

    pub(crate) fn stop_playback(&mut self) {
        self.audio_ctl.stop_now();
        self.audio_ctl.set_paused(false);

        self.state = PlayState::Stopped;
        self.started_at = None;
        self.paused_at = None;
        self.total_pause = Duration::ZERO;
    }

    pub(crate) fn is_track_finished(&self) -> bool {
        self.state == PlayState::Playing && self.audio_ctl.take_finished()
    }

    pub(crate) fn next_track(&mut self) -> Result<()> {
        if !self.has_tracks() {
            return Ok(());
        }
        self.play_pos = (self.play_pos + 1) % self.play_order.len();
        self.current = self.play_order[self.play_pos];
        self.selected = self.current;
        self.start_track(Duration::ZERO)
    }

    pub(crate) fn prev_track(&mut self) -> Result<()> {
        if !self.has_tracks() {
            return Ok(());
        }
        self.play_pos = (self.play_pos + self.play_order.len() - 1) % self.play_order.len();
        self.current = self.play_order[self.play_pos];
        self.selected = self.current;
        self.start_track(Duration::ZERO)
    }

    pub(crate) fn seek_relative(&mut self, delta: i64) -> Result<()> {
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

    pub(crate) fn position(&self) -> Duration {
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

    pub(crate) fn select_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub(crate) fn select_down(&mut self) {
        if self.selected + 1 < self.tracks.len() {
            self.selected += 1;
        }
    }

    /// Re-discover tracks from the library directory and merge new ones in.
    pub(crate) fn refresh_tracks(&mut self) {
        use crate::library::discover_tracks;

        let Ok(fresh) = discover_tracks(&self.library_path) else {
            return;
        };

        // Collect existing paths for O(n) lookup.
        let existing: std::collections::HashSet<PathBuf> =
            self.tracks.iter().map(|t| t.path.clone()).collect();

        let mut added = false;
        for track in fresh {
            if !existing.contains(&track.path) {
                self.tracks.push(track);
                added = true;
            }
        }

        if added {
            // Re-sort the full list.
            self.tracks.sort_by(|a, b| a.path.cmp(&b.path));

            // Recompute shuffle order.
            if self.shuffle {
                self.play_order = make_shuffled_order(self.tracks.len(), self.current);
                self.play_pos = 0;
            } else {
                self.play_order = (0..self.tracks.len()).collect();
                self.sync_play_pos();
            }
        }
    }

    pub(crate) fn delete_selected(&mut self) -> Result<()> {
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
            self.audio_ctl.stop_now();
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

        // Recompute current index and selection bounds.
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

impl Drop for Player {
    fn drop(&mut self) {
        // Make best-effort to stop audio immediately on any exit path.
        // (E.g. terminal closed -> SIGHUP, or event I/O error.)
        self.stop_playback();
    }
}

fn open_source(
    path: &Path,
    start_pos: Duration,
    loop_enabled: bool,
) -> Result<(Box<dyn Source<Item = f32> + Send>, Option<Duration>)> {
    let (source, total) = audio::open_source(path, start_pos, loop_enabled)?;
    let total = total.or_else(|| meta::probe_duration(path).ok());
    Ok((source, total))
}
