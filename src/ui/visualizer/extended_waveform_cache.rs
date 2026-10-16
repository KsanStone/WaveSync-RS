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

/// Fixed-capacity GPU ring bookkeeping. CPU retains only pending new columns,
/// never the cached history. Viewport width is deliberately not an input.
#[derive(Default)]
pub(super) struct ColumnCache {
    pub pending: Vec<[f32; 4]>,
    pub head: usize,
    pub len: usize,
    pub upload_start: usize,
    size: usize,
    chunk: usize,
    first_sample: i128,
    initialized: bool,
}
impl ColumnCache {
    pub fn invalidate(&mut self) {
        self.initialized = false;
    }
    pub fn chunk_size(&self) -> usize {
        self.chunk.max(1)
    }
    pub fn update(
        &mut self,
        capacity: usize,
        size: usize,
        written: u64,
        mut scan: impl FnMut(Range<usize>) -> Summary,
    ) -> bool {
        self.pending.clear();
        // Round bucket width up, then use all complete buckets. This avoids
        // leaving a large remainder of the audio history unused.
        let chunk = size.div_ceil(capacity + 1).max(1);
        let len = (size / chunk).saturating_sub(1).min(capacity);
        if len == 0 {
            self.len = 0;
            self.size = size;
            return false;
        }
        let skip = chunk - (written % chunk as u64) as usize;
        let first = written as i128 - size as i128 + skip as i128;
        let delta = first - self.first_sample;
        let rebuild = !self.initialized
            || self.len != len
            || self.size != size
            || delta < 0
            || delta % chunk as i128 != 0;
        let new_columns = if rebuild {
            len
        } else {
            (delta / chunk as i128).min(len as i128) as usize
        };
        if rebuild || new_columns == len {
            self.head = 0;
        } else {
            self.head = (self.head + new_columns) % len;
        }
        self.upload_start = (self.head + len - new_columns) % len;
        for column in len - new_columns..len {
            let start = column * chunk + skip;
            let summary = scan(start..start + chunk);
            self.pending
                .push([summary.min, summary.max, summary.rms * summary.rms, 0.0]);
        }
        self.len = len;
        self.size = size;
        self.chunk = chunk;
        self.first_sample = first;
        self.initialized = true;
        new_columns != 0
    }
}

