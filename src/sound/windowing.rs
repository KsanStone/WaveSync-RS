use std::f32::consts::PI;

pub enum WindowMethod {
    Hamming,
}

pub struct FftWindow {
    window_cache: Vec<f32>,
    window_scalar: f32,
    window_method: WindowMethod,
}

impl FftWindow {
    pub fn new(window_method: WindowMethod) -> Self {
        Self {
            window_cache: vec![],
            window_scalar: 1.0,
            window_method,
        }
    }

    pub fn set_method(&mut self, window_method: WindowMethod) {
        self.window_method = window_method;
        self.window_cache.clear();
    }

    pub fn get_window(&mut self, size: usize) -> (f32, &Vec<f32>) {
        if self.window_cache.len() != size {
            self.window_cache.resize(size, 0.0);
            match self.window_method {
                WindowMethod::Hamming => {
                    let size_minus_one = size.saturating_sub(1) as f32;
                    for i in 0..size {
                        self.window_cache[i] =
                            0.54 - 0.46 * (PI * 2.0 * i as f32 / size_minus_one).cos()
                    }
                    self.window_scalar = 1.0 / self.window_cache.iter().sum::<f32>();
                }
            }
        }
        (self.window_scalar, &self.window_cache)
    }
}
