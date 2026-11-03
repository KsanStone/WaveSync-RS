use crate::ui::visualizer::spectrum::SmootherType;

pub mod exponential_falloff_smoother;
pub mod multiplicative_smoother;

/// Smoothly merges between consecutive array states
pub trait FloatArraySmoother: Send + Sync {
    /// The factor is in the range 0..=1
    fn set_factor(&mut self, factor: f32);

    #[allow(unused)]
    fn get_factor(&self) -> f32;

    /// Smooths the data
    /// delta_t - seconds since last frame
    /// recent_data - the target towards which the data is smoothed
    fn smooth_data(&mut self, delta_t: f32, recent_data: &[f32]) -> &[f32];

    fn get_type(&self) -> SmootherType;
}

#[cfg(test)]
mod tests {
    use super::{
        FloatArraySmoother, exponential_falloff_smoother::ExponentialFalloffSmoother,
        multiplicative_smoother::MultiplicativeSmoother,
    };

    #[test]
    fn optimized_smoothers_match_original_over_frames_and_resizes() {
        for factor in [0.0_f32, 0.7, 0.96, 1.0] {
            let mut multiplicative = MultiplicativeSmoother::new();
            let mut falloff = ExponentialFalloffSmoother::new();
            multiplicative.set_factor(factor);
            falloff.set_factor(factor);
            let (mut m, mut f, mut speed) =
                (Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new());
            for frame in 0..100 {
                let len = [65, 65, 16384, 32, 0][frame / 20];
                m.resize(len, 0.0);
                f.resize(len, 0.0);
                speed.resize(len, 0.0);
                let input: Vec<f32> = (0..len)
                    .map(|i| ((i + frame * 31) as f32 * 0.17).sin().abs())
                    .collect();
                let dt = [0.0_f32, 0.001, 1.0 / 144.0, 1.0 / 60.0, 1.0 / 30.0, 1.5][frame % 6];
                for i in 0..len {
                    m[i] = (m[i]
                        + (input[i] - m[i])
                            * (1.0 - factor.clamp(0.0, 1.0))
                            * (dt * 100.0).clamp(0.0, 1.0))
                    .clamp(0.0, 1.0);
                    if input[i] > f[i] {
                        f[i] = input[i];
                        speed[i] = 0.0;
                    } else {
                        f[i] = (f[i] - speed[i] * dt.clamp(0.0, 1.0) * f[i]).clamp(0.0, 1.0);
                        speed[i] += (factor * 30.0).clamp(0.0, 30.0) * dt.clamp(0.0, 1.0);
                    }
                }
                for (&actual, &expected) in multiplicative.smooth_data(dt, &input).iter().zip(&m) {
                    assert!((actual - expected).abs() < 2e-6);
                }
                assert_eq!(falloff.smooth_data(dt, &input), f);
            }
        }
    }
}
