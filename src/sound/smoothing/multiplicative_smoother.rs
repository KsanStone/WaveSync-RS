use crate::sound::smoothing::FloatArraySmoother;

pub struct MultiplicativeSmoother {
    factor: f32,
    current_state: Vec<f32>,
}

impl MultiplicativeSmoother {
    pub fn new(factor: f32) -> Self {
        Self {
            factor,
            current_state: vec![],
        }
    }
}

impl FloatArraySmoother for MultiplicativeSmoother {
    fn set_factor(&mut self, factor: f32) {
        self.factor = factor;
    }

    fn get_factor(&self) -> f32 {
        self.factor
    }

    fn smooth_data(&mut self, delta_t: f32, recent_data: &[f32]) -> &[f32] {
        if recent_data.len() != self.current_state.len() {
            self.current_state.resize(recent_data.len(), 0.0);
        }

        let f = 1.0 - self.factor.clamp(0.0, 1.0);
        let dt = (delta_t * 100.0).clamp(0.0, 1.0);

        for (i, item) in self.current_state.iter_mut().enumerate() {
            *item = (*item + (recent_data[i] - *item) * f * dt).clamp(0.0, 1.0);
        }

        &self.current_state
    }
}
