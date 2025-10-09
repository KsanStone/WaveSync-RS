use crate::sound::AudioBackendType;

/// Identifies the audio system.
/// And a corresponding backend that should handle it.
#[derive(Debug, Clone)]
pub struct AudioSystem {
    pub backend: AudioBackendType,
    pub name: String,
}