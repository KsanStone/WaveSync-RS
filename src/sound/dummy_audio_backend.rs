use crate::sound::audio_backend::{AudioBackend, OptionCaptureCallback};
use crate::sound::audio_system::AudioSystem;
use crate::sound::capture_source::CaptureSource;
use crate::sound::{AudioBackendType, SampleFormat};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// Used for testing purposes, to avoid having to have a real audio device.
pub struct DummyAudioBackend {
    capture_callback: OptionCaptureCallback,
    audio_system: AudioSystem,
    capture_source: CaptureSource,
    sequence_index: Arc<AtomicUsize>,
}

impl DummyAudioBackend {
    pub fn new() -> Self {
        DummyAudioBackend {
            capture_callback: None,
            audio_system: AudioSystem {
                name: String::from("Dummy Audio System"),
                backend: AudioBackendType::Dummy,
            },
            capture_source: CaptureSource {
                name: "Dummy Capture Source".to_string(),
                id: "dcs".to_string(),
                channels: 1,
                sample_rate: 48000,
                format: SampleFormat::F32,
                is_loopback: false,
                backend: AudioBackendType::Dummy,
            },
            sequence_index: Default::default(),
        }
    }
}

impl AudioBackend for DummyAudioBackend {
    fn detect_supported_capture_sources(&self) -> Vec<CaptureSource> {
        vec![self.capture_source.clone()]
    }

    fn detect_supported_audio_systems(&self) -> Vec<AudioSystem> {
        vec![self.audio_system.clone()]
    }

    fn find_default_capture_source(&self) -> CaptureSource {
        self.capture_source.clone()
    }

    fn set_current_audio_system(&mut self, _system: AudioSystem) {}

    fn get_current_audio_system(&self) -> AudioSystem {
        self.audio_system.clone()
    }

    fn set_frame_callback(&mut self, callback: Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>) {
        self.capture_callback = Some(Arc::new(Mutex::new(callback)));
    }

    fn start_capture(&mut self, _source: CaptureSource) {
        let callback_weak = Arc::downgrade(self.capture_callback.as_ref().unwrap());
        let sequence_index = self.sequence_index.clone();
        thread::spawn(move || {
            const FREQS: [f32; 4] = [8000.0, 2333.0, 16000.0, 100.0];
            const PHASES: [f32; 4] = [0.0, 0.25, 0.5, 0.1];
            const SCALAR: f32 = 1.0 / FREQS.len() as f32;
            loop {
                if let Some(callback) = callback_weak.upgrade() {
                    let mut frame = vec![0.0; 100];
                    for item in &mut frame {
                        let seq = sequence_index.fetch_add(1, Ordering::Relaxed) as f32;
                        for i in 0..FREQS.len() {
                            let div = FREQS[i];
                            let phase = PHASES[i];
                            *item += (seq / div + phase)
                                .sin() * SCALAR
                        }
                    }
                    callback.lock().unwrap()(vec![frame]);

                } else {
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(8));
            }
        });
    }

    fn stop_capture(&mut self) {}
}
