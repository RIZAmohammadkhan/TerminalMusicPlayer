use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use rodio::{cpal, source::UniformSourceIterator, Source};

#[derive(Clone)]
pub struct AudioControl {
    state: Arc<Mutex<State>>,
    paused: Arc<AtomicBool>,
    gain_bits: Arc<AtomicU32>,
    finished: Arc<AtomicBool>,
    killed: Arc<AtomicBool>,
}

impl AudioControl {
    pub fn stop_now(&self) {
        // Make this lock-free / non-blocking so it can be called from a signal thread.
        // The audio callback checks `killed` before touching shared state.
        self.set_gain(0.0);
        self.killed.store(true, Ordering::Relaxed);

        // Best-effort: clear pending source + buffered audio if we can grab the lock quickly.
        if let Ok(mut state) = self.state.try_lock() {
            state.pending_source = None;
            state.buffer.clear();
            state.source_generation.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Best-effort immediate shutdown.
    ///
    /// Unlike `stop_now()`, this attempts to pause and drop the underlying CPAL stream
    /// so audio output stops even if the process is about to hang on terminal I/O.
    ///
    /// On some backends the stream handle is not `Send`, so we cannot drop it from a
    /// signal thread. Instead, this is a stronger semantic alias for `stop_now()`.
    pub fn shutdown_now(&self) {
        self.stop_now();
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn set_gain(&self, gain: f32) {
        let gain = if gain.is_finite() { gain.max(0.0) } else { 0.0 };
        self.gain_bits.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub fn set_source(
        &self,
        source: Box<dyn Source<Item = f32> + Send>,
        out_channels: u16,
        out_sample_rate: u32,
    ) {
        self.finished.store(false, Ordering::Relaxed);
        self.killed.store(false, Ordering::Relaxed);

        let src = UniformSourceIterator::new(source, out_channels, out_sample_rate);
        if let Ok(mut state) = self.state.lock() {
            state.pending_source = Some(Box::new(src));
            state.buffer.clear();
            state.source_generation.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn take_finished(&self) -> bool {
        self.finished.swap(false, Ordering::Relaxed)
    }
}

struct State {
    // Next source to play (already converted to output channels/sample-rate).
    pending_source: Option<Box<dyn Source<Item = f32> + Send>>,
    // Interleaved f32 samples ready for the audio callback.
    buffer: VecDeque<f32>,
    // Monotonic generation counter for source swaps.
    source_generation: AtomicU64,
}

pub struct AudioOutput {
    // Must be held for the lifetime of the output; dropping it stops playback.
    _stream: cpal::Stream,
    control: AudioControl,
    pub sample_rate: u32,
    pub channels: u16,

    worker_alive: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl AudioOutput {
    pub fn new_low_latency() -> Result<Self> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No default output device")?;

        let supported = device
            .default_output_config()
            .context("No default output config")?;

        let channels = supported.channels() as u16;
        let sample_rate = supported.sample_rate().0;

        let state = Arc::new(Mutex::new(State {
            pending_source: None,
            buffer: VecDeque::new(),
            source_generation: AtomicU64::new(0),
        }));
        let paused = Arc::new(AtomicBool::new(false));
        let gain_bits = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let finished = Arc::new(AtomicBool::new(false));
        let killed = Arc::new(AtomicBool::new(false));

        // Producer thread that decodes/resamples outside the audio callback.
        // Keep ~750ms of audio buffered to absorb transient stalls (terminal I/O, seeks, etc.).
        let buffer_capacity_samples: usize = {
            let secs = 0.75f32;
            let samples = (sample_rate as f32 * channels as f32 * secs).round() as usize;
            samples.clamp(16_384, 512_000)
        };

        let spawn_worker = |state: Arc<Mutex<State>>,
                            paused: Arc<AtomicBool>,
                            finished: Arc<AtomicBool>,
                            killed: Arc<AtomicBool>|
         -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
            let worker_alive = Arc::new(AtomicBool::new(true));
            let worker_alive_t = Arc::clone(&worker_alive);
            let worker = thread::spawn(move || {
                const CHUNK_SAMPLES: usize = 8192;
                let mut active: Option<Box<dyn Source<Item = f32> + Send>> = None;
                let mut active_gen: u64 = 0;

                while worker_alive_t.load(Ordering::Relaxed) {
                    if killed.load(Ordering::Relaxed) {
                        active = None;
                        if let Ok(mut st) = state.lock() {
                            st.pending_source = None;
                            st.buffer.clear();
                        }
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }

                    // Swap in a new source if requested.
                    let mut need: usize = 0;
                    let mut local_gen: u64 = active_gen;
                    let mut take_new: Option<Box<dyn Source<Item = f32> + Send>> = None;
                    if let Ok(mut st) = state.lock() {
                        let gen = st.source_generation.load(Ordering::Relaxed);
                        if gen != active_gen {
                            active_gen = gen;
                            local_gen = gen;
                            take_new = st.pending_source.take();
                            st.buffer.clear();
                        }

                        if st.buffer.len() < buffer_capacity_samples {
                            need = buffer_capacity_samples - st.buffer.len();
                        }
                    }

                    if let Some(src) = take_new {
                        active = Some(src);
                    }

                    if paused.load(Ordering::Relaxed) {
                        // No need to decode while paused; keep existing buffer.
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }

                    let Some(src) = active.as_mut() else {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    };

                    if need == 0 {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }

                    let to_pull = need.min(CHUNK_SAMPLES);
                    let mut chunk: Vec<f32> = Vec::with_capacity(to_pull);
                    for _ in 0..to_pull {
                        match src.next() {
                            Some(s) => chunk.push(s),
                            None => {
                                active = None;
                                finished.store(true, Ordering::Relaxed);
                                break;
                            }
                        }
                    }

                    if chunk.is_empty() {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }

                    // Push decoded samples into the shared buffer (but only if generation matches).
                    if let Ok(mut st) = state.lock() {
                        if st.source_generation.load(Ordering::Relaxed) == local_gen {
                            let spare = buffer_capacity_samples.saturating_sub(st.buffer.len());
                            let take = spare.min(chunk.len());
                            st.buffer.extend(chunk.into_iter().take(take));
                        }
                    }
                }
            });
            (worker_alive, worker)
        };

        let control = AudioControl {
            state: Arc::clone(&state),
            paused: Arc::clone(&paused),
            gain_bits: Arc::clone(&gain_bits),
            finished: Arc::clone(&finished),
            killed: Arc::clone(&killed),
        };

        let err_cb = |err| {
            eprintln!("an error occurred on output stream: {err}");
        };

        let mut base_config: cpal::StreamConfig = supported.clone().into();

        // Avoid ultra-small buffers; they are extremely prone to underruns on ALSA.
        let mut last_err: Option<anyhow::Error> = None;
        for frames in [1024u32, 2048, 4096] {
            base_config.buffer_size = cpal::BufferSize::Fixed(frames);
            let try_stream = build_stream(
                &device,
                &supported,
                base_config.clone(),
                Arc::clone(&state),
                Arc::clone(&paused),
                Arc::clone(&gain_bits),
                Arc::clone(&finished),
                Arc::clone(&killed),
                err_cb,
            );
            match try_stream {
                Ok(stream) => {
                    stream.play().map_err(|e| anyhow!(e))?;
                    let (worker_alive, worker) = spawn_worker(
                        Arc::clone(&state),
                        Arc::clone(&paused),
                        Arc::clone(&finished),
                        Arc::clone(&killed),
                    );
                    return Ok(Self {
                        _stream: stream,
                        control,
                        sample_rate,
                        channels,
                        worker_alive,
                        worker: Some(worker),
                    });
                }
                Err(e) => last_err = Some(e),
            }
        }

        // Fall back to default buffer size.
        base_config.buffer_size = cpal::BufferSize::Default;
        let stream = build_stream(
            &device,
            &supported,
            base_config,
            Arc::clone(&state),
            Arc::clone(&paused),
            Arc::clone(&gain_bits),
            Arc::clone(&finished),
            Arc::clone(&killed),
            err_cb,
        )
        .or_else(|e| Err(last_err.unwrap_or(e)))?;

        stream.play().map_err(|e| anyhow!(e))?;

        let (worker_alive, worker) = spawn_worker(state, paused, finished, killed);
        Ok(Self {
            _stream: stream,
            control,
            sample_rate,
            channels,
            worker_alive,
            worker: Some(worker),
        })
    }

    pub fn control(&self) -> AudioControl {
        self.control.clone()
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.control.stop_now();
        self.worker_alive.store(false, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn build_stream(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    config: cpal::StreamConfig,
    state: Arc<Mutex<State>>,
    paused: Arc<AtomicBool>,
    gain_bits: Arc<AtomicU32>,
    finished: Arc<AtomicBool>,
    killed: Arc<AtomicBool>,
    err_cb: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream> {
    use cpal::traits::DeviceTrait;

    let sample_format = supported.sample_format();

    match sample_format {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    write_data(data, &state, &paused, &gain_bits, &finished, &killed)
                },
                err_cb,
                None,
            )
            .map_err(|e| anyhow!(e)),
        cpal::SampleFormat::I16 => device
            .build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    write_data_i16(data, &state, &paused, &gain_bits, &finished, &killed)
                },
                err_cb,
                None,
            )
            .map_err(|e| anyhow!(e)),
        cpal::SampleFormat::U16 => device
            .build_output_stream(
                &config,
                move |data: &mut [u16], _| {
                    write_data_u16(data, &state, &paused, &gain_bits, &finished, &killed)
                },
                err_cb,
                None,
            )
            .map_err(|e| anyhow!(e)),
        other => Err(anyhow!(
            "Unsupported output sample format: {other:?}"
        )),
    }
}

fn current_gain(gain_bits: &AtomicU32) -> f32 {
    f32::from_bits(gain_bits.load(Ordering::Relaxed))
}

fn write_data(
    out: &mut [f32],
    state: &Mutex<State>,
    paused: &AtomicBool,
    gain_bits: &AtomicU32,
    _finished: &AtomicBool,
    killed: &AtomicBool,
) {
    if killed.load(Ordering::Relaxed) {
        out.fill(0.0);
        return;
    }

    if paused.load(Ordering::Relaxed) {
        out.fill(0.0);
        return;
    }

    let gain = current_gain(gain_bits);

    // Never block the audio callback; if we can't grab the lock immediately,
    // output silence for this period.
    let Ok(mut st) = state.try_lock() else {
        out.fill(0.0);
        return;
    };

    for sample in out.iter_mut() {
        if let Some(v) = st.buffer.pop_front() {
            *sample = (v * gain).clamp(-1.0, 1.0);
        } else {
            *sample = 0.0;
        }
    }
}

fn write_data_i16(
    out: &mut [i16],
    state: &Mutex<State>,
    paused: &AtomicBool,
    gain_bits: &AtomicU32,
    _finished: &AtomicBool,
    killed: &AtomicBool,
) {
    if killed.load(Ordering::Relaxed) {
        out.fill(0);
        return;
    }

    if paused.load(Ordering::Relaxed) {
        out.fill(0);
        return;
    }

    let gain = current_gain(gain_bits);

    let Ok(mut st) = state.try_lock() else {
        out.fill(0);
        return;
    };

    for sample in out.iter_mut() {
        if let Some(v) = st.buffer.pop_front() {
            let scaled = (v * gain).clamp(-1.0, 1.0);
            *sample = (scaled * i16::MAX as f32) as i16;
        } else {
            *sample = 0;
        }
    }
}

fn write_data_u16(
    out: &mut [u16],
    state: &Mutex<State>,
    paused: &AtomicBool,
    gain_bits: &AtomicU32,
    _finished: &AtomicBool,
    killed: &AtomicBool,
) {
    if killed.load(Ordering::Relaxed) {
        out.fill(u16::MAX / 2);
        return;
    }

    if paused.load(Ordering::Relaxed) {
        out.fill(u16::MAX / 2);
        return;
    }

    let gain = current_gain(gain_bits);
    let mid = u16::MAX as f32 / 2.0;

    let Ok(mut st) = state.try_lock() else {
        out.fill(u16::MAX / 2);
        return;
    };

    for sample in out.iter_mut() {
        if let Some(v) = st.buffer.pop_front() {
            let scaled = (v * gain).clamp(-1.0, 1.0);
            let centered = (scaled * mid) + mid;
            *sample = centered.clamp(0.0, u16::MAX as f32) as u16;
        } else {
            *sample = u16::MAX / 2;
        }
    }
}
