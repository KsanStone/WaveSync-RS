use crate::sound::audio_backend::{AudioBackend, OptionCaptureCallback};
use crate::sound::audio_system::AudioSystem;
use crate::sound::capture_source::CaptureSource;
use crate::sound::{AudioBackendType, SampleFormat};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, Stream, StreamConfig};
use log::error;
use std::any::Any;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct CpalAudioBackend {
    capture_callback: OptionCaptureCallback,
    audio_system: AudioSystem,
    capture_source: CaptureSource,
    sequence_index: Arc<AtomicUsize>,
    host: Host,
    capture_stop_signal: Option<Sender<()>>,
    /// true if it's a mic/real input, false if it's a loopback device
    input_devices: Vec<(Device, bool)>,
}

macro_rules! impl_stream_methods {
    ($fn_prefix:ident, $sample_type:ty, $convert:expr) => {
        fn $fn_prefix(
            device: &Device,
            config: &StreamConfig,
            callback: Arc<Mutex<Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>>>,
            channels: usize,
        ) -> Result<Stream, Box<dyn std::error::Error>> {
            let stream = device.build_input_stream(
                config,
                move |data: &[$sample_type], _: &cpal::InputCallbackInfo| {
                    let mut channel_data = vec![vec![0.0; data.len() / channels]; channels];
                    for (i, &sample) in data.iter().enumerate() {
                        let channel = i % channels;
                        let frame_idx = i / channels;
                        if frame_idx < channel_data[channel].len() {
                            channel_data[channel][frame_idx] = $convert(sample);
                        }
                    }
                    callback.lock().unwrap()(channel_data);
                },
                |err| error!("Stream error: {}", err),
                None,
            )?;
            Ok(stream)
        }
    };
}

impl CpalAudioBackend {
    pub fn new() -> Self {
        Self::new_with_host(cpal::default_host())
    }

    pub fn new_with_host(host: Host) -> Self {
        let mut input_devices: Vec<(Device, bool)> = Vec::new();

        // Enumerate input devices
        if let Ok(devices) = host.input_devices() {
            for device in devices {
                input_devices.push((device, true)); // real input
            }
        }

        // Add default output as potential loopback
        if let Some(default_output) = host.default_output_device() {
            input_devices.push((default_output.clone(), false));
        }

        // Enumerate other output devices for additional loopback options
        if let Ok(outputs) = host.output_devices() {
            for device in outputs {
                if let Some((_, is_default)) = host
                    .default_output_device()
                    .as_ref()
                    .and_then(|d| Some((d.name().ok()?, true)))
                    && device.name().ok() == Some(is_default.to_string())
                {
                    continue; // skip default output already added
                }
                input_devices.push((device, false)); // loopback/output
            }
        }

        CpalAudioBackend {
            capture_callback: None,
            audio_system: AudioSystem {
                name: host.id().name().to_string(),
                backend: AudioBackendType::Cpal,
            },
            capture_source: CaptureSource {
                name: "Default Input Device".to_string(),
                id: "default_input".to_string(),
                is_loopback: false,
                channels: 2,
                sample_rate: 48000,
                format: SampleFormat::F32,
                backend: AudioBackendType::Cpal,
            },
            sequence_index: Default::default(),
            host,
            capture_stop_signal: None,
            input_devices,
        }
    }
}

impl AudioBackend for CpalAudioBackend {
    fn detect_supported_capture_sources(&self) -> Vec<CaptureSource> {
        self.input_devices
            .iter()
            .enumerate()
            .filter_map(|(index, device)| {
                let is_loopback = !device.1;
                let config = if is_loopback {
                    device.0.default_output_config().ok()
                } else {
                    device.0.default_input_config().ok()
                }?;
                let name_prefix = if is_loopback { "🔄" } else { "🎤" };
                Some(CaptureSource {
                    name: format!("{} {}", name_prefix, device.0.name().unwrap_or_default()),
                    is_loopback,
                    id: if is_loopback {
                        format!("loopback_device_{}", index)
                    } else {
                        format!("input_device_{}", index)
                    },
                    channels: config.channels() as u32,
                    sample_rate: config.sample_rate().0,
                    format: SampleFormat::F32,
                    backend: AudioBackendType::Cpal,
                })
            })
            .collect()
    }

    fn detect_supported_audio_systems(&self) -> Vec<AudioSystem> {
        vec![AudioSystem {
            name: self.host.id().name().to_string(),
            backend: AudioBackendType::Cpal,
        }]
    }

    fn find_default_capture_source(&self) -> CaptureSource {
        // Find first loopback device if any
        if let Some(source) = self
            .input_devices
            .iter()
            .enumerate()
            .filter(|(i, d)| self.is_loopback_device(*i))
            .filter_map(|(i, device)| {
                device
                    .0
                    .default_output_config()
                    .ok()
                    .map(|cfg| CaptureSource {
                        name: format!("🔄 {} (Loopback)", device.0.name().unwrap_or_default()),
                        id: format!("loopback_device_{}", i),
                        channels: cfg.channels() as u32,
                        sample_rate: cfg.sample_rate().0,
                        format: SampleFormat::F32,
                        backend: AudioBackendType::Cpal,
                        is_loopback: true,
                    })
            })
            .next()
        {
            return source;
        }

        // Fallback to first input device
        if let Some((i, device)) = self.input_devices.iter().enumerate().next()
            && let Ok(cfg) = device.0.default_input_config()
        {
            return CaptureSource {
                name: device.0.name().unwrap_or_default(),
                id: format!("input_device_{}", i),
                channels: cfg.channels() as u32,
                sample_rate: cfg.sample_rate().0,
                format: SampleFormat::F32,
                backend: AudioBackendType::Cpal,
                is_loopback: false,
            };
        }

        // Final fallback
        CaptureSource {
            name: "No Audio Devices".to_string(),
            id: "none".to_string(),
            channels: 0,
            sample_rate: 0,
            format: SampleFormat::F32,
            backend: AudioBackendType::Cpal,
            is_loopback: false,
        }
    }

