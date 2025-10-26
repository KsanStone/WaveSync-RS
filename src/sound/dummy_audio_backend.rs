use itertools::Itertools;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use crate::sound::audio_backend::{AudioBackend, OptionCaptureCallback};
use crate::sound::audio_system::AudioSystem;
use crate::sound::capture_source::CaptureSource;
use crate::sound::{AudioBackendType, SampleFormat};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use log::info;
use rand::Rng;

/// Used for testing purposes, to avoid having to have a real audio device.
pub struct DummyAudioBackend {
    capture_callback: OptionCaptureCallback,
    audio_system: AudioSystem,
    capture_source: Arc<Mutex<CaptureSource>>,
    sequence_index: Arc<AtomicUsize>,
    pub(crate) pattern_data: Arc<PatternData>
}

pub struct PatternData {
    pub test_sounds: Vec<PathBuf>,
    pub pattern: Mutex<TestPattern>,
    pub sound_index: AtomicUsize,
    pub current_sound_index: Mutex<Option<usize>>,
    pub sequencer_frequency: Mutex<f32>,
    pub reader: Mutex<Option<hound::WavReader<BufReader<File>>>>
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TestPattern {
    WaveSynth,
    SampleSound
}

impl Default for DummyAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl DummyAudioBackend {
    pub fn new() -> Self {

        let mut test_sounds = vec![];
        if let Ok(dir) = std::fs::read_dir("test_sounds") {
            for entry in dir.flatten() {
                let path = entry.path();
                if path.is_file() {
                    test_sounds.push(path);
                }
            }
        }

        for sound in test_sounds.iter() {
            info!("Found test sound: {}", sound.to_str().unwrap());
        }

        DummyAudioBackend {
            capture_callback: None,
            audio_system: AudioSystem {
                name: String::from("Dummy Audio System"),
                backend: AudioBackendType::Dummy,
            },
            capture_source: Arc::new(Mutex::new(CaptureSource {
                name: "Dummy Capture Source".to_string(),
                id: "dcs".to_string(),
                channels: 2,
                sample_rate: 48000,
                format: SampleFormat::F32,
                is_loopback: false,
                backend: AudioBackendType::Dummy,
            })),
            sequence_index: Default::default(),
            pattern_data: Arc::new(PatternData {
                test_sounds,
                pattern: Mutex::new(TestPattern::SampleSound),
                sound_index: Default::default(),
                current_sound_index: Mutex::new(None),
                sequencer_frequency: Mutex::new(100.0),
                reader: Mutex::new(None)
            })
        }
    }
}

impl AudioBackend for DummyAudioBackend {
    fn detect_supported_capture_sources(&self) -> Vec<CaptureSource> {
        vec![self.capture_source.lock().unwrap().clone()]
    }

    fn detect_supported_audio_systems(&self) -> Vec<AudioSystem> {
        vec![self.audio_system.clone()]
    }

    fn find_default_capture_source(&self) -> CaptureSource {
        self.capture_source.lock().unwrap().clone()
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
        let pattern_data = self.pattern_data.clone();
        let source = self.capture_source.clone();

        thread::spawn(move || {
            loop {

                if let Some(callback) = callback_weak.upgrade() {
                    let mut frame_l = vec![0.0; 960];
                    let mut frame_r = vec![0.0; 960];
                    let pattern = { *pattern_data.pattern.lock().unwrap() };


                    match pattern {
                        TestPattern::WaveSynth => {
                            synthesize_frames(&mut frame_l, &mut frame_r, &sequence_index, &pattern_data.sequencer_frequency);
                        }
                        TestPattern::SampleSound => {
                            let sound_idx = pattern_data.sound_index.load(Ordering::Relaxed);
                            let mut current_sound_idx = pattern_data.current_sound_index.lock().unwrap();
                            let mut reader = pattern_data.reader.lock().unwrap();
                            info!("{} {}", sound_idx, pattern_data.test_sounds.len());
                            if sound_idx < pattern_data.test_sounds.len() {
                                let sound_path = &pattern_data.test_sounds[sound_idx];
                                if Some(sound_idx) != *current_sound_idx {
                                    info!("Playing sound: {}", sound_path.to_str().unwrap());
                                    *current_sound_idx = Some(sound_idx);
                                    let new_reader = hound::WavReader::open(sound_path).unwrap();
                                    *reader = Some(new_reader);
                                }

                                if let Some(reader) = reader.as_mut() {
                                    let samples = reader.samples::<i32>();
                                    let b24_max = 0x7fffff;
                                    for (l, r) in samples
                                        .take(960)
                                        .map(Result::unwrap)
                                        .tuples()
                                    {
                                        frame_l.push(l as f32 / b24_max as f32);
                                        frame_r.push(r as f32 / b24_max as f32);
                                    }
                                }
                            }
                        }
                    }



                    callback.lock().unwrap()(vec![frame_l, frame_r]);
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

fn sample_wav(frame_l: &mut [f32], frame_r: &mut [f32]) {
    let frame_size = frame_l.len();
}

fn synthesize_frames(frame_l: &mut [f32], frame_r: &mut [f32], sequence_index: &AtomicUsize, sequencer_frequency: &Mutex<f32>) {
    let sequencer_frequency = { *sequencer_frequency.lock().unwrap() };
    let frequencies = vec![sequencer_frequency];
    let scalar: f32 = 1.0 / frequencies.len() as f32;

    let phase_offset = rand::rng().random_range(0.495..0.505);
    for (l, r) in frame_l.iter_mut().zip(frame_r.iter_mut()) {
        let seq_l = sequence_index.fetch_add(1, Ordering::Relaxed) as f32
            % (100000.0 * 2.0 * std::f32::consts::PI);
        for freq in &frequencies {
            let wave_length = 48000.0 / freq;
            let phase_offset = wave_length * 2.0 * std::f32::consts::PI * phase_offset;

            *l += (seq_l / wave_length * 2.0 * std::f32::consts::PI).sin() * scalar;
            *r += ((seq_l + phase_offset) / wave_length * 2.0 * std::f32::consts::PI).sin() * scalar;
        }
    }
}