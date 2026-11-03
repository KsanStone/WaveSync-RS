use crate::deref_arc;
use crate::sound::audio_worker::AudioWorker;
use crate::sound::capture_source::CaptureSource;
use crate::sound::cpal_audio_backend::CpalAudioBackend;
use crate::sound::loudness::LoudnessMeter;
use crate::sound::loudness::lufs::LufsLoudnessMeter;
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
        Self::with_backend(Box::new(CpalAudioBackend::new()))
    }

    fn with_backend(backend: Box<dyn crate::sound::audio_backend::AudioBackend>) -> Self {
        let mut planner = FftPlanner::new();
        AudioService(Arc::new(Inner {
            audio_backend: Mutex::new(backend),
            audio_buffer: Mutex::new(std::array::from_fn(|_| {
                let mut buffer = CircularBuffer::boxed();
                // this ensures a consistent size for all channels, even if the source is mono
                buffer.fill(0.0);
                buffer
            })),
            latest_fft: Mutex::new(std::array::from_fn(|_| vec![])),
            fft_scratch: Mutex::new(FftWork::default()),
            processing_worker: Mutex::new(None),
            loudness_snapshot: Mutex::new((Vec::new(), "LUFS")),
            fft_peaks: Mutex::new(std::array::from_fn(|_| None)),
            samples_written: Default::default(),
            channel_samples_written: Default::default(),
            last_fft_idx: Default::default(),
            last_frame_size: Default::default(),
            fft_rate: AtomicU32::new(60),
            current_capture_source: Mutex::new(None),
            fft_plan: Mutex::new(planner.plan_fft_forward(8192)),
            planner: Mutex::new(planner),
            fft_window: Mutex::new(FftWindow::new(WindowMethod::Hamming)),
            available_sources: Mutex::new(vec![]),
            fft_listeners: Mutex::new(vec![]),
            loudness_meter: Mutex::new(Box::new(LufsLoudnessMeter::new())),
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

        let source = if let Some(prev_device_id) = prev_device {
            devices
                .iter()
                .find(|x| x.id == prev_device_id)
                .cloned()
                .unwrap_or_else(|| backend.find_default_capture_source())
        } else {
            backend.find_default_capture_source()
        };
        self.start_capture(backend.as_mut(), source);
        self.available_sources.lock().unwrap().clear();
        self.available_sources.lock().unwrap().extend(devices);
    }

    pub fn update_source(&self, id: String) {
        if self.get_source().id == id {
            return;
        }
        let source = self
            .available_sources
            .lock()
            .unwrap()
            .iter()
            .find(|x| x.id == id)
            .cloned();
        if let Some(source) = source {
            info!("Switching to source: {}", source.name);
            self.start_capture(self.audio_backend.lock().unwrap().as_mut(), source);
        }
    }

    fn start_capture(
        &self,
        backend: &mut dyn crate::sound::audio_backend::AudioBackend,
        source: CaptureSource,
    ) {
        backend.stop_capture();
        // Join the old processor before resetting state. Its old capture closure
        // remains harmless even if the backend takes a moment to stop its stream.
        drop(self.processing_worker.lock().unwrap().take());
        {
            let mut buffers = self.audio_buffer.lock().unwrap();
            for buffer in buffers.iter_mut() {
                buffer.fill(0.0);
            }
            self.samples_written.store(0, Ordering::Release);
            for position in &self.channel_samples_written {
                position.store(0, Ordering::Release);
            }
        }
        self.last_fft_idx.store(0, Ordering::Release);
        self.last_frame_size.store(0, Ordering::Release);
        for fft in self.latest_fft.lock().unwrap().iter_mut() {
            fft.clear();
        }
        *self.fft_peaks.lock().unwrap() = [None; CHANNELS];
        *self.loudness_meter.lock().unwrap() = Box::new(LufsLoudnessMeter::new());
        self.loudness_snapshot.lock().unwrap().0.clear();
        self.current_capture_source
            .lock()
            .unwrap()
            .replace(source.clone());
        let weak = Arc::downgrade(&self.0);
        let rate = source.sample_rate.max(1);
        let (submitter, worker) =
            AudioWorker::start(source.channels.max(1) as usize, rate, move |frame, gap| {
                if let Some(inner) = weak.upgrade() {
                    inner.handle_samples(frame, rate, gap);
                }
            });
        *self.processing_worker.lock().unwrap() = Some(worker);
        backend.set_frame_callback(Box::new(move |frame| submitter.submit(frame)));
        backend.start_capture(source);
    }
}

pub struct Inner {
    pub audio_backend: Mutex<Box<dyn crate::sound::audio_backend::AudioBackend>>,
    pub fft_rate: AtomicU32,
    pub available_sources: Mutex<Vec<CaptureSource>>,
    pub audio_buffer: Mutex<[Box<CircularBuffer<384000, f32>>; CHANNELS]>,
    latest_fft: Mutex<[Vec<f32>; CHANNELS]>,
    fft_scratch: Mutex<FftWork>,
    processing_worker: Mutex<Option<AudioWorker>>,
    loudness_snapshot: Mutex<(Vec<f32>, &'static str)>,
    fft_peaks: Mutex<[Option<FftPeak>; CHANNELS]>,
    samples_written: AtomicU64,
    channel_samples_written: [AtomicU64; CHANNELS],
    last_fft_idx: AtomicU64,
    last_frame_size: AtomicU64,
    current_capture_source: Mutex<Option<CaptureSource>>,
    planner: Mutex<FftPlanner<f32>>,
    fft_plan: Mutex<Arc<dyn rustfft::Fft<f32>>>,
    fft_window: Mutex<FftWindow>,
    fft_listeners: Mutex<Vec<Box<dyn Visualizer>>>,
    loudness_meter: Mutex<Box<dyn LoudnessMeter>>,
}

// FFT input/workspace and published outputs retain their capacity between transforms.
#[derive(Default)]
struct FftScratch {
    input: Vec<Complex<f32>>,
    workspace: Vec<Complex<f32>>,
}

#[derive(Default)]
struct FftWork {
    scratch: FftScratch,
    samples: [Vec<f32>; CHANNELS],
    output: [Vec<f32>; CHANNELS],
}

impl FftScratch {
    fn transform(
        &mut self,
        plan: &dyn rustfft::Fft<f32>,
        samples: impl Iterator<Item = f32>,
        window: &[f32],
        scalar: f32,
        output: &mut Vec<f32>,
    ) {
        self.input.clear();
        self.input.extend(
            samples
                .zip(window)
                .map(|(sample, weight)| Complex::new(sample * weight, 0.0)),
        );
        self.workspace
            .resize(plan.get_inplace_scratch_len(), Complex::default());
        plan.process_with_scratch(&mut self.input, &mut self.workspace);
        output.clear();
        output.extend(
            self.input[..plan.len() / 2]
                .iter()
                .map(|value| value.norm() * scalar),
        );
    }
}
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

    fn handle_samples(&self, samples: &[Vec<f32>], sample_rate: u32, gap: u64) {
        if samples.is_empty() {
            return;
        }
        self.last_frame_size
            .store(samples[0].len() as u64, Ordering::Release);
        let mut buffer = self.audio_buffer.lock().unwrap();
        if gap != 0 {
            // Preserve the sample clock and represent dropped audio as silence,
            // rather than joining unrelated samples into a false continuous wave.
            for history in buffer.iter_mut() {
                if gap >= history.len() as u64 {
                    history.fill(0.0);
                } else {
                    history.extend(std::iter::repeat_n(0.0, gap as usize));
                }
            }
            let position = self.samples_written.fetch_add(gap, Ordering::AcqRel) + gap;
            for position in &self.channel_samples_written {
                position.fetch_add(gap, Ordering::AcqRel);
            }
            self.last_fft_idx.store(position, Ordering::Release);
        }
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
                for item in samples {
                    v += item[i];
                }
                buffer[0].push_back(v * scalar);
            }
            channels + 1
        };
        // Publish buffer positions while holding the same lock as the samples.
        for channel in 0..fft_channels {
            let count = if channel == 0 {
                samples[0].len()
            } else {
                samples[channel - 1].len()
            };
            self.channel_samples_written[channel].fetch_add(count as u64, Ordering::Release);
        }
        self.samples_written
            .fetch_add(samples[0].len() as u64, Ordering::Release);
        drop(buffer);
        if gap != 0 {
            *self.loudness_meter.lock().unwrap() = Box::new(LufsLoudnessMeter::new());
        }
        self.do_fft(fft_channels, sample_rate);
        self.do_loudness_calc(samples, sample_rate);
    }

    fn do_loudness_calc(&self, samples: &[Vec<f32>], sample_rate: u32) {
        let mut meter = self.loudness_meter.lock().unwrap();
        meter.process_frame(samples, sample_rate);
        // The UI reads only this published snapshot, never the working meter.
        *self.loudness_snapshot.lock().unwrap() = (meter.get_loudness(), meter.unit());
    }

    fn do_fft(&self, channels: usize, sample_rate: u32) {
        profiling::scope!("audio.process_fft");
        let fft_rate = self.fft_rate.load(Ordering::Acquire).max(1);
        let fft_sample_offset = (sample_rate as u64 / fft_rate as u64).max(1);
        let samples_written = self.samples_written.load(Ordering::Acquire);
        let mut last_fft_idx = self.last_fft_idx.load(Ordering::Acquire);
        let to_do = samples_written.saturating_sub(last_fft_idx) / fft_sample_offset;
        if to_do == 0 {
            return;
        }

        // Clone the Arc, releasing the plan lock before running any transforms.
        let plan = self.fft_plan.lock().unwrap().clone();
        let mut window = self.fft_window.lock().unwrap();
        let (window_scalar, window) = window.get_window(plan.len());
        let mut work = self.fft_scratch.lock().unwrap();
        let FftWork {
            scratch,
            samples,
            output,
        } = &mut *work;
        for _ in 0..to_do {
            last_fft_idx += fft_sample_offset;
            let offset = (samples_written - last_fft_idx) as usize;
            {
                profiling::scope!("audio.snapshot_fft_input");
                let buffers = self.audio_buffer.lock().unwrap();
                for i in 0..channels {
                    let end = buffers[i].len().saturating_sub(offset);
                    let start = end.saturating_sub(plan.len());
                    samples[i].clear();
                    if end - start == plan.len() {
                        samples[i].extend(buffers[i].range(start..end).copied());
                    }
                }
            }
            // Neither the sample history nor published FFT is locked during DSP.
            for i in 0..channels {
                if samples[i].is_empty() {
                    output[i].clear();
                    continue;
                }
                scratch.transform(
                    plan.as_ref(),
                    samples[i].iter().copied(),
                    window,
                    window_scalar,
                    &mut output[i],
                );
            }
            for data in &mut output[channels..] {
                data.clear();
            }
            let peaks = std::array::from_fn(|i| {
                if output[i].is_empty() {
                    None
                } else {
                    estimate_frequency_peak(&output[i], sample_rate as usize)
                }
            });
            {
                let listeners = self.fft_listeners.lock().unwrap();
                for listener in listeners.iter() {
                    listener.accept_fft(output, plan.len());
                }
            }
            // Swap retained allocations rather than copy the full result under the lock.
            std::mem::swap(&mut *self.latest_fft.lock().unwrap(), output);
            *self.fft_peaks.lock().unwrap() = peaks;
            self.last_fft_idx.store(last_fft_idx, Ordering::Release);
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
        let mut result = Vec::new();
        self.get_samples_aligned_into(channel, count, drop, take, &mut result);
        result
    }

    pub fn get_samples_aligned_into(
        &self,
        channel: AudioChannel,
        count: usize,
        drop: usize,
        take: usize,
        result: &mut Vec<f32>,
    ) {
        self.get_samples_aligned_with_position_into(channel, count, drop, take, result);
    }

    /// Snapshot the samples and their channel position atomically for phase tracking.
    pub fn get_samples_aligned_with_position_into(
        &self,
        channel: AudioChannel,
        count: usize,
        drop: usize,
        take: usize,
        result: &mut Vec<f32>,
    ) -> u64 {
        let buffers = self.audio_buffer.lock().unwrap();
        let buffer = &buffers[channel.get_index()];
        let start_idx = buffer
            .len()
            .saturating_sub(count)
            .saturating_add(drop)
            .min(buffer.len());
        let end_idx = start_idx.saturating_add(take).min(buffer.len());
        crate::refill_vec!(*result, buffer.range(start_idx..end_idx).copied());
        self.channel_samples_written[channel.get_index()].load(Ordering::Acquire)
    }

    pub fn get_fft_rate(&self) -> u32 {
        self.fft_rate.load(Ordering::Acquire)
    }

    pub fn get_fft(&self, channel: AudioChannel) -> Vec<f32> {
        let latest_fft = &self.latest_fft.lock().unwrap()[channel.get_index()];
        latest_fft.clone()
    }

    pub fn get_fft_into(&self, channel: AudioChannel, result: &mut Vec<f32>) {
        let latest_fft = self.latest_fft.lock().unwrap();
        crate::refill_vec!(*result, latest_fft[channel.get_index()].iter().copied());
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

    /// Per-channel position for caching immutable ranges of audio history.
    pub fn get_channel_samples_written(&self, channel: AudioChannel) -> u64 {
        self.channel_samples_written[channel.get_index()].load(Ordering::Acquire)
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
        self.loudness_snapshot.lock().unwrap().0.clone()
    }

    pub fn get_loudness_unit(&self) -> &'static str {
        self.loudness_snapshot.lock().unwrap().1
    }
}

#[cfg(test)]
mod fft_scratch_tests {
    use super::*;

    #[test]
    fn reused_fft_matches_original_transform_across_plan_changes() {
        let mut planner = FftPlanner::new();
        let mut scratch = FftScratch::default();
        let mut output = Vec::new();
        for size in [64, 128, 64] {
            let plan = planner.plan_fft_forward(size);
            let samples: Vec<_> = (0..size).map(|i| (i as f32 * 0.23).sin()).collect();
            let window: Vec<_> = (0..size)
                .map(|i| 0.5 - 0.5 * (i as f32 * 0.1).cos())
                .collect();
            let mut reference: Vec<_> = samples
                .iter()
                .zip(&window)
                .map(|(s, w)| Complex::new(s * w, 0.0))
                .collect();
            plan.process(&mut reference);
            scratch.transform(
                plan.as_ref(),
                samples.iter().copied(),
                &window,
                0.25,
                &mut output,
            );
            for (actual, expected) in output.iter().zip(&reference[..size / 2]) {
                assert!((actual - expected.norm() * 0.25).abs() < 1e-5);
            }
            let input_ptr = scratch.input.as_ptr();
            let workspace_ptr = scratch.workspace.as_ptr();
            let output_ptr = output.as_ptr();
            scratch.transform(
                plan.as_ref(),
                samples.iter().copied(),
                &window,
                0.25,
                &mut output,
            );
            assert_eq!(scratch.input.as_ptr(), input_ptr);
            assert_eq!(scratch.workspace.as_ptr(), workspace_ptr);
            assert_eq!(output.as_ptr(), output_ptr);
            assert_eq!(output.len(), size / 2);
        }
    }
}

#[cfg(test)]
mod processing_tests {
    use super::*;
    use crate::sound::audio_backend::{AudioBackend, OptionCaptureCallback};
    use crate::sound::audio_system::AudioSystem;
    use crate::ui::plot::{PlotData, XYPlotData};
    use crossbeam_channel::{Receiver, Sender, bounded};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct TestBackend {
        callback: Arc<Mutex<OptionCaptureCallback>>,
    }
    impl AudioBackend for TestBackend {
        fn detect_supported_capture_sources(&self) -> Vec<CaptureSource> {
            Vec::new()
        }
        fn detect_supported_audio_systems(&self) -> Vec<AudioSystem> {
            Vec::new()
        }
        fn find_default_capture_source(&self) -> CaptureSource {
            CaptureSource::default()
        }
        fn set_current_audio_system(&mut self, _: AudioSystem) {}
        fn get_current_audio_system(&self) -> AudioSystem {
            panic!("not used")
        }
        fn set_frame_callback(&mut self, callback: Box<dyn FnMut(&[Vec<f32>]) + Send + Sync>) {
            *self.callback.lock().unwrap() = Some(Arc::new(Mutex::new(callback)));
        }
        fn start_capture(&mut self, _: CaptureSource) {}
        fn stop_capture(&mut self) {}
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn wait_for(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !ready() {
            assert!(
                Instant::now() < deadline,
                "audio worker did not make progress"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    struct PausedListener {
        entered: Sender<()>,
        resume: Receiver<()>,
    }
    impl Visualizer for PausedListener {
        fn get_plot_data(&self) -> PlotData {
            PlotData::XY(XYPlotData::default())
        }
        fn min_canvas_size(&self) -> egui::Vec2 {
            egui::Vec2::ZERO
        }
        fn get_draw_callback(
            &self,
            _: egui::Rect,
            _: &crate::wavesync::WaveSyncVisuals,
        ) -> Option<egui::epaint::PaintCallback> {
            None
        }
        fn accept_fft(&self, _: &[Vec<f32>; CHANNELS], _: usize) {
            let _ = self.entered.try_send(());
            let _ = self.resume.recv_timeout(Duration::from_secs(2));
        }
    }

    #[test]
    fn fft_listener_cannot_block_render_snapshot_locks_or_capture() {
        let backend = TestBackend::default();
        let callback = backend.callback.clone();
        let service = AudioService::with_backend(Box::new(backend));
        service.update_fft_plan(64);
        service.update_fft_rate(3000);
        let (entered_tx, entered_rx) = bounded(1);
        let (resume_tx, resume_rx) = bounded(1);
        service.register_fft_listener(Box::new(PausedListener {
            entered: entered_tx,
            resume: resume_rx,
        }));
        service.start_capture(
            service.audio_backend.lock().unwrap().as_mut(),
            CaptureSource {
                channels: 2,
                sample_rate: 192000,
                ..CaptureSource::default()
            },
        );
        let capture = callback.lock().unwrap().as_ref().unwrap().clone();
        capture.lock().unwrap()(&[vec![0.5; 64], vec![-0.25; 64]]);
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        // Worker is blocked in its FFT listener. These are the UI-facing locks.
        assert!(service.audio_buffer.try_lock().is_ok());
        assert!(service.latest_fft.try_lock().is_ok());
        assert!(service.fft_plan.try_lock().is_ok());
        assert!(service.fft_peaks.try_lock().is_ok());
        assert!(service.loudness_snapshot.try_lock().is_ok());
        assert_eq!(service.get_samples(AudioChannel::Master, 4), vec![0.125; 4]);
        // Capture still returns while DSP is stalled.
        capture.lock().unwrap()(&[vec![0.25; 64], vec![0.25; 64]]);
        resume_tx.send(()).unwrap();
        drop(resume_tx);
        // Do not keep the listener's bounded notification channel full.
        drop(entered_rx);
        wait_for(|| service.last_fft_idx.load(Ordering::Acquire) >= 64);
        assert_eq!(service.get_fft(AudioChannel::Master).len(), 32);
        drop(service.processing_worker.lock().unwrap().take());
    }

    #[test]
    fn source_switch_retires_old_capture_and_resets_processing_state() {
        let backend = TestBackend::default();
        let callback = backend.callback.clone();
        let service = AudioService::with_backend(Box::new(backend));
        let source = CaptureSource {
            channels: 2,
            sample_rate: 192000,
            ..CaptureSource::default()
        };
        service.start_capture(
            service.audio_backend.lock().unwrap().as_mut(),
            source.clone(),
        );
        let old = callback.lock().unwrap().as_ref().unwrap().clone();
        old.lock().unwrap()(&[vec![0.5; 256], vec![0.25; 256]]);
        wait_for(|| service.get_samples_written() == 256);
        service.start_capture(
            service.audio_backend.lock().unwrap().as_mut(),
            CaptureSource {
                id: "new".into(),
                sample_rate: 96000,
                ..source
            },
        );
        old.lock().unwrap()(&[vec![1.0; 256], vec![1.0; 256]]);
        assert_eq!(service.get_samples_written(), 0);
        assert!(service.get_fft(AudioChannel::Master).is_empty());
        let new = callback.lock().unwrap().as_ref().unwrap().clone();
        new.lock().unwrap()(&[vec![-0.5; 256], vec![-0.5; 256]]);
        wait_for(|| service.get_samples_written() == 256);
        assert_eq!(service.get_samples(AudioChannel::Master, 4), vec![-0.5; 4]);
        drop(service.processing_worker.lock().unwrap().take());
    }

    #[test]
    fn dropped_samples_advance_clock_with_a_silent_gap() {
        let service = AudioService::with_backend(Box::<TestBackend>::default());
        service.handle_samples(&[vec![0.5; 4], vec![0.5; 4]], 192000, 0);
        service.handle_samples(&[vec![-0.5; 4], vec![-0.5; 4]], 192000, 3);
        assert_eq!(service.get_samples_written(), 11);
        assert_eq!(service.get_channel_samples_written(AudioChannel::Left), 11);
        assert_eq!(
            service.get_samples(AudioChannel::Master, 11),
            vec![0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, -0.5, -0.5, -0.5, -0.5]
        );
        assert_eq!(service.last_fft_idx.load(Ordering::Acquire), 7);
    }
}
