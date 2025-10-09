use crate::sound::AudioChannel;
use crate::sound::capture_source::CaptureSource;
use crate::sound::cpal_audio_backend::CpalAudioBackend;
use circular_buffer::CircularBuffer;
use log::debug;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const CHANNELS: usize = 1;

#[derive(Clone)]
pub struct AudioService(Arc<Inner>);

impl Deref for AudioService {
    type Target = Inner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AudioService {
    pub fn new() -> Self {
        let mut planner = FftPlanner::new();
        AudioService(Arc::new(Inner {
            audio_backend: Mutex::new(Box::new(CpalAudioBackend::new_with_fallback())),
            audio_buffer: Mutex::new([CircularBuffer::boxed()]),
            latest_fft: Mutex::new([vec![]]),
            samples_written: Default::default(),
            last_fft_idx: Default::default(),
            current_capture_source: Mutex::new(None),
            fft_plan: Mutex::new(planner.plan_fft_forward(256)),
            planner: Mutex::new(planner),
        }))
    }

    pub fn init(&self) {
        let mut backend = self.audio_backend.lock().unwrap();

        // Print detected audio devices
        let devices = backend.detect_supported_capture_sources();
        println!("🎵 Detected Audio Devices:");
        if devices.is_empty() {
            println!("   ❌ No audio input devices found!");
        } else {
            for (i, device) in devices.iter().enumerate() {
                println!(
                    "   {}. {} ({}ch, {}Hz, {:?})",
                    i + 1,
                    device.name,
                    device.channels,
                    device.sample_rate,
                    device.format
                );
            }
        }

        // Print audio systems
        let systems = backend.detect_supported_audio_systems();
        println!("\n🔊 Audio Systems:");
        for system in &systems {
            println!("   {}", system.name);
        }

        let weak_inner = Arc::downgrade(&self.0);
        backend.set_frame_callback(Box::new(move |frame| {
            let upgrade_result = weak_inner.upgrade();
            if let Some(inner) = upgrade_result {
                inner.handle_samples(frame);
            }
        }));

        let source = backend.find_default_capture_source();
        println!(
            "\n▶️  Starting capture from: {} (ID: {})",
            source.name, source.id
        );
        backend.start_capture(source)
    }
}

pub struct Inner {
    audio_backend: Mutex<Box<dyn crate::sound::audio_backend::AudioBackend>>,
    audio_buffer: Mutex<[Box<CircularBuffer<48000, f32>>; CHANNELS]>,
    latest_fft: Mutex<[Vec<f32>; CHANNELS]>,
    samples_written: AtomicU64,
    last_fft_idx: AtomicU64,
    current_capture_source: Mutex<Option<CaptureSource>>,
    planner: Mutex<FftPlanner<f32>>,
    fft_plan: Mutex<Arc<dyn rustfft::Fft<f32>>>,
}

impl Inner {
    pub fn update_fft_plan(&self, size: usize) {
        let fft_plan = self.planner.lock().unwrap().plan_fft_forward(size);
        *self.fft_plan.lock().unwrap() = fft_plan;
    }

    pub fn handle_samples(&self, samples: Vec<Vec<f32>>) {
        let mut buffer = self.audio_buffer.lock().unwrap();
        let fft_channels = if samples.len() == 1 {
            // Mono audio, no need to mix channels
            buffer[0].extend_from_slice(samples[0].deref());
            1
        } else {
            // Stereo audio, store per-channel data, and later mix them together into master buffer
            let channels = buffer.len().saturating_sub(1).min(samples.len());
            for i in 0..channels {
                buffer[i + 1].extend_from_slice(samples[i].deref());
            }
            channels + 1
        };
        drop(buffer);
        self.samples_written
            .fetch_add(samples[0].len() as u64, Ordering::Release);
        self.do_fft(fft_channels);
    }

    fn do_fft(&self, channels: usize) {
        let fft_sample_offset = 4096;
        let samples_written = self.samples_written.load(Ordering::Acquire);
        let mut last_fft_idx = self.last_fft_idx.load(Ordering::Acquire);
        let mut latest_fft = self.latest_fft.lock().unwrap();
        let since_last_fft = samples_written - last_fft_idx;
        let to_do = since_last_fft / fft_sample_offset;
        let audio_buffer = self.audio_buffer.lock().unwrap();
        let fft_plan = self.fft_plan.lock().unwrap();

        for _ in 0..to_do {
            last_fft_idx += fft_sample_offset;
            let offset_from_buffer_end = samples_written - last_fft_idx;

            for i in 0..channels {
                if audio_buffer[i].len() < fft_plan.len() {
                    continue;
                }
                let end_idx = audio_buffer[i]
                    .len()
                    .saturating_sub(offset_from_buffer_end as usize);
                let start_idx = end_idx.saturating_sub(fft_plan.len());
                if end_idx - start_idx != fft_plan.len() {
                    debug!(
                        "Buffer too smol {} {} {}",
                        end_idx,
                        start_idx,
                        end_idx - start_idx
                    );
                    continue;
                } else {
                    debug!(
                        "Buffer ok {} {} {}",
                        end_idx,
                        start_idx,
                        end_idx - start_idx
                    );
                }

                let sample_iter = audio_buffer[i].range(start_idx..end_idx);
                let mut fft_in = sample_iter
                    .map(|sample| Complex::new(*sample, 0.0f32))
                    .collect::<Vec<_>>();
                fft_plan.process(&mut fft_in);
                let mut magnitudes = fft_in.iter().map(|x| x.norm()).collect::<Vec<_>>();
                let scalar = 1.0 / magnitudes.len() as f32;
                magnitudes.iter_mut().for_each(|x| *x *= scalar);
                latest_fft[i] = magnitudes;
            }
            self.last_fft_idx.store(last_fft_idx, Ordering::Release);
        }
    }

    pub fn get_samples(&self, channel: AudioChannel, count: usize) -> Vec<f32> {
        let buffer = &self.audio_buffer.lock().unwrap()[channel.get_index()];
        buffer.range(..count).copied().collect()
    }

    pub fn get_fft(&self, channel: AudioChannel) -> Vec<f32> {
        let latest_fft = &self.latest_fft.lock().unwrap()[channel.get_index()];
        latest_fft.clone()
    }
}
