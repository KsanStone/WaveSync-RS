pub mod hold_smooth_falloff;

pub enum PeakComputerType {
    HoldSmoothFalloff,
}

pub trait PeakComputer: Send + Sync {
    /// Sets the hold time in seconds for the peak value
    fn set_hold_time(&mut self, hold_time_secs: f32);
    /// Gets the hold time in seconds for the peak value
    fn get_hold_time(&self) -> f32;

    /// How quickly the peak value decays in dB per second after the hold time has elapsed
    fn set_decay_factor(&mut self, factor: f32);
    /// Gets the decay factor for the peak value after the hold time has elapsed
    fn get_decay_factor(&self) -> f32;

    /// Applies the peak computation,
    /// delta_t - seconds since last frame
    /// recent_data - current state to get the peak from.
    fn compute_peaks(&mut self, delta_t: f32, recent_data: &[f32]) -> &[f32];

    fn get_type(&self) -> PeakComputerType;
}
/// Shared persisted controls. Decay is normalized in the UI and converted to dB/s.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PeakSettings {
    pub enabled: bool,
    pub hold_secs: f32,
    pub decay: f32,
}

impl Default for PeakSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            hold_secs: 1.0,
            decay: 0.25,
        }
    }
}

impl PeakSettings {
    pub fn draw_ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.enabled, "Show peaks");
        ui.add_enabled_ui(self.enabled, |ui| {
            ui.add(
                egui::Slider::new(&mut self.hold_secs, 0.0..=10.0)
                    .text("Peak hold")
                    .suffix(" s"),
            );
            ui.add(egui::Slider::new(&mut self.decay, 0.0..=1.0).text("Peak decay"))
                .on_hover_text("0: no decay; 1: 60 dB/s after the hold time");
        });
    }

    pub fn configure(&self, computer: &mut hold_smooth_falloff::HoldSmoothFalloffPeakComputer) {
        computer.set_hold_time(self.hold_secs.clamp(0.0, 10.0));
        computer.set_decay_factor(self.decay.clamp(0.0, 1.0) * 60.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_visualizer_settings_load_and_peak_controls_round_trip() {
        let spectrum: crate::ui::visualizer::spectrum::SpectrumVisualizerSettings = ron::from_str(
            "(draw_type:Line,smoother_type:Multiplicative,smoother_factor:0.7,frequency_axis_logarithmic:true,smooth_line:true,freq_min:20,freq_max:24000)"
        ).unwrap();
        let stereo: crate::ui::visualizer::stereo_imager::StereoImagerVisualizerSettings =
            ron::from_str("(smoothing_factor:0.8)").unwrap();
        assert!(!spectrum.peaks.enabled);
        assert!(!stereo.peaks.enabled);
        let settings = PeakSettings {
            enabled: true,
            hold_secs: 2.5,
            decay: 0.7,
        };
        let restored: PeakSettings = ron::from_str(&ron::to_string(&settings).unwrap()).unwrap();
        assert!(restored.enabled);
        assert_eq!(restored.hold_secs, 2.5);
        let mut computer = hold_smooth_falloff::HoldSmoothFalloffPeakComputer::default();
        restored.configure(&mut computer);
        assert_eq!(computer.get_hold_time(), 2.5);
        assert_eq!(computer.get_decay_factor(), 42.0);
    }
}
