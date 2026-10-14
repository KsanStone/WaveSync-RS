use std::collections::VecDeque;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Summary {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}
impl Summary {
    pub fn scan<'a>(samples: impl Iterator<Item = &'a f32>) -> Self {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        let mut sum_squares = 0.0;
        let mut count = 0;
        for &sample in samples {
            min = min.min(sample);
            max = max.max(sample);
            sum_squares += sample * sample;
            count += 1;
        }
        if count == 0 {
            return Self::default();
        }
        Self {
            min,
            max,
            rms: (sum_squares / count as f32).sqrt(),
        }
    }
}

/// Cached columns refer to absolute sample ranges, including negative positions
/// for the zero-filled startup history. The original column alignment is kept.
#[derive(Default)]
pub(super) struct ColumnCache {
    pub columns: VecDeque<Summary>,
    width: usize,
    size: usize,
    first_sample: i128,
}
impl ColumnCache {
    pub fn update(
        &mut self,
        width: usize,
        size: usize,
        written: u64,
        mut scan: impl FnMut(Range<usize>) -> Summary,
    ) -> bool {
        let width = width.min(size.saturating_sub(1));
        if width == 0 {
            let changed = !self.columns.is_empty();
            self.columns.clear();
            self.width = 0;
            self.size = size;
            return changed;
        }
        let chunk = size / (width + 1);
        let skip = chunk - (written % chunk as u64) as usize;
        let first = written as i128 - size as i128 + skip as i128;
        let delta = first - self.first_sample;
        if self.width != width || self.size != size || delta < 0 || delta % chunk as i128 != 0 {
            self.columns.clear();
        } else {
            let shift = (delta / chunk as i128).min(width as i128) as usize;
            self.columns.drain(..shift.min(self.columns.len()));
        }
        let changed = self.columns.len() != width;
        for column in self.columns.len()..width {
            let start = column * chunk + skip;
            self.columns.push_back(scan(start..start + chunk));
        }
        self.width = width;
        self.size = size;
        self.first_sample = first;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_buffer::CircularBuffer;
    fn verify<const N: usize>(
        cache: &mut ColumnCache,
        buffer: &CircularBuffer<N, f32>,
        width: usize,
        written: u64,
    ) -> usize {
        let mut scans = 0;
        cache.update(width, buffer.len(), written, |range| {
            scans += 1;
            Summary::scan(buffer.range(range))
        });
        let width = width.min(buffer.len().saturating_sub(1));
        if width > 0 {
            let chunk = buffer.len() / (width + 1);
            let skip = chunk - (written % chunk as u64) as usize;
            for (i, result) in cache.columns.iter().enumerate() {
                let samples: Vec<_> = buffer
                    .range(i * chunk + skip..(i + 1) * chunk + skip)
                    .copied()
                    .collect();
                let expected_min = samples.iter().copied().fold(f32::INFINITY, f32::min);
                let expected_max = samples.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let expected_rms =
                    (samples.iter().map(|x| x * x).sum::<f32>() / chunk as f32).sqrt();
                assert_eq!(result.min, expected_min);
                assert_eq!(result.max, expected_max);
                assert!((result.rms - expected_rms).abs() < 1e-6);
            }
        }
        scans
    }
    #[test]
    fn cached_columns_match_full_scan_across_wraps_and_subcolumn_updates() {
        let mut buffer = CircularBuffer::<64, f32>::new();
        buffer.fill(0.0);
        let mut cache = ColumnCache::default();
        assert_eq!(verify(&mut cache, &buffer, 7, 0), 7);
        assert_eq!(verify(&mut cache, &buffer, 7, 0), 0);
        for written in 1..=500 {
            buffer.push_back((written as f32 * 0.17).sin());
            let scans = verify(&mut cache, &buffer, 7, written);
            assert_eq!(scans, if written % 8 == 0 { 1 } else { 0 });
        }
    }
    #[test]
    fn resize_skipped_frames_and_counter_reset_rebuild_correctly() {
        let mut buffer = CircularBuffer::<64, f32>::new();
        buffer.fill(0.0);
        let mut cache = ColumnCache::default();
        verify(&mut cache, &buffer, 7, 0);
        assert_eq!(verify(&mut cache, &buffer, 8, 0), 8);
        for i in 0..200 {
            buffer.push_back(i as f32 / 200.0);
        }
        assert_eq!(verify(&mut cache, &buffer, 8, 200), 8);
        buffer.fill(0.0);
        assert_eq!(verify(&mut cache, &buffer, 8, 0), 8);
    }
    #[test]
    fn irregular_updates_and_non_divisible_columns_match_full_rescans() {
        let mut buffer = CircularBuffer::<127, f32>::new();
        buffer.fill(0.0);
        let mut cache = ColumnCache::default();
        let mut written = 0;
        for frame in 0..200 {
            let delta = (frame * 13 % 37) + 1;
            for _ in 0..delta {
                written += 1;
                buffer.push_back((written as f32 * 0.09).cos());
            }
            let width = [9, 14, 31][(frame / 20) % 3];
            verify(&mut cache, &buffer, width, written);
            assert_eq!(verify(&mut cache, &buffer, width, written), 0);
        }
    }

    #[test]
    fn empty_and_narrow_history_do_not_divide_by_zero() {
        let mut buffer = CircularBuffer::<16, f32>::new();
        let mut cache = ColumnCache::default();
        verify(&mut cache, &buffer, 100, 0);
        buffer.push_back(1.0);
        verify(&mut cache, &buffer, 100, 1);
        buffer.push_back(-1.0);
        assert_eq!(verify(&mut cache, &buffer, 100, 2), 1);
        verify(&mut cache, &buffer, 0, 2);
    }
}
