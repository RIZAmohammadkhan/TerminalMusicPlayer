use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
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

        // Best-effort: clear the source if we can grab the lock quickly.
        if let Ok(mut state) = self.state.try_lock() {
            state.source = None;
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
            state.source = Some(Box::new(src));
        }
    }

    pub fn take_finished(&self) -> bool {
        self.finished.swap(false, Ordering::Relaxed)
    }
}

struct State {
    // Already converted to output channels/sample-rate.
    source: Option<Box<dyn Source<Item = f32> + Send>>,
}

pub struct AudioOutput {
    // Must be held for the lifetime of the output; dropping it stops playback.
    _stream: cpal::Stream,
    control: AudioControl,
    pub sample_rate: u32,
    pub channels: u16,
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

        let state = Arc::new(Mutex::new(State { source: None }));
        let paused = Arc::new(AtomicBool::new(false));
        let gain_bits = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let finished = Arc::new(AtomicBool::new(false));
        let killed = Arc::new(AtomicBool::new(false));

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

        // Try smaller buffers first for responsiveness.
        let mut last_err: Option<anyhow::Error> = None;
        for frames in [256u32, 512, 1024, 2048] {
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
                    return Ok(Self {
                        _stream: stream,
                        control,
                        sample_rate,
                        channels,
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
            state,
            paused,
            gain_bits,
            finished,
            killed,
            err_cb,
        )
        .or_else(|e| Err(last_err.unwrap_or(e)))?;

        stream.play().map_err(|e| anyhow!(e))?;

        Ok(Self {
            _stream: stream,
            control,
            sample_rate,
            channels,
        })
    }

    pub fn control(&self) -> AudioControl {
        self.control.clone()
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
    finished: &AtomicBool,
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

    let Ok(mut st) = state.lock() else {
        out.fill(0.0);
        return;
    };

    let Some(mut src) = st.source.take() else {
        out.fill(0.0);
        return;
    };

    let mut ended = false;
    for sample in out.iter_mut() {
        if ended {
            *sample = 0.0;
            continue;
        }

        match src.next() {
            Some(v) => {
                let scaled = (v * gain).clamp(-1.0, 1.0);
                *sample = scaled;
            }
            None => {
                ended = true;
                finished.store(true, Ordering::Relaxed);
                *sample = 0.0;
            }
        }
    }

    if !ended {
        st.source = Some(src);
    }
}

fn write_data_i16(
    out: &mut [i16],
    state: &Mutex<State>,
    paused: &AtomicBool,
    gain_bits: &AtomicU32,
    finished: &AtomicBool,
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

    let Ok(mut st) = state.lock() else {
        out.fill(0);
        return;
    };

    let Some(mut src) = st.source.take() else {
        out.fill(0);
        return;
    };

    let mut ended = false;
    for sample in out.iter_mut() {
        if ended {
            *sample = 0;
            continue;
        }

        match src.next() {
            Some(v) => {
                let scaled = (v * gain).clamp(-1.0, 1.0);
                *sample = (scaled * i16::MAX as f32) as i16;
            }
            None => {
                ended = true;
                finished.store(true, Ordering::Relaxed);
                *sample = 0;
            }
        }
    }

    if !ended {
        st.source = Some(src);
    }
}

fn write_data_u16(
    out: &mut [u16],
    state: &Mutex<State>,
    paused: &AtomicBool,
    gain_bits: &AtomicU32,
    finished: &AtomicBool,
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

    let Ok(mut st) = state.lock() else {
        out.fill(u16::MAX / 2);
        return;
    };

    let Some(mut src) = st.source.take() else {
        out.fill(u16::MAX / 2);
        return;
    };

    let mid = u16::MAX as f32 / 2.0;
    let mut ended = false;
    for sample in out.iter_mut() {
        if ended {
            *sample = u16::MAX / 2;
            continue;
        }

        match src.next() {
            Some(v) => {
                let scaled = (v * gain).clamp(-1.0, 1.0);
                let centered = (scaled * mid) + mid;
                *sample = centered.clamp(0.0, u16::MAX as f32) as u16;
            }
            None => {
                ended = true;
                finished.store(true, Ordering::Relaxed);
                *sample = u16::MAX / 2;
            }
        }
    }

    if !ended {
        st.source = Some(src);
    }
}
