use std::ops::Deref;
use std::sync::{Arc, Mutex};
use circular_buffer::CircularBuffer;
use rustfft::FftPlanner;
use crate::sound::AudioChannel;
use crate::sound::capture_source::CaptureSource;
use crate::sound::dummy_audio_backend::DummyAudioBackend;

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
            audio_backend: Mutex::new(Box::new(DummyAudioBackend::new())),
            audio_buffer: Mutex::new([CircularBuffer::boxed()]),
            current_capture_source: Mutex::new(None),
            fft_plan: Mutex::new(planner.plan_fft_forward(4096)),
            planner: Mutex::new(planner),
        }))
    }

    pub fn init(&self) {
        let mut backend = self.audio_backend.lock().unwrap();
        let weak_inner = Arc::downgrade(&self.0);
        backend.set_frame_callback(Box::new(move |frame| {
            let upgrade_result = weak_inner.upgrade();
            if let Some(inner) = upgrade_result {
                inner.handle_samples(frame);
            }
        }));
        let source = backend.find_default_capture_source();
        backend.start_capture(source)
    }
}

pub struct Inner {
    audio_backend: Mutex<Box<dyn crate::sound::audio_backend::AudioBackend>>,
    audio_buffer: Mutex<[Box<CircularBuffer<48000, f32>>; 1]>,
    current_capture_source: Mutex<Option<CaptureSource>>,
    planner: Mutex<FftPlanner<f32>>,
    fft_plan: Mutex<Arc<dyn rustfft::Fft<f32>>>
}

impl Inner {

    pub fn update_fft_plan(&self, size: usize) {
        let fft_plan = self.planner.lock().unwrap().plan_fft_forward(size);
        *self.fft_plan.lock().unwrap() = fft_plan;
    }

    pub fn handle_samples(&self, samples: Vec<Vec<f32>>) {
        let mut buffer = self.audio_buffer.lock().unwrap();
        buffer[AudioChannel::Master.get_index()].extend_from_slice(samples[0].deref());
    }

    pub fn get_samples(&self, channel: AudioChannel, count: usize) -> Vec<f32> {
        let buffer = &self.audio_buffer.lock().unwrap()[channel.get_index()];
        buffer.range(..count).map(|x| *x).collect()
    }
}