/// Smooth audio sample position with bounded look-behind. Only small local
/// differences are sent as f32; long-running absolute positions stay in f64/u64.
#[derive(Default)]
pub(super) struct SampleScroll {
    cursor: Option<f64>,
    previous_written: u64,
}
impl SampleScroll {
    pub fn offset(
        &mut self,
        written: u64,
        chunk: usize,
        sample_rate: u32,
        batch: u64,
        dt: f64,
        guards: usize,
    ) -> f32 {
        let newest = written as f64;
        let max_lag = guards.saturating_sub(2) as f64 * chunk as f64;
        let delay = (2.0 * chunk as f64).max(batch as f64 * 2.0).min(max_lag);
        let target = newest - delay;
        let cursor = self.cursor.get_or_insert(target);
        if written < self.previous_written
            || dt > 0.25
            || (newest - *cursor).abs() > max_lag + batch as f64
        {
            *cursor = target;
        } else {
            let previous = *cursor;
            *cursor += dt.max(0.0) * sample_rate as f64;
            *cursor += (target - *cursor) * (1.0 - (-dt.max(0.0) * 2.0).exp());
            *cursor = cursor.max(previous);
        }
        *cursor = cursor.clamp(newest - max_lag, newest);
        self.previous_written = written;
        ((written % chunk as u64) as f64 / chunk as f64 + (*cursor - newest) / chunk as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_buffer::CircularBuffer;
    fn verify<const N: usize>(
        cache: &mut ColumnCache,
        gpu: &mut [[f32; 4]],
        buffer: &CircularBuffer<N, f32>,
        written: u64,
    ) -> usize {
        cache.update(gpu.len(), buffer.len(), written, |range| {
            Summary::scan(buffer.range(range))
        });
        for (i, value) in cache.pending.iter().enumerate() {
            gpu[(cache.upload_start + i) % cache.len] = *value;
        }
        let chunk = cache.chunk_size();
        let skip = chunk - (written % chunk as u64) as usize;
        for i in 0..cache.len {
            let values: Vec<_> = buffer
                .range(i * chunk + skip..(i + 1) * chunk + skip)
                .copied()
                .collect();
            let min = values.iter().copied().fold(f32::INFINITY, f32::min);
            let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let squares = values.iter().map(|x| x * x).sum::<f32>() / chunk as f32;
            let actual = gpu[(cache.head + i) % cache.len];
            assert_eq!(actual[0], min);
            assert_eq!(actual[1], max);
            assert!((actual[2] - squares).abs() < 1e-6);
        }
        cache.pending.len()
    }
    #[test]
    fn gpu_ring_matches_full_rescan_after_wraps_and_skipped_frames() {
        let mut buffer = CircularBuffer::<127, f32>::new();
        buffer.fill(0.0);
        let mut cache = ColumnCache::default();
        let mut gpu = [[0.0; 4]; 31];
        assert_eq!(verify(&mut cache, &mut gpu, &buffer, 0), 30);
        assert_eq!(verify(&mut cache, &mut gpu, &buffer, 0), 0);
        let mut written = 0;
        for frame in 0..400 {
            for _ in 0..(frame * 13 % 37 + 1) {
                written += 1;
                buffer.push_back((written as f32 * 0.09).sin());
            }
            verify(&mut cache, &mut gpu, &buffer, written);
            assert_eq!(verify(&mut cache, &mut gpu, &buffer, written), 0);
        }
        for _ in 0..300 {
            written += 1;
            buffer.push_back(0.5);
        }
        assert_eq!(verify(&mut cache, &mut gpu, &buffer, written), 30);
        cache.invalidate();
        assert_eq!(verify(&mut cache, &mut gpu, &buffer, written), 30);
    }
    #[test]
    fn only_new_columns_are_queued_and_counter_resets_rebuild() {
        let mut buffer = CircularBuffer::<64, f32>::new();
        buffer.fill(0.0);
        let mut cache = ColumnCache::default();
        let mut gpu = [[0.0; 4]; 7];
        verify(&mut cache, &mut gpu, &buffer, 0);
        for written in 1..=100 {
            buffer.push_back(written as f32 / 100.0);
            assert_eq!(
                verify(&mut cache, &mut gpu, &buffer, written),
                usize::from(written % 8 == 0)
            );
        }
        buffer.fill(0.0);
        assert_eq!(verify(&mut cache, &mut gpu, &buffer, 0), 7);
    }
    #[test]
    fn larger_audio_blocks_do_not_force_clock_jumps() {
        for batch in [256, 512, 1024, 2048] {
            let mut scroll = SampleScroll::default();
            let mut previous = None;
            for frame in 0..1000 {
                let written = (frame * 200 / batch * batch) as u64;
                let offset = scroll.offset(written, 47, 48000, batch as u64, 1.0 / 240.0, 256);
                let position = (written / 47) as f64 + offset as f64;
                if let Some(last) = previous {
                    let advance: f64 = (position - last) * 47.0;
                    assert!(
                        (150.0..=250.0).contains(&advance),
                        "clock jumped by {advance} samples for block {batch}"
                    );
                }
                previous = Some(position);
            }
        }
    }
    #[test]
    fn sample_clock_is_continuous_across_column_and_audio_block_boundaries() {
        let mut scroll = SampleScroll::default();
        let mut previous = -1000.0;
        for frame in 0..1000 {
            let written = ((frame * 200) / 256 * 256) as u64;
            let offset = scroll.offset(written, 47, 48000, 256, 1.0 / 240.0, 16);
            let position = (written / 47) as f64 + offset as f64;
            assert!(position >= previous - 1e-5);
            if frame > 0 {
                assert!(position - previous < 5.0);
            }
            assert!((-14.0..=1.0).contains(&offset));
            previous = position;
        }
    }
}
