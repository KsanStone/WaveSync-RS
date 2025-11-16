use crate::deref_arc;
use crate::sound::capture_source::CaptureSource;
use crate::sound::cpal_audio_backend::CpalAudioBackend;
use crate::sound::loudness::LoudnessMeter;
use crate::sound::loudness::rms::RmsLoudnessMeter;
use crate::sound::windowing::{FftWindow, WindowMethod};
use crate::sound::{AudioChannel, FftPeak, estimate_frequency_peak};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::wavesync::{FREQ_LABEL_FONT_SIZE, stable_num};
use circular_buffer::CircularBuffer;
use egui::RichText;
use log::info;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use std::ops::Deref;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const CHANNELS: usize = 3;

deref_arc!(AudioService);

impl Default for AudioService {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioService {
    pub fn new() -> Self {
        let mut planner = FftPlanner::new();

        AudioService(Arc::new(Inner {
            audio_backend: Mutex::new(Box::new(CpalAudioBackend::new())),
            audio_buffer: Mutex::new(std::array::from_fn(|_| {
                let mut buffer = CircularBuffer::boxed();
                // this ensures a consistent size for all channels, even if the source is mono
                buffer.fill(0.0);
                buffer
            })),
            latest_fft: Mutex::new(std::array::from_fn(|_| vec![])),
            fft_peaks: Mutex::new(std::array::from_fn(|_| None)),
            samples_written: Default::default(),
            last_fft_idx: Default::default(),
            last_frame_size: Default::default(),
            fft_rate: AtomicU32::new(60),
            current_capture_source: Mutex::new(None),
            fft_plan: Mutex::new(planner.plan_fft_forward(8192)),
            planner: Mutex::new(planner),
            fft_window: Mutex::new(FftWindow::new(WindowMethod::Hamming)),
            available_sources: Mutex::new(vec![]),
            fft_listeners: Mutex::new(vec![]),
            loudness_meter: Mutex::new(Box::new(RmsLoudnessMeter::new())),
        }))
    }

