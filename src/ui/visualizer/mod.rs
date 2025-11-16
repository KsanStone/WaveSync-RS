pub mod extended_waveform;
pub mod spectrogram;
pub mod spectrum;
pub mod vectorscope;
pub mod visualizer_widget;
pub mod waveform;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum VisualizerType {
    Spectrogram,
    Spectrum,
    Vectorscope,
    Waveform,
}
