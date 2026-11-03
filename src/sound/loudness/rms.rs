use crate::sound::loudness::LoudnessMeter;
use crate::sound::scale_to_db;

pub struct RmsLoudnessMeter {
    loudness: Vec<f32>,
}

impl Default for RmsLoudnessMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl RmsLoudnessMeter {
    pub fn new() -> Self {
        Self {
            loudness: Vec::new(),
        }
    }
}

impl LoudnessMeter for RmsLoudnessMeter {
    fn process_frame(&mut self, frame: &[Vec<f32>], _sample_rate: u32) {
        self.loudness.resize(frame.len(), 0.0);

        for (i, channel) in frame.iter().enumerate() {
            let rms_value = calc_rms(channel.iter(), channel.len());
            self.loudness[i] = scale_to_db(rms_value);
        }
    }

    fn get_loudness(&self) -> Vec<f32> {
        self.loudness.clone()
    }

    fn unit(&self) -> &'static str {
        "dB"
    }
}

pub fn calc_rms<'a, I>(channel: I, len: usize) -> f32
where
    I: Iterator<Item = &'a f32>,
{
    // Allow independent SIMD accumulators; audio RMS tolerates reassociation.
    let squared_sum = channel.fold(0.0_f32, |sum, &x| sum.algebraic_add(x.algebraic_mul(x)));

    (squared_sum / len as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::calc_rms;

    #[test]
    fn reassociated_rms_matches_double_precision_reference() {
        for len in [1, 32, 256, 1024, 3200, 32768] {
            for amplitude in [0.0, 1e-12, 0.01, 1.0] {
                let samples: Vec<f32> = (0..len)
                    .map(|i| (i as f32 * 0.13).sin() * amplitude)
                    .collect();
                let expected =
                    (samples.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / len as f64).sqrt();
                // Also accommodate scalar accumulation in unoptimized builds.
                let actual = calc_rms(samples.iter(), len) as f64;
                assert!((actual - expected).abs() <= 1e-5 * expected.max(1e-30));
                // Circular-buffer ranges can cross their backing allocation boundary.
                let split = len / 3;
                let chained = samples[..split].iter().chain(samples[split..].iter());
                assert!(
                    (calc_rms(chained, len) as f64 - expected).abs() <= 1e-5 * expected.max(1e-30)
                );
            }
        }
    }
}
