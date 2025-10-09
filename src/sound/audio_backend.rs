use crate::sound::audio_system::AudioSystem;
use crate::sound::capture_source::CaptureSource;

/// Interacts with the audio lib.
pub trait AudioBackend: Send + Sync {
    fn detect_supported_capture_sources(&self) -> Vec<CaptureSource>;

    fn detect_supported_audio_systems(&self) -> Vec<AudioSystem>;

    fn find_default_capture_source(&self) -> CaptureSource;

    fn set_current_audio_system(&mut self, system: AudioSystem);

    fn get_current_audio_system(&self) -> AudioSystem;

    fn set_frame_callback(&mut self, callback: Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>);

    fn start_capture(&mut self, source: CaptureSource);

    fn stop_capture(&mut self);
}
