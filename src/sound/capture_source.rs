use crate::sound::{bin_of_frequency, frequency_of_bin, AudioBackendType, SampleFormat};

/// Represents a capture source.
/// Is linked to a specific audio backend.
///
/// This is meant to be used as:
/// - A metadata store to access info about the device quickly to adjust visualizers
/// - A unique identifier of the actual capture source within the audio backend.
///
/// The audio backend should keep its source handles within-itself, and look them up based on this
/// id by itself.
#[derive(Debug, Clone)]
pub struct CaptureSource {
    /// The name of the capture source.
    /// This is displayed in the ui.
    pub name: String,
    /// The unique identifier of the capture source.
    /// To be used internally by the audio backend.
    pub id: String,
    pub channels: u32,
    pub sample_rate: u32,
    pub format: SampleFormat,
    /// Whether the capture source is a loopback device, or a separate device, such as a microphone.
    pub is_loopback: bool,
    /// The audio backend that this source is linked to.
    /// Using it on any other backend should panic.
    pub backend: AudioBackendType,
}

impl Default for CaptureSource {
    fn default() -> Self {
        CaptureSource {
            name: "Default".to_string(),
            id: "".to_string(),
            channels: 1,
            sample_rate: 48000,
            format: SampleFormat::F32,
            is_loopback: true,
            backend: AudioBackendType::Dummy,
        }
    }
}

impl CaptureSource {
    /// Calculate how many bins we need to skip so that the 1st bin is at the requested frequency.
    pub fn calculate_buffer_beginning_skip_for(&self, frequency: f32, fft_size: usize) -> usize {
        let factor = self.sample_rate as f32 / fft_size as f32;
        let skip = (frequency / factor).ceil() as usize;
        skip.min(fft_size / 2)
    }
    
    pub fn bin_of_frequency(&self, frequency: f32, fft_size: usize) -> usize {
        bin_of_frequency(frequency, self.sample_rate as usize, fft_size)
    }
}