use crate::sound::audio_system::AudioSystem;
use crate::sound::capture_source::CaptureSource;
use std::any::Any;
use std::sync::{Arc, Mutex};

pub type OptionCaptureCallback = Option<Arc<Mutex<Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>>>>;

/// The AudioBackend trait is used to abstract away the actual audio backend.
/// Each backend can potentially handle many audio systems, (such as ASIO and WASAPI on windows)
pub trait AudioBackend: Send + Sync + Any {
    fn detect_supported_capture_sources(&self) -> Vec<CaptureSource>;

    fn detect_supported_audio_systems(&self) -> Vec<AudioSystem>;

    fn find_default_capture_source(&self) -> CaptureSource;

    /// Sets the current audio system.
    /// The sources should come from this AudioSystem
    fn set_current_audio_system(&mut self, system: AudioSystem);

    /// Return the AudioSystem currently in use.
    fn get_current_audio_system(&self) -> AudioSystem;

    fn set_frame_callback(&mut self, callback: Box<dyn FnMut(Vec<Vec<f32>>) + Send + Sync>);

    fn start_capture(&mut self, source: CaptureSource);

    fn stop_capture(&mut self);

    fn as_any(&self) -> &dyn Any;
    
}
