use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

/// Linux volume controller.
///
/// - Prefer the native ALSA *system mixer* backend when available.
/// - Fall back to per-app gain using `rodio::Sink::set_volume`.
pub struct VolumeControl {
    backend: Backend,
    app_gain: f32, // 0.0..=1.5

    display: f32,
    display_label: &'static str,
    last_refresh: Instant,
}

impl VolumeControl {
    pub fn new() -> Self {
        let app_gain = 1.0;

        let mut backend = try_system_backend().unwrap_or(Backend::AppGain);
        let (display, display_label) = match &mut backend {
            Backend::System(sys) => {
                let v = sys.get().unwrap_or(1.0);
                (v, sys.label())
            }
            Backend::AppGain => (app_gain, "App gain"),
        };

        Self {
            backend,
            app_gain,
            display,
            display_label,
            last_refresh: Instant::now(),
        }
    }

    pub fn is_system(&self) -> bool {
        matches!(self.backend, Backend::System(_))
    }

    pub fn label(&self) -> &'static str {
        self.display_label
    }

    /// The currently shown volume in UI.
    ///
    /// - For system backends this is $0..=1$.
    /// - For app gain this is $0..=1.5$.
    pub fn display(&self) -> f32 {
        self.display
    }

    pub fn app_gain(&self) -> f32 {
        self.app_gain
    }

    pub fn refresh(&mut self) {
        // Avoid hammering the system backend every frame.
        let min_period = Duration::from_millis(150);
        if self.last_refresh.elapsed() < min_period {
            return;
        }
        self.last_refresh = Instant::now();

        match &mut self.backend {
            Backend::System(sys) => match sys.get() {
                Ok(v) => {
                    self.display = v;
                    self.display_label = sys.label();
                }
                Err(_) => {
                    self.fallback_to_app_gain();
                }
            },
            Backend::AppGain => {
                self.display = self.app_gain;
                self.display_label = "App gain";
            }
        }
    }

    pub fn apply_to_sink(&self, sink: &rodio::Sink) {
        match &self.backend {
            Backend::System(_) => sink.set_volume(1.0),
            Backend::AppGain => sink.set_volume(self.app_gain),
        }
    }

    pub fn adjust(&mut self, sink: Option<&rodio::Sink>, delta: f32) {
        match &mut self.backend {
            Backend::System(sys) => {
                // System volume is normalized 0..=1.
                let current = sys.get().unwrap_or(self.display);
                let next = (current + delta).clamp(0.0, 1.0);
                if let Err(_) = sys.set(next) {
                    self.fallback_to_app_gain();
                    self.adjust(sink, delta);
                    return;
                }
                self.display = next;
                self.display_label = sys.label();

                // Keep app gain at unity when we're controlling system volume.
                self.app_gain = 1.0;
                if let Some(s) = sink {
                    s.set_volume(1.0);
                }
            }
            Backend::AppGain => {
                self.app_gain = (self.app_gain + delta).clamp(0.0, 1.5);
                self.display = self.app_gain;
                self.display_label = "App gain";
                if let Some(s) = sink {
                    s.set_volume(self.app_gain);
                }
            }
        }
    }

    fn fallback_to_app_gain(&mut self) {
        self.backend = Backend::AppGain;
        self.display = self.app_gain;
        self.display_label = "App gain";
    }
}

enum Backend {
    System(SystemBackend),
    AppGain,
}

enum SystemBackend {
    LinuxAlsa(linux::AlsaSystemVolume),
}

impl SystemBackend {
    fn label(&self) -> &'static str {
        match self {
            SystemBackend::LinuxAlsa(_) => "System (ALSA)",
        }
    }

    fn get(&mut self) -> Result<f32> {
        match self {
            SystemBackend::LinuxAlsa(v) => v.get(),
        }
    }

    fn set(&mut self, value: f32) -> Result<()> {
        match self {
            SystemBackend::LinuxAlsa(v) => v.set(value),
        }
    }
}

fn try_system_backend() -> Option<Backend> {
    linux::AlsaSystemVolume::new()
        .ok()
        .map(|b| Backend::System(SystemBackend::LinuxAlsa(b)))
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    use alsa::mixer::{Mixer, Selem, SelemChannelId, SelemId};

    pub struct AlsaSystemVolume {
        mixer: Mixer,
        selem_id: SelemId,
        min: i64,
        max: i64,
    }

    impl AlsaSystemVolume {
        pub fn new() -> Result<Self> {
            // "default" works for most ALSA setups. On PipeWire/PulseAudio systems
            // it often maps to an ALSA compatibility device.
            let mixer = Mixer::new("default", false).context("open ALSA mixer")?;

            let candidates = ["Master", "PCM", "Speaker", "Headphone", "Front", "Line Out"];
            for name in candidates {
                let id = SelemId::new(name, 0);
                if let Some(selem) = mixer.find_selem(&id) {
                    if selem.has_playback_volume() {
                        let (min, max) = selem.get_playback_volume_range();
                        if max > min {
                            return Ok(Self {
                                mixer,
                                selem_id: id,
                                min,
                                max,
                            });
                        }
                    }
                }
            }

            // Fallback: pick the first element that looks usable.
            for elem in mixer.iter() {
                let Some(selem) = Selem::new(elem) else {
                    continue;
                };
                if selem.has_playback_volume() {
                    let (min, max) = selem.get_playback_volume_range();
                    if max > min {
                        let selem_id = selem.get_id();
                        return Ok(Self {
                            mixer,
                            selem_id,
                            min,
                            max,
                        });
                    }
                }
            }

            Err(anyhow!("No usable ALSA mixer control found"))
        }

        pub fn get(&mut self) -> Result<f32> {
            let selem = self
                .mixer
                .find_selem(&self.selem_id)
                .context("find ALSA mixer element")?;

            let raw = selem
                .get_playback_volume(SelemChannelId::FrontLeft)
                .or_else(|_| selem.get_playback_volume(SelemChannelId::FrontRight))
                .or_else(|_| selem.get_playback_volume(SelemChannelId::mono()))
                .context("read ALSA playback volume")?;
            let range = (self.max - self.min).max(1) as f32;
            let normalized = ((raw - self.min) as f32 / range).clamp(0.0, 1.0);
            Ok(normalized)
        }

        pub fn set(&mut self, value: f32) -> Result<()> {
            let value = value.clamp(0.0, 1.0);
            let selem = self
                .mixer
                .find_selem(&self.selem_id)
                .context("find ALSA mixer element")?;

            let range = (self.max - self.min).max(1) as f32;
            let raw = self.min + (value * range).round() as i64;

            selem.set_playback_volume_all(raw)
                .context("set ALSA playback volume")?;
            Ok(())
        }
    }
}
