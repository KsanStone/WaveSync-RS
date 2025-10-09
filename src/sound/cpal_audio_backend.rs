use crate::sound::audio_backend::AudioBackend;
use crate::sound::audio_system::AudioSystem;
use crate::sound::capture_source::CaptureSource;
use crate::sound::{AudioBackendType, SampleFormat};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, SampleFormat as CpalSampleFormat, Stream, StreamConfig};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

pub struct CpalAudioBackend {
    capture_callback: Option<Arc<Mutex<Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>>>>,
    audio_system: AudioSystem,
    capture_source: CaptureSource,
    sequence_index: Arc<AtomicUsize>,
    host: Host,
    current_stream: Option<Stream>,
    input_devices: Vec<Device>,
}

impl CpalAudioBackend {
    fn sample_format_from_cpal(cpal_format: CpalSampleFormat) -> SampleFormat {
        match cpal_format {
            CpalSampleFormat::U8 => SampleFormat::U8,
            CpalSampleFormat::I16 => SampleFormat::I16,
            CpalSampleFormat::I32 => SampleFormat::I32,
            CpalSampleFormat::F32 => SampleFormat::F32,
            _ => SampleFormat::F32, // Default fallback
        }
    }
    pub fn new() -> Self {
        Self::new_with_host(cpal::default_host())
    }

    pub fn new_with_host(host: Host) -> Self {
        let mut input_devices: Vec<Device> = match host.input_devices() {
            Ok(devices) => {
                let devices_vec: Vec<Device> = devices.collect();
                if !devices_vec.is_empty() {
                    println!("🎤 Found {} input devices", devices_vec.len());
                }
                devices_vec
            }
            Err(e) => {
                eprintln!("❌ Warning: Could not enumerate input devices: {}", e);
                Vec::new()
            }
        };

        // Try to create loopback devices from the default output device
        if let Some(default_output) = host.default_output_device()
            && let Ok(_output_config) = default_output.default_output_config()
        {
            // Add the default output device as a potential loopback source
            input_devices.push(default_output.clone());
        }

        // Also try to enumerate all output devices for additional loopback options
        match host.output_devices() {
            Ok(output_devices) => {
                let output_devices_vec: Vec<Device> = output_devices.collect();

                for output_device in output_devices_vec.iter() {
                    if let Ok(_device_name) = output_device.name() {
                        // Skip if this is the default output device (already added)
                        if let Some(ref default_device) = host.default_output_device()
                            && let (Ok(n1), Ok(n2)) = (output_device.name(), default_device.name())
                            && n1 == n2
                        {
                            continue; // Skip, already added as default
                        }

                        input_devices.push(output_device.clone());
                    }
                }
            }
            Err(_e) => {
                // Silently ignore output device enumeration errors
            }
        }

        CpalAudioBackend {
            capture_callback: None,
            audio_system: AudioSystem {
                name: format!("CPAL Audio System ({})", host.id().name()),
                backend: AudioBackendType::Cpal,
            },
            capture_source: CaptureSource {
                name: "Default Input Device".to_string(),
                id: "default_input".to_string(),
                channels: 2,
                sample_rate: 48000,
                format: SampleFormat::F32,
                backend: AudioBackendType::Cpal,
            },
            sequence_index: Default::default(),
            host,
            current_stream: None,
            input_devices,
        }
    }
}

