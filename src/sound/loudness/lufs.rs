use crate::sound::loudness::LoudnessMeter;
use ebur128::{EbuR128, Mode};

/// EBU R128 momentary loudness (400 ms) for every displayed channel.
pub struct LufsLoudnessMeter {
    meters: Vec<EbuR128>,
    sample_rate: u32,
    loudness: Vec<f32>,
}

impl Default for LufsLoudnessMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl LufsLoudnessMeter {
    pub fn new() -> Self {
        Self {
            meters: Vec::new(),
            sample_rate: 0,
            loudness: Vec::new(),
        }
    }

    fn reset(&mut self, channels: usize, sample_rate: u32) {
        self.meters = (0..channels)
            .filter_map(|_| EbuR128::new(1, sample_rate, Mode::M).ok())
            .collect();
        self.sample_rate = sample_rate;
        self.loudness.resize(channels, -70.0);
        self.loudness.fill(-70.0);
    }
}

impl LoudnessMeter for LufsLoudnessMeter {
    fn process_frame(&mut self, frame: &[Vec<f32>], sample_rate: u32) {
        if frame.is_empty() || sample_rate == 0 {
            return;
        }
        if self.sample_rate != sample_rate || self.meters.len() != frame.len() {
            self.reset(frame.len(), sample_rate);
        }

        for (meter, (value, channel)) in self
            .meters
            .iter_mut()
            .zip(self.loudness.iter_mut().zip(frame))
        {
            if meter.add_frames_f32(channel).is_ok() {
                *value = meter
                    .loudness_momentary()
                    .ok()
                    .filter(|value| value.is_finite())
                    .map(|value| value as f32)
                    .unwrap_or(-70.0)
                    .max(-70.0);
            }
        }
    }

    fn get_loudness(&self) -> Vec<f32> {
        self.loudness.clone()
    }

    fn unit(&self) -> &'static str {
        "LUFS"
    }
}

#[cfg(test)]
mod tests {
    use super::LufsLoudnessMeter;
    use crate::sound::loudness::LoudnessMeter;

    #[test]
    fn produces_a_finite_momentary_value_after_400ms() {
        let rate = 48_000;
        let signal: Vec<f32> = (0..rate / 2)
            .map(|index| {
                let phase = index as f32 * std::f32::consts::TAU * 1_000.0 / rate as f32;
                phase.sin() * 0.1
            })
            .collect();
        let mut meter = LufsLoudnessMeter::new();

        meter.process_frame(&[signal], rate);

        let value = meter.get_loudness()[0];
        assert!(value.is_finite());
        assert!((-70.0..0.0).contains(&value));
    }
}
