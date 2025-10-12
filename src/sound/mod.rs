pub mod audio_backend;
pub mod audio_service;
pub mod audio_system;
pub mod capture_source;
pub mod cpal_audio_backend;
pub mod dummy_audio_backend;
pub mod windowing;

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

impl TryFrom<u32> for AudioChannel {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AudioChannel::Master),
            1 => Ok(AudioChannel::Left),
            2 => Ok(AudioChannel::Right),
            _ => Err(()),
        }
    }
}

impl AudioChannel {
    pub fn get_index(&self) -> usize {
        match self {
            AudioChannel::Master => 0,
            AudioChannel::Left => 1,
            AudioChannel::Right => 2,
        }
    }

    pub fn get_label(&self) -> &'static str {
        match self {
            AudioChannel::Master => "M",
            AudioChannel::Left => "L",
            AudioChannel::Right => "R",
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
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

#[derive(Clone, Copy, Debug)]
pub struct FftPeak {
    pub value: f32,
    pub index: usize,
    pub interpolated_frequency: f32,
}

impl FftPeak {
    pub fn new() -> Self {
        FftPeak {
            value: 0.0,
            index: 0,
            interpolated_frequency: 0.0,
        }
    }
}

/// Computes the audio frequency that the given bin is associated with
pub fn frequency_of_bin(bin: usize, rate: usize, fft_size: usize) -> f32 {
    (bin as f32) * (rate as f32 / fft_size as f32)
}

#[inline]
pub fn scale_to_db(value: f32) -> f32 {
    20.0 * value.log10()
}

/// Scale the magnitudes in accordance with the db scale
pub fn db_scale_magnitudes(magnitudes: &mut [f32]) {
    magnitudes.iter_mut().for_each(|m| *m = scale_to_db(*m));
}

/// Calculate the bin index that contains the frequency.
/// Rounds down.
pub fn bin_of_frequency(frequency: f32, rate: usize, fft_size: usize) -> usize {
    (frequency * fft_size as f32 / rate as f32).round() as usize
}

pub fn estimate_frequency_peak(magnitudes: &[f32], sample_rate: usize) -> Option<FftPeak> {
    if magnitudes.len() < 3 {
        return None;
    }
    let fft_size = magnitudes.len() * 2;
    let idx = magnitudes
        .iter()
        .enumerate()
        .skip(1)
        .take(magnitudes.len() - 2)
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i);
    if let Some(idx) = idx {
        if idx == magnitudes.len() - 1 || idx == 0 {
            return None;
        }

        let estimated_peak_frequency = calculate_parabola_peak(
            frequency_of_bin(idx - 1, sample_rate, fft_size),
            scale_to_db(magnitudes[idx - 1]),
            frequency_of_bin(idx, sample_rate, fft_size),
            scale_to_db(magnitudes[idx]),
            frequency_of_bin(idx + 1, sample_rate, fft_size),
            scale_to_db(magnitudes[idx + 1]),
        );

        return Some(FftPeak {
            value: magnitudes[idx],
            index: idx,
            interpolated_frequency: estimated_peak_frequency,
        });
    }
    None
}

/// Fits a parabola to the 3 points, and computes its peak x coordinate
pub fn calculate_parabola_peak(x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32) -> f32 {
    let denominator = (x1 - x2) * (x1 - x3) * (x2 - x3);
    let a = (x3 * (y2 - y1) + x2 * (y1 - y3) + x1 * (y3 - y2)) / denominator;
    let b = (x3 * x3 * (y1 - y2) + x2 * x2 * (y3 - y1) + x1 * x1 * (y2 - y3)) / denominator;

    -b / (2.0 * a)
}
