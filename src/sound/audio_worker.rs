//! Bounded capture handoff. No DSP, blocking sends, allocation or logging in submit.
use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const QUEUE_BLOCKS: usize = 3;
const MAX_QUEUE_AGE: Duration = Duration::from_millis(20);

struct Block {
    samples: Vec<Vec<f32>>,
    captured: Instant,
    position: u64,
}

#[derive(Default)]
struct State {
    stopped: AtomicBool,
    captured_samples: AtomicU64,
    queued_samples: AtomicUsize,
    dropped_blocks: AtomicU64,
    dropped_samples: AtomicU64,
}

pub(super) struct AudioSubmitter {
    ready: Sender<Block>,
    free: Receiver<Block>,
    recycle: Sender<Block>,
    state: Arc<State>,
    sample_budget: usize,
    capacity: usize,
    channels: usize,
}

impl AudioSubmitter {
    /// Copies borrowed capture data into a preallocated block, or drops immediately.
    pub fn submit(&self, samples: &[Vec<f32>]) {
        if self.state.stopped.load(Ordering::Acquire) {
            return;
        }
        let count = samples.first().map_or(0, Vec::len);
        if count == 0 {
            return;
        }
        let position = self
            .state
            .captured_samples
            .fetch_add(count as u64, Ordering::Relaxed);
        if samples.len() != self.channels
            || count > self.capacity
            || samples.iter().any(|channel| channel.len() != count)
        {
            self.drop_block(count);
            return;
        }
        // Permit one callback larger than the budget when the queue is empty.
        // Otherwise this bounds backlog by audio duration, independent of block size.
        #[allow(deprecated)] // Keep Rust 1.98 support; renamed try_update in 1.99.
        let reservation = self.state.queued_samples.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |pending| {
                (pending == 0 || pending.saturating_add(count) <= self.sample_budget)
                    .then_some(pending + count)
            },
        );
        if reservation.is_err() {
            self.drop_block(count);
            return;
        }
        let Ok(mut block) = self.free.try_recv() else {
            self.state.queued_samples.fetch_sub(count, Ordering::AcqRel);
            self.drop_block(count);
            return;
        };
        block.captured = Instant::now();
        block.position = position;
        for (dest, src) in block.samples.iter_mut().zip(samples) {
            dest.clear();
            dest.extend_from_slice(src);
        }
        if let Err(error) = self.ready.try_send(block) {
            self.state.queued_samples.fetch_sub(count, Ordering::AcqRel);
            let _ = self.recycle.try_send(error.into_inner());
            self.drop_block(count);
        }
    }

    fn drop_block(&self, count: usize) {
        self.state.dropped_blocks.fetch_add(1, Ordering::Relaxed);
        self.state
            .dropped_samples
            .fetch_add(count as u64, Ordering::Relaxed);
    }
}

pub(super) struct AudioWorker {
    state: Arc<State>,
    thread: Option<JoinHandle<()>>,
}

impl AudioWorker {
    pub fn start(
        channels: usize,
        sample_rate: u32,
        process: impl FnMut(&[Vec<f32>], u64) + Send + 'static,
    ) -> (AudioSubmitter, Self) {
        Self::start_with_age(channels, sample_rate, MAX_QUEUE_AGE, process)
    }