    fn set_current_audio_system(&mut self, _system: AudioSystem) {}

    fn get_current_audio_system(&self) -> AudioSystem {
        self.audio_system.clone()
    }

    fn set_frame_callback(&mut self, callback: Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>) {
        self.capture_callback = Some(Arc::new(Mutex::new(callback)));
    }

    fn start_capture(&mut self, source: CaptureSource) {
        if let Some(stream) = self.capture_stop_signal.take() {
            drop(stream);
        }

        let is_loopback = source.id.starts_with("loopback_device_");
        let device_index = source
            .id
            .strip_prefix("input_device_")
            .or_else(|| source.id.strip_prefix("loopback_device_"))
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or_else(|| {
                error!("Unknown device type: {}", source.id);
                0
            });

        let device = if let Some(d) = self.input_devices.get(device_index) {
            d.0.clone()
        } else {
            error!("Device not found: {}", source.id);
            return;
        };

        let callback = self.capture_callback.as_ref().unwrap().clone();

        // Determine which function to call based on sample format
        let make_stream = move |device: &Device,
                                config: &StreamConfig,
                                channels: usize|
              -> Result<Stream, Box<dyn std::error::Error>> {
            use cpal::SampleFormat::*;
            let format = if is_loopback {
                // For Windows shared loopback, use the shared functions
                match device.default_output_config() {
                    Ok(cfg) => cfg.sample_format(),
                    Err(_) => F32,
                }
            } else {
                match device.default_input_config() {
                    Ok(cfg) => cfg.sample_format(),
                    Err(_) => F32,
                }
            };

            match format {
                F32 => Self::create_stream_f32(&device, config, callback.clone(), channels),
                I16 => Self::create_stream_i16(&device, config, callback.clone(), channels),
                U16 => Self::create_stream_u16(&device, config, callback.clone(), channels),
                I32 => Self::create_stream_i32(&device, config, callback.clone(), channels),
                U8 => Self::create_stream_u8(&device, config, callback.clone(), channels),

                _ => {
                    error!("Unsupported sample format: {:?}", format);
                    Err("Unsupported sample format".into())
                }
            }
        };

        // Build stream config
        let config = if is_loopback {
            match device.default_output_config() {
                Ok(cfg) => StreamConfig {
                    channels: cfg.channels(),
                    sample_rate: cfg.sample_rate(),
                    buffer_size: cpal::BufferSize::Default,
                },
                Err(e) => {
                    error!("Error getting output config: {}", e);
                    return;
                }
            }
        } else {
            match device.default_input_config() {
                Ok(cfg) => cfg.into(),
                Err(e) => {
                    error!("Error getting input config: {}", e);
                    return;
                }
            }
        };

        let channels = config.channels as usize;

        let (tx, rx) = std::sync::mpsc::channel();
        self.capture_stop_signal = Some(tx);

        let device = device.clone();

        thread::spawn(move || match make_stream(&device, &config, channels) {
            Ok(stream) => {
                if let Err(e) = stream.play() {
                    error!(
                        "Error starting capture for device '{}': {}",
                        device.name().unwrap_or_else(|_| "Unknown".to_string()),
                        e
                    );
                }

                let _ = rx.recv();
                drop(stream);
            }
            Err(_) => {
                error!(
                    "Failed to create stream for device '{}'",
                    device.name().unwrap_or_else(|_| "Unknown".to_string())
                );
            }
        });
    }

    fn stop_capture(&mut self) {
        if let Some(sender) = self.capture_stop_signal.take() {
            sender.send(()).expect("Failed to send the stop signal");
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CpalAudioBackend {
    impl_stream_methods!(create_stream_f32, f32, |s| s);
    impl_stream_methods!(create_stream_i16, i16, |s| s as f32 / i16::MAX as f32);
    impl_stream_methods!(create_stream_u16, u16, |s| (s as f32 / u16::MAX as f32)
        * 2.0
        - 1.0);
    impl_stream_methods!(create_stream_i32, i32, |s| s as f32 / i32::MAX as f32);
    impl_stream_methods!(create_stream_u8, u8, |s| (s as f32 / u8::MAX as f32) * 2.0
        - 1.0);

    /// Returns true if the device is a loopback device
    fn is_loopback_device(&self, index: usize) -> bool {
        !self
            .input_devices
            .get(index)
            .map(|(_, is_real_input)| *is_real_input)
            .unwrap_or(true)
    }

    pub fn new_with_fallback() -> Self {
        let default_host = cpal::default_host();
        let backend = Self::new_with_host(default_host);

        if backend.input_devices.is_empty() {
            #[cfg(target_os = "windows")]
            let host_id = cpal::HostId::Wasapi;
            #[cfg(target_os = "macos")]
            let host_id = cpal::HostId::CoreAudio;
            #[cfg(target_os = "linux")]
            let host_id = cpal::HostId::Alsa;

            if let Ok(host) = cpal::host_from_id(host_id) {
                let alt_backend = Self::new_with_host(host);
                if !alt_backend.input_devices.is_empty() {
                    return alt_backend;
                }
            }
        }

        backend
    }
}
