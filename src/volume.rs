use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

/// Cross-platform volume controller.
///
/// - Prefer a native *system mixer* backend when available for the target OS.
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
    #[cfg(target_os = "linux")]
    LinuxAlsa(linux::AlsaSystemVolume),

    #[cfg(windows)]
    WindowsEndpoint(windows_backend::WindowsSystemVolume),

    #[cfg(target_os = "macos")]
    MacCoreAudio(macos_backend::CoreAudioSystemVolume),
}

impl SystemBackend {
    fn label(&self) -> &'static str {
        match self {
            #[cfg(target_os = "linux")]
            SystemBackend::LinuxAlsa(_) => "System (ALSA)",
            #[cfg(windows)]
            SystemBackend::WindowsEndpoint(_) => "System (Windows)",
            #[cfg(target_os = "macos")]
            SystemBackend::MacCoreAudio(_) => "System (CoreAudio)",
        }
    }

    fn get(&mut self) -> Result<f32> {
        match self {
            #[cfg(target_os = "linux")]
            SystemBackend::LinuxAlsa(v) => v.get(),
            #[cfg(windows)]
            SystemBackend::WindowsEndpoint(v) => v.get(),
            #[cfg(target_os = "macos")]
            SystemBackend::MacCoreAudio(v) => v.get(),
        }
    }

    fn set(&mut self, value: f32) -> Result<()> {
        match self {
            #[cfg(target_os = "linux")]
            SystemBackend::LinuxAlsa(v) => v.set(value),
            #[cfg(windows)]
            SystemBackend::WindowsEndpoint(v) => v.set(value),
            #[cfg(target_os = "macos")]
            SystemBackend::MacCoreAudio(v) => v.set(value),
        }
    }
}

fn try_system_backend() -> Option<Backend> {
    #[cfg(target_os = "linux")]
    {
        return linux::AlsaSystemVolume::new()
            .ok()
            .map(|b| Backend::System(SystemBackend::LinuxAlsa(b)));
    }

    #[cfg(windows)]
    {
        return windows_backend::WindowsSystemVolume::new()
            .ok()
            .map(|b| Backend::System(SystemBackend::WindowsEndpoint(b)));
    }

    #[cfg(target_os = "macos")]
    {
        return macos_backend::CoreAudioSystemVolume::new()
            .ok()
            .map(|b| Backend::System(SystemBackend::MacCoreAudio(b)));
    }

    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        None
    }
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

            Err(anyhow!("No ALSA mixer element with playback volume found"))
        }

        fn chan_for_get(selem: &Selem) -> SelemChannelId {
            // Prefer a stable channel; fall back progressively.
            let preferred = [
                SelemChannelId::FrontLeft,
                SelemChannelId::FrontRight,
                SelemChannelId::mono(),
            ];

            for ch in preferred {
                if selem.has_playback_channel(ch) {
                    return ch;
                }
            }

            SelemChannelId::FrontLeft
        }

        pub fn get(&mut self) -> Result<f32> {
            let Some(selem) = self.mixer.find_selem(&self.selem_id) else {
                return Err(anyhow!("ALSA mixer element disappeared"));
            };

            let ch = Self::chan_for_get(&selem);
            let raw = selem
                .get_playback_volume(ch)
                .context("get ALSA playback volume")?;

            let denom = (self.max - self.min) as f32;
            if denom <= 0.0 {
                return Err(anyhow!("invalid ALSA playback volume range"));
            }

            let norm = ((raw - self.min) as f32 / denom).clamp(0.0, 1.0);
            Ok(norm)
        }

        pub fn set(&mut self, value: f32) -> Result<()> {
            let v = value.clamp(0.0, 1.0);
            let raw = self.min + ((v * (self.max - self.min) as f32).round() as i64);

            // Prefer setting all channels if available.
            let Some(selem) = self.mixer.find_selem(&self.selem_id) else {
                return Err(anyhow!("ALSA mixer element disappeared"));
            };

            selem
                .set_playback_volume_all(raw)
                .context("set ALSA playback volume")?;

            Ok(())
        }
    }
}

#[cfg(windows)]
mod windows_backend {
    use super::*;

