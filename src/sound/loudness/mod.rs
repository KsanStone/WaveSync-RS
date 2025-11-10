pub mod rms;

pub trait LoudnessMeter: Send + Sync {
    fn process_frame(&mut self, frame: &[Vec<f32>]);

    fn get_loudness(&self) -> Vec<f32>;

    #[allow(unused)]
    fn unit(&self) -> &'static str;
}