impl AudioBackend for CpalAudioBackend {
    fn detect_supported_capture_sources(&self) -> Vec<CaptureSource> {
        let mut sources = Vec::new();

        // Add all available input devices
        for (index, device) in self.input_devices.iter().enumerate() {
            let device_name = device
                .name()
                .unwrap_or_else(|_| "Unknown Device".to_string());

            // Check if this is a loopback device (output device used for input)
            let is_loopback = self.is_loopback_device(device, index);

            // Try to get input configurations
            match device.default_input_config() {
                Ok(config) => {
                    let display_name = if is_loopback {
                        format!("🔄 {} (Loopback)", device_name)
                    } else {
                        format!("🎤 {}", device_name)
                    };

                    sources.push(CaptureSource {
                        name: display_name,
                        id: format!("input_device_{}", index),
                        channels: config.channels() as u32,
                        sample_rate: config.sample_rate().0,
                        format: SampleFormat::F32, // Always report as F32 since we convert everything
                        backend: AudioBackendType::Cpal,
                    });
                }
                Err(_e) => {
                    // For loopback devices, try a different approach
                    if is_loopback {
                        // For loopback devices, we'll use the output config as a reference
                        if let Ok(output_config) = device.default_output_config() {
                            let display_name = format!("🔄 {} (Loopback)", device_name);
                            sources.push(CaptureSource {
                                name: display_name,
                                id: format!("loopback_device_{}", index),
                                channels: output_config.channels() as u32,
                                sample_rate: output_config.sample_rate().0,
                                format: SampleFormat::F32,
                                backend: AudioBackendType::Cpal,
                            });
                        }
                    }
                }
            }
        }

        // Also try to find the default device separately (in case it's not in our list)
        if let Some(default_device) = self.host.default_input_device() {
            let default_name = default_device
                .name()
                .unwrap_or_else(|_| "Default Device".to_string());

            // Check if this device is already in our list
            let mut already_found = false;
            for device in self.input_devices.iter() {
                if let (Ok(n1), Ok(n2)) = (device.name(), default_device.name())
                    && n1 == n2
                {
                    already_found = true;
                    break;
                }
            }

            if !already_found && let Ok(config) = default_device.default_input_config() {
                sources.push(CaptureSource {
                    name: format!("{} (Default)", default_name),
                    id: "default_input".to_string(),
                    channels: config.channels() as u32,
                    sample_rate: config.sample_rate().0,
                    format: SampleFormat::F32,
                    backend: AudioBackendType::Cpal,
                });
            }
        }

        if !sources.is_empty() {
            println!("📊 Found {} audio devices available", sources.len());
        }
        sources
    }

    fn detect_supported_audio_systems(&self) -> Vec<AudioSystem> {
        vec![AudioSystem {
            name: format!("CPAL Audio System ({})", self.host.id().name()),
            backend: AudioBackendType::Cpal,
        }]
    }

    fn find_default_capture_source(&self) -> CaptureSource {
        // Return the first available real device, or a placeholder if none available
        if let Some(first_device) = self.input_devices.first()
            && let Ok(config) = first_device.default_input_config()
        {
            return CaptureSource {
                name: first_device
                    .name()
                    .unwrap_or_else(|_| "Audio Device 1".to_string()),
                id: "input_device_0".to_string(),
                channels: config.channels() as u32,
                sample_rate: config.sample_rate().0,
                format: SampleFormat::F32,
                backend: AudioBackendType::Cpal,
            };
        }

        // Fallback if no devices available
        CaptureSource {
            name: "No Audio Devices".to_string(),
            id: "none".to_string(),
            channels: 0,
            sample_rate: 0,
            format: SampleFormat::F32,
            backend: AudioBackendType::Cpal,
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
        // Stop any existing stream
        if let Some(stream) = self.current_stream.take() {
            drop(stream);
        }

        // Get the device to use
        let device = if let Some(stripped) = source.id.strip_prefix("input_device_") {
            let device_index = stripped.parse::<usize>().unwrap_or(0);
            if let Some(device) = self.input_devices.get(device_index) {
                device.clone()
            } else {
                eprintln!("Input device not found: {}", source.id);
                return;
            }
        } else if let Some(stripped) = source.id.strip_prefix("loopback_device_") {
            let device_index = stripped.parse::<usize>().unwrap_or(0);
            if let Some(device) = self.input_devices.get(device_index) {
                device.clone()
            } else {
                eprintln!("Loopback device not found: {}", source.id);
                return;
            }
        } else {
            eprintln!("Unknown device type: {}", source.id);
            return;
        };

        // Get the input config
        let config = match device.default_input_config() {
            Ok(config) => config,
            Err(e) => {
                // For loopback devices, try a different approach
                if source.id.starts_with("loopback_device_") {
                    // Try to use the default input device as a proxy for system audio
                    if let Some(default_input) = self.host.default_input_device()
                        && let Ok(_fallback_config) = default_input.default_input_config()
                    {
                        return; // For now, we'll need more sophisticated implementation
                    }
                }

                eprintln!(
                    "Error getting input config for device '{}': {}",
                    device.name().unwrap_or_else(|_| "Unknown".to_string()),
                    e
                );
                return;
            }
        };

        let callback = self.capture_callback.as_ref().unwrap().clone();
        let channels = config.channels() as usize;

        // Create the stream - all formats will be converted to f32
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                self.create_stream_f32(&device, &config.into(), callback, channels)
            }
            cpal::SampleFormat::I16 => {
                self.create_stream_i16(&device, &config.into(), callback, channels)
            }
            cpal::SampleFormat::U16 => {
                self.create_stream_u16(&device, &config.into(), callback, channels)
            }
            cpal::SampleFormat::I32 => {
                self.create_stream_i32(&device, &config.into(), callback, channels)
            }
            cpal::SampleFormat::U8 => {
                self.create_stream_u8(&device, &config.into(), callback, channels)
            }
            _ => {
                eprintln!("Unsupported sample format: {:?}", config.sample_format());
                return;
            }
        };