    use windows::{
        core::Interface,
        Win32::{
            Media::Audio::{
                eConsole, eRender, IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator,
            },
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
            },
        },
    };

    pub struct WindowsSystemVolume {
        endpoint: IAudioEndpointVolume,
    }

    impl WindowsSystemVolume {
        pub fn new() -> Result<Self> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED)
                    .context("CoInitializeEx for system volume")?;

                // If any later step fails, ensure we uninitialize.
                let res = (|| {
                    let enumerator: IMMDeviceEnumerator =
                        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                            .context("Create IMMDeviceEnumerator")?;

                    let device = enumerator
                        .GetDefaultAudioEndpoint(eRender, eConsole)
                        .context("GetDefaultAudioEndpoint")?;

                    let endpoint: IAudioEndpointVolume = device
                        .Activate(CLSCTX_ALL, None)
                        .context("Activate IAudioEndpointVolume")?;

                    Ok::<_, anyhow::Error>(Self { endpoint })
                })();

                if res.is_err() {
                    CoUninitialize();
                }

                res
            }
        }

        pub fn get(&mut self) -> Result<f32> {
            unsafe {
                let mut v: f32 = 0.0;
                self.endpoint
                    .GetMasterVolumeLevelScalar(&mut v)
                    .context("GetMasterVolumeLevelScalar")?;
                Ok(v.clamp(0.0, 1.0))
            }
        }

        pub fn set(&mut self, value: f32) -> Result<()> {
            unsafe {
                let v = value.clamp(0.0, 1.0);
                self.endpoint
                    .SetMasterVolumeLevelScalar(v, std::ptr::null())
                    .context("SetMasterVolumeLevelScalar")?;
                Ok(())
            }
        }
    }

    impl Drop for WindowsSystemVolume {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod macos_backend {
    use super::*;

    use coreaudio_sys::{
        kAudioDevicePropertyScopeOutput, kAudioHardwarePropertyDefaultOutputDevice,
        kAudioObjectPropertyElementMaster, kAudioObjectSystemObject, kAudioPropertyElementMaster,
        kAudioPropertyScopeOutput, AudioObjectGetPropertyData, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectSetPropertyData, AudioValueRange, OSStatus,
    };

    pub struct CoreAudioSystemVolume {
        device: AudioObjectID,
    }

    impl CoreAudioSystemVolume {
        pub fn new() -> Result<Self> {
            unsafe {
                let mut device: AudioObjectID = 0;
                let mut size = std::mem::size_of::<AudioObjectID>() as u32;

                let addr = AudioObjectPropertyAddress {
                    mSelector: kAudioHardwarePropertyDefaultOutputDevice,
                    mScope: kAudioObjectPropertyElementMaster,
                    mElement: kAudioObjectPropertyElementMaster,
                };

                let status: OSStatus = AudioObjectGetPropertyData(
                    kAudioObjectSystemObject,
                    &addr,
                    0,
                    std::ptr::null(),
                    &mut size,
                    &mut device as *mut _ as *mut _,
                );

                if status != 0 || device == 0 {
                    return Err(anyhow!("CoreAudio: failed to get default output device"));
                }

                Ok(Self { device })
            }
        }

        pub fn get(&mut self) -> Result<f32> {
            unsafe {
                let mut volume: f32 = 0.0;
                let mut size = std::mem::size_of::<f32>() as u32;

                let addr = AudioObjectPropertyAddress {
                    mSelector: coreaudio_sys::kAudioDevicePropertyVolumeScalar,
                    mScope: kAudioPropertyScopeOutput,
                    mElement: kAudioPropertyElementMaster,
                };

                let status = AudioObjectGetPropertyData(
                    self.device,
                    &addr,
                    0,
                    std::ptr::null(),
                    &mut size,
                    &mut volume as *mut _ as *mut _,
                );

                if status != 0 {
                    return Err(anyhow!("CoreAudio: get volume failed"));
                }

                Ok(volume.clamp(0.0, 1.0))
            }
        }

        pub fn set(&mut self, value: f32) -> Result<()> {
            unsafe {
                let mut volume = value.clamp(0.0, 1.0);
                let size = std::mem::size_of::<f32>() as u32;

                let addr = AudioObjectPropertyAddress {
                    mSelector: coreaudio_sys::kAudioDevicePropertyVolumeScalar,
                    mScope: kAudioPropertyScopeOutput,
                    mElement: kAudioPropertyElementMaster,
                };

                let status = AudioObjectSetPropertyData(
                    self.device,
                    &addr,
                    0,
                    std::ptr::null(),
                    size,
                    &mut volume as *mut _ as *mut _,
                );

                if status != 0 {
                    return Err(anyhow!("CoreAudio: set volume failed"));
                }

                Ok(())
            }
        }
    }
}