    fn start_with_age(
        channels: usize,
        sample_rate: u32,
        max_age: Duration,
        mut process: impl FnMut(&[Vec<f32>], u64) + Send + 'static,
    ) -> (AudioSubmitter, Self) {
        assert!(channels > 0 && sample_rate > 0);
        let (ready_tx, ready_rx) = bounded::<Block>(QUEUE_BLOCKS);
        let (free_tx, free_rx) = bounded(QUEUE_BLOCKS + 1);
        // Reserve up to 100 ms per callback, outside the capture thread. Typical
        // callbacks are much smaller. Oversized blocks are dropped, never resized.
        let capacity = (sample_rate as usize / 10).max(1);
        for _ in 0..=QUEUE_BLOCKS {
            free_tx
                .send(Block {
                    samples: (0..channels)
                        .map(|_| Vec::with_capacity(capacity))
                        .collect(),
                    captured: Instant::now(),
                    position: 0,
                })
                .unwrap();
        }
        let state = Arc::new(State::default());
        let worker_state = state.clone();
        let recycle = free_tx.clone();
        let thread = thread::Builder::new().name("audio-process".into()).spawn(move || {
            let mut last_warning = Instant::now() - Duration::from_secs(1);
            let mut reported = 0;
            let mut next_sample = 0;
            while !worker_state.stopped.load(Ordering::Acquire) {
                match ready_rx.recv_timeout(Duration::from_millis(25)) {
                    Ok(block) => {
                        let count = block.samples[0].len();
                        worker_state.queued_samples.fetch_sub(count, Ordering::AcqRel);
                        if worker_state.stopped.load(Ordering::Acquire) { break; }
                        if block.captured.elapsed() > max_age {
                            worker_state.dropped_blocks.fetch_add(1, Ordering::Relaxed);
                            worker_state.dropped_samples.fetch_add(count as u64, Ordering::Relaxed);
                        } else {
                            let gap = block.position.saturating_sub(next_sample);
                            process(&block.samples, gap);
                            next_sample = block.position + count as u64;
                        }
                        let _ = free_tx.try_send(block);
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                }
                let dropped = worker_state.dropped_blocks.load(Ordering::Relaxed);
                if dropped != reported && last_warning.elapsed() >= Duration::from_secs(1) {
                    log::warn!("Audio pipeline overloaded: dropped {} capture blocks ({} total, {} samples/channel total); queue full, stale or oversized audio block",
                        dropped - reported, dropped, worker_state.dropped_samples.load(Ordering::Relaxed));
                    reported = dropped;
                    last_warning = Instant::now();
                }
            }
        }).expect("failed to start audio processing worker");
        (
            AudioSubmitter {
                ready: ready_tx,
                free: free_rx,
                recycle,
                state: state.clone(),
                sample_budget: (sample_rate as usize / 50).max(1),
                capacity,
                channels,
            },
            Self {
                state,
                thread: Some(thread),
            },
        )
    }
}

impl Drop for AudioWorker {
    fn drop(&mut self) {
        self.state.stopped.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            // The worker briefly upgrades a Weak<AudioService> while processing.
            // Its last release can destroy the service on this very thread.
            if handle.thread().id() != thread::current().id() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TIMEOUT: Duration = Duration::from_secs(2);

    #[test]
    fn blocked_processing_drops_instead_of_blocking_capture() {
        let (entered_tx, entered_rx) = bounded(1);
        let (release_tx, release_rx) = bounded::<()>(1);
        let (submit, worker) = AudioWorker::start_with_age(2, 192000, TIMEOUT, move |_, _| {
            let _ = entered_tx.try_send(());
            let _ = release_rx.recv_timeout(TIMEOUT);
        });
        let samples = vec![vec![0.5; 1920]; 2]; // 10 ms stereo callbacks
        submit.submit(&samples);
        entered_rx.recv_timeout(TIMEOUT).unwrap();
        let state = submit.state.clone();
        let (done_tx, done_rx) = bounded(1);
        let capture = thread::spawn(move || {
            // The processing thread remains blocked for all these submissions.
            for _ in 0..100 {
                submit.submit(&samples);
            }
            done_tx.send(submit).unwrap();
        });
        let result = done_rx.recv_timeout(TIMEOUT);
        release_tx.send(()).unwrap();
        drop(release_tx);
        let submit = result.expect("capture waited on the blocked processing worker");
        capture.join().unwrap();
        assert_eq!(state.dropped_blocks.load(Ordering::Relaxed), 98);
        assert!(state.queued_samples.load(Ordering::Acquire) <= 3840);
        drop(worker);
        let before = state.captured_samples.load(Ordering::Relaxed);
        submit.submit(&vec![vec![0.0; 10]; 2]);
        assert_eq!(state.captured_samples.load(Ordering::Relaxed), before);
    }

    #[test]
    fn recycled_blocks_preserve_stereo_data_and_run_on_worker() {
        let capture_thread = thread::current().id();
        let (done_tx, done_rx) = bounded(4);
        let (submit, worker) = AudioWorker::start_with_age(2, 192000, TIMEOUT, move |data, gap| {
            done_tx
                .send((
                    data[0][0],
                    data[1][0],
                    data[0].len(),
                    gap,
                    data[0].as_ptr() as usize,
                    thread::current().id(),
                ))
                .unwrap();
        });
        let mut pointers = std::collections::HashSet::new();
        for frame in 0..20 {
            let samples = vec![vec![frame as f32; 256], vec![-(frame as f32); 256]];
            submit.submit(&samples);
            let (l, r, n, gap, pointer, worker_thread) = done_rx.recv_timeout(TIMEOUT).unwrap();
            assert_eq!((l, r, n, gap), (frame as f32, -(frame as f32), 256, 0));
            assert_ne!(worker_thread, capture_thread);
            pointers.insert(pointer);
        }
        assert!(pointers.len() <= QUEUE_BLOCKS + 1);
        assert_eq!(submit.state.dropped_blocks.load(Ordering::Relaxed), 0);
        drop(worker);
    }

    #[test]
    fn stale_audio_is_discarded_and_next_block_reports_the_gap() {
        let (done_tx, done_rx) = bounded(4);
        let (release_tx, release_rx) = bounded::<()>(1);
        let (submit, worker) = AudioWorker::start_with_age(1, 192000, TIMEOUT, move |data, gap| {
            done_tx.send((data[0][0], gap)).unwrap();
            if data[0][0] == 1.0 {
                let _ = release_rx.recv_timeout(TIMEOUT);
            }
        });
        submit.submit(&[vec![1.0; 256]]);
        assert_eq!(done_rx.recv_timeout(TIMEOUT).unwrap(), (1.0, 0));
        // Inject a deliberately old block: no wall-clock sleeps in the test.
        let mut stale = submit.free.try_recv().unwrap();
        stale.samples[0].extend_from_slice(&[2.0; 256]);
        stale.position = 256;
        stale.captured = Instant::now() - Duration::from_secs(3);
        submit.state.captured_samples.store(512, Ordering::Relaxed);
        submit.state.queued_samples.fetch_add(256, Ordering::AcqRel);
        submit.ready.try_send(stale).unwrap();
        submit.submit(&[vec![3.0; 256]]);
        release_tx.send(()).unwrap();
        assert_eq!(done_rx.recv_timeout(TIMEOUT).unwrap(), (3.0, 256));
        assert_eq!(submit.state.dropped_blocks.load(Ordering::Relaxed), 1);
        drop(worker);
    }

    #[test]
    fn malformed_and_oversized_blocks_are_dropped_without_resizing() {
        let (submit, worker) =
            AudioWorker::start(2, 192000, |_, _| panic!("invalid input reached DSP"));
        submit.submit(&vec![vec![0.0; 19201]; 2]);
        submit.submit(&[vec![0.0; 256], vec![0.0; 255]]);
        submit.submit(&[vec![0.0; 256]]);
        submit.submit(&[]);
        assert_eq!(submit.state.dropped_blocks.load(Ordering::Relaxed), 3);
        assert_eq!(submit.state.queued_samples.load(Ordering::Acquire), 0);
        drop(worker);
    }
}
