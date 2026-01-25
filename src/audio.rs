use std::{
    collections::VecDeque,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use rodio::Source;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder, DecoderOptions},
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
    units::Time,
};

pub fn open_source(
    path: &Path,
    start_pos: Duration,
    loop_enabled: bool,
) -> Result<(Box<dyn Source<Item = f32> + Send>, Option<Duration>)> {
    // Prefer our own Symphonia source, because it allows us to disable strict
    // verification and to recover from decode errors.
    match SymphoniaSource::try_new(path.to_path_buf(), start_pos, loop_enabled) {
        Ok(src) => {
            let total = src.total_duration();
            return Ok((Box::new(src), total));
        }
        Err(primary) => {
            // Fallback to rodio's built-in decoder.
            // This can still succeed for formats Symphonia doesn't handle well
            // in our streaming wrapper.
            let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
            let reader = BufReader::new(file);
            let decoder = rodio::Decoder::new(reader)
                .with_context(|| format!("rodio decode failed: {primary:#}"))?;
            let total = decoder.total_duration();
            let source = decoder.skip_duration(start_pos).convert_samples();
            let source: Box<dyn Source<Item = f32> + Send> = if loop_enabled {
                Box::new(source.repeat_infinite())
            } else {
                Box::new(source)
            };
            Ok((source, total))
        }
    }
}

struct SymphoniaSource {
    path: PathBuf,
    loop_enabled: bool,

    // Decoder state.
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,

    // Audio format.
    sample_rate: u32,
    channels: u16,

    // Playback.
    total: Option<Duration>,
    fifo: VecDeque<f32>,
    skip_samples: u64,

    // Error recovery.
    consecutive_decode_errors: u32,
}

impl SymphoniaSource {
    fn try_new(path: PathBuf, start_pos: Duration, loop_enabled: bool) -> Result<Self> {
        let (format, track_id) = open_format(&path)?;

        let track = format
            .tracks()
            .iter()
            .find(|t| t.id == track_id)
            .cloned()
            .ok_or_else(|| anyhow!("no default track"))?;

        let total = best_effort_duration(&track.codec_params);

        let decoder = symphonia::default::get_codecs()
            .make(
                &track.codec_params,
                &DecoderOptions {
                    // This is the key to being more tolerant of "almost valid" MP3s.
                    // Many players rely on ffmpeg-style error recovery; we emulate that
                    // by disabling strict verification and skipping bad frames.
                    verify: false,
                    ..Default::default()
                },
            )
            .context("create decoder")?;

        let mut source = Self {
            path,
            loop_enabled,
            format,
            decoder,
            track_id,
            sample_rate: track.codec_params.sample_rate.unwrap_or(48_000),
            channels: track
                .codec_params
                .channels
                .map(|c| c.count() as u16)
                .unwrap_or(2),
            total,
            fifo: VecDeque::with_capacity(48_000),
            skip_samples: 0,
            consecutive_decode_errors: 0,
        };

        // Derive a skip budget once we know the best-effort format.
        let start_frames = (start_pos.as_secs_f64() * source.sample_rate as f64)
            .max(0.0)
            .round() as u64;
        source.skip_samples = start_frames.saturating_mul(source.channels as u64);

        // Prime the decoder so we can fail early instead of hanging on a corrupt stream.
        source.prime()?;
        Ok(source)
    }

    fn prime(&mut self) -> Result<()> {
        // Try to decode some audio. If we cannot get any samples after a reasonable
        // number of packets / errors, consider the file unsupported.
        let mut packets_seen = 0u32;
        while self.fifo.is_empty() && packets_seen < 250 {
            packets_seen += 1;
            if self.decode_more().is_ok() {
                // keep looping until we have samples
            }
        }

        if self.fifo.is_empty() {
            return Err(anyhow!(
                "no decodable audio frames (stream may be badly corrupted or unsupported)"
            ));
        }
        Ok(())
    }

    fn reopen_for_loop(&mut self) -> Result<()> {
        let (format, track_id) = open_format(&self.path)?;
        self.format = format;
        self.track_id = track_id;

        let track = self
            .format
            .tracks()
            .iter()
            .find(|t| t.id == track_id)
            .ok_or_else(|| anyhow!("no default track"))?;

        self.decoder = symphonia::default::get_codecs().make(
            &track.codec_params,
            &DecoderOptions {
                verify: false,
                ..Default::default()
            },
        )?;

        self.fifo.clear();
        self.skip_samples = 0;
        self.consecutive_decode_errors = 0;
        Ok(())
    }

    fn decode_more(&mut self) -> Result<()> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(_)) => {
                    // Treat IO errors as EOF for local files.
                    return Err(anyhow!("eof"));
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(anyhow!(e)),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(audio) => {
                    self.consecutive_decode_errors = 0;

                    // Convert to interleaved f32 *immediately* so we don't keep
                    // a borrow from the decoder alive.
                    let spec = *audio.spec();
                    let mut sample_buf =
                        SampleBuffer::<f32>::new(audio.frames() as u64, spec);
                    sample_buf.copy_interleaved_ref(audio);

                    // Track observed format (best-effort). If it changes mid-stream,
                    // keep the original. Rodio expects these to stay stable.
                    if self.sample_rate == 0 {
                        self.sample_rate = spec.rate;
                    }
                    if self.channels == 0 {
                        self.channels = spec.channels.count() as u16;
                    }

                    self.fifo.extend(sample_buf.samples());
                    return Ok(());
                }
                Err(SymphoniaError::DecodeError(_)) => {
                    // Bad frame: skip and continue.
                    self.consecutive_decode_errors = self.consecutive_decode_errors.saturating_add(1);
                    if self.consecutive_decode_errors > 1_000 {
                        return Err(anyhow!("too many consecutive decode errors"));
                    }
                    continue;
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(anyhow!(e)),
            }
        }
    }
}

impl Iterator for SymphoniaSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Ensure we have data.
            if self.fifo.is_empty() {
                match self.decode_more() {
                    Ok(()) => {}
                    Err(_) => {
                        if self.loop_enabled {
                            if self.reopen_for_loop().is_ok() {
                                continue;
                            }
                        }
                        return None;
                    }
                }
            }

            // Apply initial skip.
            while self.skip_samples > 0 {
                if self.fifo.pop_front().is_some() {
                    self.skip_samples -= 1;
                } else {
                    break;
                }
            }

            if self.skip_samples > 0 {
                continue;
            }

            if let Some(s) = self.fifo.pop_front() {
                return Some(s);
            }
        }
    }
}

impl Source for SymphoniaSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels.max(1)
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate.max(1)
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total
    }
}

fn open_format(path: &Path) -> Result<(Box<dyn FormatReader>, u32)> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let track_id = probed
        .format
        .default_track()
        .or_else(|| probed.format.tracks().first())
        .ok_or_else(|| anyhow!("no tracks in container"))?
        .id;

    Ok((probed.format, track_id))
}

fn best_effort_duration(params: &symphonia::core::codecs::CodecParameters) -> Option<Duration> {
    if let (Some(time_base), Some(n_frames)) = (params.time_base, params.n_frames) {
        let Time { seconds, frac, .. } = time_base.calc_time(n_frames);
        return Some(Duration::from_secs(seconds) + Duration::from_secs_f64(frac));
    }

    if let (Some(sample_rate), Some(n_frames)) = (params.sample_rate, params.n_frames) {
        let secs = n_frames as f64 / sample_rate as f64;
        if secs.is_finite() && secs > 0.0 {
            return Some(Duration::from_secs_f64(secs));
        }
    }

    None
}
