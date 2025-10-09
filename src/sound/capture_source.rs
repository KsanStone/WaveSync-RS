use crate::sound::{AudioBackendType, SampleFormat};

/// Represents a capture source.
/// Is linked to a specific audio backend.
#[derive(Debug, Clone)]
pub struct CaptureSource {
    pub name: String,
    pub id: String,
    pub channels: u32,
    pub sample_rate: u32,
    pub format: SampleFormat,
    pub backend: AudioBackendType
}
