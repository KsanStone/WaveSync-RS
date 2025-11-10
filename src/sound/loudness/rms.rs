use crate::sound::loudness::LoudnessMeter;

pub struct RmsLoudnessMeter {
    loudness: Vec<f32>,
}

impl RmsLoudnessMeter {
    pub fn new() -> Self {
        Self {
            loudness: Vec::new(),
        }
    }
}

impl LoudnessMeter for RmsLoudnessMeter {
    fn process_frame(&mut self, frame: &[Vec<f32>]) {
        self.loudness.resize(frame.len(), 0.0);

        for (i, channel) in frame.iter().enumerate() {
            let squared_sum = channel.iter().map(|x| *x * *x).sum::<f32>();
            let rms_value = (squared_sum / channel.len() as f32).sqrt();
            self.loudness[i] = 20.0 * rms_value.log10();
        }
    }

    fn get_loudness(&self) -> Vec<f32> {
        self.loudness.clone()
    }

    fn unit(&self) -> &'static str {
        "dB"
    }
}
