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
    pub sequencer_frequency: Arc<Mutex<f32>>,
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
            sequencer_frequency: Arc::new(Mutex::new(100.0)),
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
        let sequencer_frequency = self.sequencer_frequency.clone();
        thread::spawn(move || {
            loop {
                let sequencer_frequency = { *sequencer_frequency.lock().unwrap() };
                let frequencies = vec![sequencer_frequency];
                let scalar: f32 = 1.0 / frequencies.len() as f32;
                if let Some(callback) = callback_weak.upgrade() {
                    let mut frame = vec![0.0; 960];
                    for item in &mut frame {
                        let seq = sequence_index.fetch_add(1, Ordering::Relaxed) as f32
                            % (100000.0 * 2.0 * std::f32::consts::PI);
                        for freq in &frequencies {
                            let wave_length = 48000.0 / freq;

                            *item += (seq / wave_length * 2.0 * std::f32::consts::PI).sin() * scalar
                        }
                    }
                    callback.lock().unwrap()(vec![frame]);
                } else {
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(20));
            }
        });
    }

    fn stop_capture(&mut self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
