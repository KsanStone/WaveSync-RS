pub mod audio_backend;
pub mod audio_service;
pub mod audio_system;
pub mod capture_source;
pub mod cpal_audio_backend;
pub mod dummy_audio_backend;

#[derive(Debug, Clone, Copy)]
pub enum AudioBackendType {
    Dummy,
    Cpal,
}

#[derive(Debug, Clone, Copy)]
pub enum AudioChannel {
    Master,
    Left,
    Right,
}

impl AudioChannel {
    pub fn get_index(&self) -> usize {
        match self {
            AudioChannel::Master => 0,
            AudioChannel::Left => 1,
            AudioChannel::Right => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SampleFormat {
    U8,
    I16,
    I32,
    F32,
}

impl SampleFormat {
    pub fn get_size(&self) -> usize {
        match self {
            SampleFormat::U8 => 1,
            SampleFormat::I16 => 2,
            SampleFormat::I32 => 4,
            SampleFormat::F32 => 4,
        }
    }
}