    pub fn init(&self, prev_device: Option<String>) {
        let mut backend = self.audio_backend.lock().unwrap();

        // Print detected audio devices
        let devices = backend.detect_supported_capture_sources();
        info!("Detected Audio Devices:");
        if devices.is_empty() {
            info!("   No audio input devices found!");
        } else {
            for (i, device) in devices.iter().enumerate() {
                info!(
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
        info!("Audio Systems:");
        for system in &systems {
            info!("   {}", system.name);
        }

        let weak_inner = Arc::downgrade(&self.0);
        backend.set_frame_callback(Box::new(move |frame| {
            let upgrade_result = weak_inner.upgrade();
            if let Some(inner) = upgrade_result {
                inner.handle_samples(frame);
            }
        }));

        let source = if let Some(prev_device_id) = prev_device {
            devices
                .iter()
                .find(|x| x.id == prev_device_id)
                .cloned()
                .unwrap_or_else(|| backend.find_default_capture_source())
        } else {
            backend.find_default_capture_source()
        };
        backend.start_capture(source.clone());
        self.current_capture_source.lock().unwrap().replace(source);
        self.available_sources.lock().unwrap().clear();
        self.available_sources.lock().unwrap().extend(devices);
    }
}

pub struct Inner {
    pub audio_backend: Mutex<Box<dyn crate::sound::audio_backend::AudioBackend>>,
    pub fft_rate: AtomicU32,
    pub available_sources: Mutex<Vec<CaptureSource>>,
    pub audio_buffer: Mutex<[Box<CircularBuffer<384000, f32>>; CHANNELS]>,
    latest_fft: Mutex<[Vec<f32>; CHANNELS]>,
    fft_peaks: Mutex<[Option<FftPeak>; CHANNELS]>,
    samples_written: AtomicU64,
    last_fft_idx: AtomicU64,
    last_frame_size: AtomicU64,
    current_capture_source: Mutex<Option<CaptureSource>>,
    planner: Mutex<FftPlanner<f32>>,
    fft_plan: Mutex<Arc<dyn rustfft::Fft<f32>>>,
    fft_window: Mutex<FftWindow>,
    fft_listeners: Mutex<Vec<Box<dyn Visualizer>>>,
    loudness_meter: Mutex<Box<dyn LoudnessMeter>>,
}

// TODO: Ensure that we clone fft and audio buffers as little as possible.
impl Inner {
    pub fn register_fft_listener(&self, listener: Box<dyn Visualizer>) {
        self.fft_listeners.lock().unwrap().push(listener);
    }

    pub fn update_fft_plan(&self, size: usize) {
        if size > 0 {
            let fft_plan = self.planner.lock().unwrap().plan_fft_forward(size);
            *self.fft_plan.lock().unwrap() = fft_plan;
        }
    }

    pub fn update_fft_rate(&self, rate: u32) {
        if rate >= 1 {
            self.fft_rate.store(rate, Ordering::Release);
        }
    }

    pub fn update_source(&self, id: String) {
        let current_id = self.get_source().id;

        if current_id != id {
            let available = self.available_sources.lock().unwrap();
            let mut audio_backend = self.audio_backend.lock().unwrap();
            let new_source = available.iter().find(|x| x.id == id);
            if let Some(new_source) = new_source.cloned() {
                info!("Switching to source: {}", new_source.name);
                audio_backend.stop_capture();
                audio_backend.start_capture(new_source.clone());
                self.current_capture_source
                    .lock()
                    .unwrap()
                    .replace(new_source);
            }
        }
    }

    pub fn handle_samples(&self, samples: Vec<Vec<f32>>) {
        if samples.is_empty() {
            return;
        }
        self.last_frame_size
            .store(samples[0].len() as u64, Ordering::Release);
        let mut buffer = self.audio_buffer.lock().unwrap();
        let fft_channels = if samples.len() == 1 {
            // Mono audio, no need to mix channels
            buffer[0].extend_from_slice(samples[0].deref());
            1
        } else {
            // Stereo audio, store per-channel data, and later mix them together into master buffer
            let channels = buffer.len().saturating_sub(1).max(1).min(samples.len());
            for i in 0..channels {
                buffer[i + 1].extend_from_slice(samples[i].deref());
            }
            let scalar = 1.0 / channels as f32;
            for i in 0..samples[0].len() {
                let mut v = 0.0;
                for item in &samples {
                    v += item[i];
                }
                buffer[0].push_back(v * scalar);
            }
            channels + 1
        };
        drop(buffer);
        self.samples_written
            .fetch_add(samples[0].len() as u64, Ordering::Release);
        self.do_fft(fft_channels);
        self.do_loudness_calc(&samples);
    }

    fn do_loudness_calc(&self, samples: &[Vec<f32>]) {
        let mut loudness_meter = self.loudness_meter.lock().unwrap();
        loudness_meter.process_frame(samples);
    }

    fn do_fft(&self, channels: usize) {
        let fft_rate = self.fft_rate.load(Ordering::Acquire);
        let source = self.get_source();
        let fft_sample_offset = source.sample_rate as u64 / fft_rate as u64;

        let samples_written = self.samples_written.load(Ordering::Acquire);
        let mut last_fft_idx = self.last_fft_idx.load(Ordering::Acquire);
        let mut latest_fft = self.latest_fft.lock().unwrap();
        let since_last_fft = samples_written - last_fft_idx;
        let to_do = since_last_fft / fft_sample_offset;
        let audio_buffer = self.audio_buffer.lock().unwrap();
        let fft_plan = self.fft_plan.lock().unwrap();
        let mut fft_window = self.fft_window.lock().unwrap();
        let mut fft_peaks = self.fft_peaks.lock().unwrap();
        let current_capture_source = self.current_capture_source.lock().unwrap();
        let listeners = self.fft_listeners.lock().unwrap();
        if current_capture_source.is_none() {
            return;
        }

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
                    continue;
                }

                let sample_iter = audio_buffer[i].range(start_idx..end_idx);
                let (window_scalar, window) = fft_window.get_window(fft_plan.len());

                let mut fft_in = sample_iter
                    .enumerate()
                    .map(|(i, sample)| Complex::new(*sample * window[i], 0.0f32))
                    .collect::<Vec<_>>();
                fft_plan.process(&mut fft_in);

                let mut magnitudes = fft_in[0..fft_plan.len() / 2]
                    .iter()
                    .map(|x| x.norm())
                    .collect::<Vec<_>>();

                magnitudes.iter_mut().for_each(|x| *x *= window_scalar);
                latest_fft[i] = magnitudes;
            }
            self.last_fft_idx.store(last_fft_idx, Ordering::Release);
            listeners.iter().for_each(|listener| {
                listener.accept_fft(&latest_fft, fft_plan.len());
            })
        }

        let rate = current_capture_source.as_ref().unwrap().sample_rate;
        for i in 0..channels {
            fft_peaks[i] = estimate_frequency_peak(&latest_fft[i], rate as usize);
        }
    }

