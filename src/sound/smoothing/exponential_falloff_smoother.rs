use crate::sound::smoothing::FloatArraySmoother;

pub struct ExponentialFalloffSmoother {
    factor: f32,
    current_state: Vec<f32>,
    falloff_speed: Vec<f32>,
}

impl ExponentialFalloffSmoother {
    pub fn new() -> Self {
        Self {
            factor: 0.96,
            current_state: vec![],
            falloff_speed: vec![],
        }
    }
}

impl FloatArraySmoother for ExponentialFalloffSmoother {
    fn set_factor(&mut self, factor: f32) {
        self.factor = factor;
    }

    fn get_factor(&self) -> f32 {
        self.factor
    }

    fn smooth_data(&mut self, delta_t: f32, recent_data: &[f32]) -> &[f32] {
        if recent_data.len() != self.current_state.len() {
            self.current_state.resize(recent_data.len(), 0.0);
            self.falloff_speed.resize(recent_data.len(), 0.0);
        }

        let f = (self.factor * 3.0).clamp(0.0, 3.0);
        let dt = delta_t.clamp(0.0, 1.0);

        for (i, item) in self.current_state.iter_mut().enumerate() {
            if recent_data[i] > *item {
                *item = recent_data[i];
                self.falloff_speed[i] = 0.0;
            } else {
                *item = (*item - self.falloff_speed[i] * dt).clamp(0.0, 1.0);
                self.falloff_speed[i] += f * dt;
            }
        }

        &self.current_state
    }
}
