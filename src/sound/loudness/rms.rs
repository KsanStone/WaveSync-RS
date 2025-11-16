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
    fn process_frame(&mut self, frame: &[Vec<f32>]) {
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
    let squared_sum = channel.map(|x| *x * *x).sum::<f32>();

    (squared_sum / len as f32).sqrt()
}
