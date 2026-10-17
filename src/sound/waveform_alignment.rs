//! Waveform trigger strategies. Positions are local to a recent audio snapshot;
//! persistent phase is anchored to an observed crossing, never a global modulo.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignmentMode {
    Legacy,
    #[default]
    LocalTrigger,
}
impl AlignmentMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Legacy => "Legacy frequency lock",
            Self::LocalTrigger => "Local trigger",
        }
    }
}
pub struct Request {
    pub enabled: bool,
    pub mode: AlignmentMode,
    pub sample_rate: u32,
    pub frequency: Option<f32>,
    /// End position captured under the same lock as the audio samples.
    pub samples_written: u64,
    pub max_waves: usize,
}
#[derive(Debug)]
pub struct Selection {
    pub start: f64,
    pub len: usize,
}
impl Selection {
    fn full(samples: &[f32]) -> Self {
        Self {
            start: 0.0,
            len: samples.len(),
        }
    }
    /// Fractional triggers need fractional sample reads to avoid one-sample jitter.
    pub fn sample(&self, samples: &[f32], index: usize) -> f32 {
        let position = self.start + index as f64;
        let i = (position.floor() as usize).min(samples.len() - 1);
        let fraction = position.fract() as f32;
        let a = samples[i];
        if fraction == 0.0 {
            a
        } else {
            a + (samples[(i + 1).min(samples.len() - 1)] - a) * fraction
        }
    }
}
#[derive(Clone, Copy)]
struct Trigger {
    sample: u64,
    fraction: f64,
    period: f64,
}
#[derive(Default)]
pub struct WaveformAligner {
    trigger: Option<Trigger>,
    last_end: Option<u64>,
    mode: Option<AlignmentMode>,
    rate: u32,
}
impl WaveformAligner {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn select(&mut self, samples: &[f32], request: Request) -> Selection {
        profiling::scope!("waveform.alignment");
        if self.mode != Some(request.mode)
            || self.rate != request.sample_rate
            || self.last_end.is_some_and(|end| {
                request.samples_written < end
                    || request.samples_written - end > request.sample_rate as u64 * 2
            })
        {
            self.reset();
        }
        let previous_end = self.last_end;
        self.mode = Some(request.mode);
        self.rate = request.sample_rate;
        self.last_end = Some(request.samples_written);
        if !request.enabled || samples.len() < 12 || request.sample_rate == 0 {
            self.trigger = None;
            return Selection::full(samples);
        }
        let minimum_frequency = (1.2 * request.sample_rate as f64 / samples.len() as f64).max(1.0);
        let frequency = request
            .frequency
            .map(f64::from)
            .filter(|f| f.is_finite() && *f >= minimum_frequency && *f <= 20_000.0);
        match request.mode {
            AlignmentMode::Legacy => {
                profiling::scope!("waveform.alignment.legacy");
                let Some(frequency) = frequency else {
                    return Selection::full(samples);
                };
                // Preserve the old whole-Hz period and global-counter calculation.
                let period = (request.sample_rate as f32 / (frequency as f32).floor()) as f64;
                let drop = (period - request.samples_written as f64 % period)
                    .max(0.0)
                    .min((samples.len() - 10) as f64) as usize;
                let take = samples
                    .len()
                    .saturating_sub(period as usize)
                    .max(1)
                    .min((period as usize).saturating_mul(request.max_waves))
                    .min(samples.len() - drop);
                Selection {
                    start: drop as f64,
                    len: take,
                }
            }
            AlignmentMode::LocalTrigger => {
                profiling::scope!("waveform.alignment.local_trigger");
                self.local_trigger(samples, &request, frequency, previous_end)
            }
        }
    }
    fn local_trigger(
        &mut self,
        samples: &[f32],
        request: &Request,
        frequency: Option<f64>,
        previous_end: Option<u64>,
    ) -> Selection {
        let measured_period = frequency.map(|f| request.sample_rate as f64 / f);
        let period = match (measured_period, self.trigger) {
            (Some(measured), Some(previous)) => {
                if !(0.67..=1.5).contains(&(measured / previous.period)) {
                    // A genuinely different dominant frequency needs a fresh lock.
                    self.trigger = None;
                    measured
                } else {
                    let elapsed = request
                        .samples_written
                        .saturating_sub(previous_end.unwrap_or(request.samples_written))
                        as f64
                        / request.sample_rate as f64;
                    previous.period + (measured - previous.period) * (1.0 - (-elapsed / 0.08).exp())
                }
            }
            (Some(measured), None) => measured,
            (None, Some(previous)) => previous.period, // hold through weak FFT estimates
            (None, None) => return Selection::full(samples),
        };
        if !period.is_finite() || period < 2.0 || period > (samples.len() - 2) as f64 {
            return Selection::full(samples);
        }
        let take = ((period * request.max_waves.max(1) as f64).floor() as usize)
            .max(10)
            .min(samples.len().saturating_sub(period.ceil() as usize).max(10));
        let maximum_start = (samples.len() - take - 1) as f64;
        let window_start = request.samples_written.saturating_sub(samples.len() as u64);
        let predicted = if let Some(previous) = self.trigger {
            // Subtract integer positions BEFORE conversion: uptime cannot magnify
            // a fractional frequency change into an arbitrary phase jump.
            let relative =
                (previous.sample as i128 - window_start as i128) as f64 + previous.fraction;
            let cycles = (-relative / period).ceil();
            (relative + cycles * period).clamp(0.0, maximum_start)
        } else {
            (period * 0.5).min(maximum_start)
        };
        let radius = if self.trigger.is_some() {
            period * 0.35
        } else {
            period
        };
        let begin = (predicted - radius).max(0.0).floor() as usize;
        let end = (predicted + radius).min(maximum_start).ceil() as usize;
        let scan_begin = begin.saturating_sub((period * 0.5).ceil() as usize);
        let scan_end = (end + (period * 0.5).ceil() as usize + 2).min(samples.len());
        let region = &samples[scan_begin..scan_end];
        let rms = (region
            .iter()
            .filter(|v| v.is_finite())
            .map(|&v| (v as f64).powi(2))
            .sum::<f64>()
            / region.len().max(1) as f64)
            .sqrt();
        let hysteresis = (rms * 0.15).max(0.00001) as f32;
        let mut armed = false;
        let mut pending = None;
        let mut best: Option<(f64, f64)> = None;
        for i in scan_begin..scan_end {
            let value = samples[i];
            if !value.is_finite() {
                armed = false;
                pending = None;
                continue;
            }
            if value <= -hysteresis {
                armed = true;
                pending = None;
            }
            if armed && i > scan_begin && samples[i - 1] <= 0.0 && value > 0.0 {
                let a = samples[i - 1] as f64;
                pending = Some(i as f64 - 1.0 - a / (value as f64 - a));
            }
            if armed && value >= hysteresis {
                if let Some(position) = pending.take() {
                    if position >= begin as f64
                        && position <= end as f64
                        && position <= maximum_start
                    {
                        let distance = (position - predicted).abs();
                        if best.is_none_or(|(_, previous_distance)| distance < previous_distance) {
                            best = Some((position, distance));
                        }
                    }
                }
                armed = false;
            }
        }
        if let Some((start, _)) = best {
            self.trigger = Some(Trigger {
                sample: window_start + start.floor() as u64,
                fraction: start.fract(),
                period,
            });
            Selection { start, len: take }
        } else if self.trigger.is_some() {
            // No trustworthy crossing: coast on the previous lock, don't snap
            // to an unrelated peak or invent phase from a new global modulo.
            Selection {
                start: predicted,
                len: take,
            }
        } else {
            Selection::full(samples)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(end: u64) -> Vec<f32> {
        (end - 7200..end)
            .map(|s| {
                // Exact rational 30.5-Hz phase, including very large sample counters.
                (std::f64::consts::TAU * ((s % 96000) * 61) as f64 / 96000.0).sin() as f32
            })
            .collect()
    }
    fn request(mode: AlignmentMode, end: u64, frequency: Option<f32>) -> Request {
        Request {
            enabled: true,
            mode,
            sample_rate: 48000,
            frequency,
            samples_written: end,
            max_waves: 2,
        }
    }
    #[test]
    fn fractional_tone_locks_despite_estimator_jitter_and_long_uptime() {
        let mut aligner = WaveformAligner::default();
        for frame in 0..180 {
            let end = 9_000_000_000_000 + frame * 800;
            let samples = tone(end);
            let frequency = 30.5 + if frame % 2 == 0 { 0.25 } else { -0.25 };
            let selection = aligner.select(
                &samples,
                request(AlignmentMode::LocalTrigger, end, Some(frequency)),
            );
            assert!(selection.len < samples.len());
            assert!(selection.sample(&samples, 0).abs() < 0.0001);
            assert!(selection.sample(&samples, 1) > selection.sample(&samples, 0));
        }
    }
    #[test]
    fn legacy_preserves_whole_hz_modulo_and_fractional_tone_drift() {
        let mut aligner = WaveformAligner::default();
        let mut maximum: f32 = 0.0;
        for frame in 0..120 {
            let end = 96000 + frame * 800;
            let samples = tone(end);
            let selection =
                aligner.select(&samples, request(AlignmentMode::Legacy, end, Some(30.5)));
            let expected_drop = (1600.0 - end as f64 % 1600.0) as usize;
            assert_eq!(selection.start, expected_drop as f64);
            assert_eq!(selection.len, 3200);
            maximum = maximum.max(selection.sample(&samples, 0).abs());
        }
        assert!(maximum > 0.9);
    }
    #[test]
    fn weak_signal_coasts_and_disabling_or_switching_resets_lock() {
        let mut aligner = WaveformAligner::default();
        let end = 96000;
        aligner.select(
            &tone(end),
            request(AlignmentMode::LocalTrigger, end, Some(30.5)),
        );
        let anchor = aligner.trigger.unwrap().sample;
        let positive = vec![0.01; 7200];
        let held = aligner.select(
            &positive,
            request(AlignmentMode::LocalTrigger, end + 800, None),
        );
        assert_eq!(aligner.trigger.unwrap().sample, anchor);
        assert!(held.start >= 0.0 && held.start + (held.len as f64) < positive.len() as f64);
        let mut disabled = request(AlignmentMode::LocalTrigger, end + 1600, Some(30.5));
        disabled.enabled = false;
        assert_eq!(aligner.select(&positive, disabled).len, positive.len());
        assert!(aligner.trigger.is_none());
        aligner.select(
            &tone(end + 2400),
            request(AlignmentMode::LocalTrigger, end + 2400, Some(30.5)),
        );
        aligner.select(
            &positive,
            request(AlignmentMode::Legacy, end + 3200, Some(30.5)),
        );
        assert!(aligner.trigger.is_none());
    }
    #[test]
    fn invalid_estimates_and_short_or_silent_buffers_are_safe() {
        let mut aligner = WaveformAligner::default();
        for mode in [AlignmentMode::Legacy, AlignmentMode::LocalTrigger] {
            for frequency in [None, Some(f32::NAN), Some(0.0), Some(30.5)] {
                for len in [0, 1, 10, 7200] {
                    let samples = vec![0.0; len];
                    let selection = aligner.select(&samples, request(mode, 96000, frequency));
                    assert!(selection.start.is_finite());
                    assert!(selection.start + selection.len as f64 <= len as f64);
                }
            }
        }
    }
}