    pub fn get_samples(&self, channel: AudioChannel, count: usize) -> Vec<f32> {
        self.get_samples_aligned(channel, count, 0, count)
    }

    pub fn get_samples_aligned(
        &self,
        channel: AudioChannel,
        count: usize,
        drop: usize,
        take: usize,
    ) -> Vec<f32> {
        let buffer = &self.audio_buffer.lock().unwrap()[channel.get_index()];
        let start_idx = buffer.len().saturating_sub(count).saturating_add(drop);
        let end_idx = start_idx.saturating_add(take).min(buffer.len());
        buffer.range(start_idx..end_idx).copied().collect()
    }

    pub fn get_fft_rate(&self) -> u32 {
        self.fft_rate.load(Ordering::Acquire)
    }

    pub fn get_fft(&self, channel: AudioChannel) -> Vec<f32> {
        let latest_fft = &self.latest_fft.lock().unwrap()[channel.get_index()];
        latest_fft.clone()
    }

    pub fn get_fft_size(&self) -> usize {
        self.fft_plan.lock().unwrap().len()
    }

    pub fn get_fft_peak(&self, channel: AudioChannel) -> Option<FftPeak> {
        self.fft_peaks.lock().unwrap()[channel.get_index()]
    }

    pub fn set_fft_size(&self, size: usize) {
        if size == self.fft_plan.lock().unwrap().len() {
            return;
        }
        self.update_fft_plan(size);
    }

    pub fn get_samples_written(&self) -> u64 {
        self.samples_written.load(Ordering::Acquire)
    }

    pub fn get_device_name(&self) -> Option<String> {
        self.current_capture_source
            .lock()
            .unwrap()
            .as_ref()
            .map(|x| x.id.clone())
    }

    pub fn get_source(&self) -> CaptureSource {
        self.current_capture_source
            .lock()
            .unwrap()
            .as_ref()
            .cloned()
            .unwrap_or_default()
    }

    /// Returns the count of audio channels with which sth is being done.
    /// If the source is mono, only one channel is active
    /// Otherwise, all source channels + the master combined channel are active.
    pub fn get_active_audio_channels(&self) -> u32 {
        let source = self.get_source();
        if source.channels == 1 {
            1
        } else {
            source.channels + 1
        }
    }

    /// Compute the label with info about estimated frequency peaks for each
    /// currently active channel
    pub fn get_peak_labels(&self) -> Vec<RichText> {
        let channels = self.get_active_audio_channels();
        let mut labels = vec![];
        for i in 0..channels {
            let audio_chanel: Result<AudioChannel, ()> = i.try_into();
            if let Ok(audio_chanel) = audio_chanel {
                let fft_peak = self.get_fft_peak(audio_chanel);
                let label = RichText::new(
                    if let Some(fft_peak) = fft_peak
                        && fft_peak.interpolated_frequency >= 0.0
                        && fft_peak.interpolated_frequency <= 96000.0
                    {
                        format!(
                            "{}: {}",
                            audio_chanel.get_label(),
                            stable_num(7, 1, fft_peak.interpolated_frequency)
                        )
                    } else {
                        format!("{}: -----.-", audio_chanel.get_label())
                    },
                )
                .monospace()
                .size(FREQ_LABEL_FONT_SIZE);
                labels.push(label);
            } else {
                break;
            }
        }
        labels
    }

    pub fn get_last_frame_size(&self) -> u64 {
        self.last_frame_size.load(Ordering::Acquire)
    }

    /// Return the freq present in the last fft bin
    /// based on the current sources' sample rate
    pub fn get_max_freq(&self) -> u32 {
        self.get_source().calculate_nyquist_frequency() as u32
    }

    pub fn get_loudness_values(&self) -> Vec<f32> {
        let meter = self.loudness_meter.lock().unwrap();
        meter.get_loudness()
    }
}
