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