        match stream {
            Ok(stream) => {
                self.current_stream = Some(stream);
                if let Err(e) = self.current_stream.as_ref().unwrap().play() {
                    eprintln!(
                        "Error starting capture for device '{}': {}",
                        device.name().unwrap_or_else(|_| "Unknown".to_string()),
                        e
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "Error creating stream for device '{}': {}",
                    device.name().unwrap_or_else(|_| "Unknown".to_string()),
                    e
                );
            }
        }
    }

    fn stop_capture(&mut self) {
        if let Some(stream) = self.current_stream.take() {
            drop(stream);
        }
    }
}

impl CpalAudioBackend {
    fn create_stream_f32(
        &self,
        device: &Device,
        config: &StreamConfig,
        callback: Arc<Mutex<Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>>>,
        channels: usize,
    ) -> Result<Stream, Box<dyn std::error::Error>> {
        let stream = device.build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mut channel_data = vec![vec![0.0; data.len() / channels]; channels];
                for (i, &sample) in data.iter().enumerate() {
                    let channel = i % channels;
                    let frame_idx = i / channels;
                    if frame_idx < channel_data[channel].len() {
                        channel_data[channel][frame_idx] = sample;
                    }
                }
                callback.lock().unwrap()(channel_data);
            },
            |err| eprintln!("Stream error: {}", err),
            None,
        )?;
        Ok(stream)
    }

    fn create_stream_i16(
        &self,
        device: &Device,
        config: &StreamConfig,
        callback: Arc<Mutex<Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>>>,
        channels: usize,
    ) -> Result<Stream, Box<dyn std::error::Error>> {
        let stream = device.build_input_stream(
            config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let max_value = i16::MAX as f32;
                let mut channel_data = vec![vec![0.0; data.len() / channels]; channels];
                for (i, &sample) in data.iter().enumerate() {
                    let channel = i % channels;
                    let frame_idx = i / channels;
                    if frame_idx < channel_data[channel].len() {
                        channel_data[channel][frame_idx] = sample as f32 / max_value;
                    }
                }
                callback.lock().unwrap()(channel_data);
            },
            |err| eprintln!("Stream error: {}", err),
            None,
        )?;
        Ok(stream)
    }

    fn create_stream_u16(
        &self,
        device: &Device,
        config: &StreamConfig,
        callback: Arc<Mutex<Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>>>,
        channels: usize,
    ) -> Result<Stream, Box<dyn std::error::Error>> {
        let stream = device.build_input_stream(
            config,
            move |data: &[u16], _: &cpal::InputCallbackInfo| {
                let max_value = u16::MAX as f32;
                let mut channel_data = vec![vec![0.0; data.len() / channels]; channels];
                for (i, &sample) in data.iter().enumerate() {
                    let channel = i % channels;
                    let frame_idx = i / channels;
                    if frame_idx < channel_data[channel].len() {
                        channel_data[channel][frame_idx] = (sample as f32 / max_value) * 2.0 - 1.0;
                    }
                }
                callback.lock().unwrap()(channel_data);
            },
            |err| eprintln!("Stream error: {}", err),
            None,
        )?;
        Ok(stream)
    }

    fn create_stream_i32(
        &self,
        device: &Device,
        config: &StreamConfig,
        callback: Arc<Mutex<Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>>>,
        channels: usize,
    ) -> Result<Stream, Box<dyn std::error::Error>> {
        let stream = device.build_input_stream(
            config,
            move |data: &[i32], _: &cpal::InputCallbackInfo| {
                let max_value = i32::MAX as f32;
                let mut channel_data = vec![vec![0.0; data.len() / channels]; channels];
                for (i, &sample) in data.iter().enumerate() {
                    let channel = i % channels;
                    let frame_idx = i / channels;
                    if frame_idx < channel_data[channel].len() {
                        channel_data[channel][frame_idx] = sample as f32 / max_value;
                    }
                }
                callback.lock().unwrap()(channel_data);
            },
            |err| eprintln!("Stream error: {}", err),
            None,
        )?;
        Ok(stream)
    }

    fn create_stream_u8(
        &self,
        device: &Device,
        config: &StreamConfig,
        callback: Arc<Mutex<Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>>>,
        channels: usize,
    ) -> Result<Stream, Box<dyn std::error::Error>> {
        let stream = device.build_input_stream(
            config,
            move |data: &[u8], _: &cpal::InputCallbackInfo| {
                let max_value = u8::MAX as f32;
                let mut channel_data = vec![vec![0.0; data.len() / channels]; channels];
                for (i, &sample) in data.iter().enumerate() {
                    let channel = i % channels;
                    let frame_idx = i / channels;
                    if frame_idx < channel_data[channel].len() {
                        channel_data[channel][frame_idx] = (sample as f32 / max_value) * 2.0 - 1.0;
                    }
                }
                callback.lock().unwrap()(channel_data);
            },
            |err| eprintln!("Stream error: {}", err),
            None,
        )?;
        Ok(stream)
    }

    /// Try to create a backend with alternative hosts if the default one doesn't find devices
    /// Check if a device is a loopback device (output device used for input capture)
    fn is_loopback_device(&self, device: &Device, original_index: usize) -> bool {
        // For now, we'll consider devices beyond the original input devices as potential loopback devices
        // In a more sophisticated implementation, we would check device properties or try to identify
        // WASAPI loopback devices specifically

        // Simple heuristic: if this device index is beyond the original input devices, it's likely a loopback device
        let original_input_count = match self.host.input_devices() {
            Ok(devices) => devices.count(),
            Err(_) => 0,
        };

        original_index >= original_input_count
    }

    pub fn new_with_fallback() -> Self {
        let default_host = cpal::default_host();
        let backend = Self::new_with_host(default_host);

        // If we didn't find any input devices, try alternative hosts
        if backend.input_devices.is_empty() {
            // Try WASAPI on Windows
            #[cfg(target_os = "windows")]
            {
                if let Ok(host) = cpal::host_from_id(cpal::HostId::Wasapi) {
                    let alt_backend = Self::new_with_host(host);
                    if !alt_backend.input_devices.is_empty() {
                        return alt_backend;
                    }
                }
            }

            // Try CoreAudio on macOS
            #[cfg(target_os = "macos")]
            {
                if let Ok(host) = cpal::host_from_id(cpal::HostId::CoreAudio) {
                    let alt_backend = Self::new_with_host(host);
                    if !alt_backend.input_devices.is_empty() {
                        return alt_backend;
                    }
                }
            }

            // Try ALSA on Linux
            #[cfg(target_os = "linux")]
            {
                if let Ok(host) = cpal::host_from_id(cpal::HostId::Alsa) {
                    let alt_backend = Self::new_with_host(host);
                    if !alt_backend.input_devices.is_empty() {
                        return alt_backend;
                    }
                }
            }
        }

        backend
    }
}
