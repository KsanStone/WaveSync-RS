use crate::sound::peaking::PeakComputer;

pub struct HoldSmoothFalloffPeakComputer {
    hold_time_secs: f32,
    decay_factor: f32,

    /// Current peak value for each channel
    current_state: Vec<f32>,
    /// Array containing the time each value has been held for, in seconds
    /// this will be incremented by delta_t each frame, and reset to 0 when a new peak is detected
    held_for: Vec<f32>,
}

impl Default for HoldSmoothFalloffPeakComputer {
    fn default() -> Self {
        Self {
            hold_time_secs: 1.0,
            decay_factor: 15.0,
            current_state: Vec::new(),
            held_for: Vec::new(),
        }
    }
}

impl HoldSmoothFalloffPeakComputer {
    pub fn reset(&mut self) {
        self.current_state.clear();
        self.held_for.clear();
    }
}

impl PeakComputer for HoldSmoothFalloffPeakComputer {
    fn set_hold_time(&mut self, hold_time_secs: f32) {
        self.hold_time_secs = hold_time_secs.max(0.0);
    }

    fn get_hold_time(&self) -> f32 {
        self.hold_time_secs
    }

    fn set_decay_factor(&mut self, factor: f32) {
        self.decay_factor = factor.max(0.0);
    }

    fn get_decay_factor(&self) -> f32 {
        self.decay_factor
    }

    fn compute_peaks(&mut self, delta_t: f32, recent_data: &[f32]) -> &[f32] {
        if recent_data.len() != self.current_state.len() {
            self.current_state.clear();
            self.current_state.extend_from_slice(recent_data);
            self.held_for.clear();
            self.held_for.resize(recent_data.len(), 0.0);
            return &self.current_state;
        }

        let dt = if delta_t.is_finite() {
            delta_t.max(0.0)
        } else {
            0.0
        };
        for (i, &recent_value) in recent_data.iter().enumerate() {
            if recent_value >= self.current_state[i] {
                self.current_state[i] = recent_value;
                self.held_for[i] = 0.0;
            } else {
                let remaining_hold = (self.hold_time_secs - self.held_for[i]).max(0.0);
                let decay_time = (dt - remaining_hold).max(0.0);
                self.held_for[i] += dt;
                self.current_state[i] = (self.current_state[i] - self.decay_factor * decay_time)
                    .max(recent_value)
                    .max(-1000.0);
            }
        }

        &self.current_state
    }

    fn get_type(&self) -> super::PeakComputerType {
        super::PeakComputerType::HoldSmoothFalloff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_db_initialization_hold_and_partial_frame_decay() {
        let mut peaks = HoldSmoothFalloffPeakComputer::default();
        peaks.set_hold_time(1.0);
        peaks.set_decay_factor(10.0);
        assert_eq!(peaks.compute_peaks(0.0, &[-20.0]), &[-20.0]);
        assert_eq!(peaks.compute_peaks(0.75, &[-80.0]), &[-20.0]);
        assert_eq!(peaks.compute_peaks(0.5, &[-80.0]), &[-22.5]);
        assert_eq!(peaks.compute_peaks(0.1, &[-10.0]), &[-10.0]);
        assert_eq!(peaks.compute_peaks(0.5, &[-80.0]), &[-10.0]);
    }

    #[test]
    fn decay_is_frame_rate_independent_and_does_not_fall_below_signal() {
        let run = |frames: usize| {
            let mut peaks = HoldSmoothFalloffPeakComputer::default();
            peaks.set_hold_time(0.5);
            peaks.set_decay_factor(10.0);
            peaks.compute_peaks(0.0, &[-10.0]);
            for _ in 0..frames {
                peaks.compute_peaks(2.0 / frames as f32, &[-80.0]);
            }
            peaks.compute_peaks(0.0, &[-80.0])[0]
        };
        assert!((run(60) - run(240)).abs() < 0.001);
        assert!((run(60) + 25.0).abs() < 0.001);
        let mut peaks = HoldSmoothFalloffPeakComputer::default();
        peaks.set_hold_time(0.0);
        peaks.compute_peaks(0.0, &[-10.0]);
        assert_eq!(peaks.compute_peaks(100.0, &[-15.0]), &[-15.0]);
    }

    #[test]
    fn resize_reset_and_zero_decay() {
        let mut peaks = HoldSmoothFalloffPeakComputer::default();
        peaks.set_decay_factor(0.0);
        peaks.compute_peaks(0.0, &[-10.0]);
        assert_eq!(peaks.compute_peaks(100.0, &[-50.0]), &[-10.0]);
        assert_eq!(peaks.compute_peaks(0.0, &[-30.0, -40.0]), &[-30.0, -40.0]);
        peaks.reset();
        assert_eq!(peaks.compute_peaks(0.0, &[-70.0, -80.0]), &[-70.0, -80.0]);
    }
}
