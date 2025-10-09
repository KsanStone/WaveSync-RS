use crate::sound::AudioBackendType;

/// Identifies the audio system.
/// And a corresponding backend that should handle it.
///
/// Meant to be used as an identifier that the audio backend can use to
/// find the correct backend handle.
#[derive(Debug, Clone)]
pub struct AudioSystem {
    pub backend: AudioBackendType,
    pub name: String,
}
