use egui_wgpu::wgpu;
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

const WINDOW: usize = 600;
#[derive(Default)]
struct Samples(VecDeque<f64>);
impl Samples {
    fn push(&mut self, value: f64) {
        if self.0.len() == WINDOW {
            self.0.pop_front();
        }
        self.0.push_back(value);
    }
    fn stats(&self) -> Option<[f64; 3]> {
        self.percentiles(0.95, 0.99)
    }
    fn fps_stats(&self) -> Option<[f64; 3]> {
        self.percentiles(0.05, 0.01)
    }
    fn percentiles(&self, p95: f64, p99: f64) -> Option<[f64; 3]> {
        let current = *self.0.back()?;
        let mut sorted: Vec<_> = self.0.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let percentile =
            |p: f64| sorted[((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1)];
        Some([current, percentile(p95), percentile(p99)])
    }
}
#[derive(Clone, Default)]
pub struct Snapshot {
    fps: Option<[f64; 3]>,
    cpu: Option<[f64; 3]>,
    gpu: Option<[f64; 3]>,
    gpu_supported: bool,
}
impl Snapshot {
    pub fn ui(&self, ui: &mut egui::Ui) {
        let mut lines = Vec::with_capacity(3);
        for (name, values, unit) in [
            ("FPS", self.fps, ""),
            ("GPU", self.gpu, "ms"),
            ("CPU", self.cpu, "ms"),
        ] {
            lines.push(match values {
                Some(v) => format!(
                    "{name} {unit:2} curr {:6.2}  p95 {:6.2}  p99 {:6.2}",
                    v[0], v[1], v[2]
                ),
                None => format!(
                    "{name} {unit:2} {}",
                    if name == "GPU" && !self.gpu_supported {
                        "N/A"
                    } else {
                        "warming up"
                    }
                ),
            });
        }
        let response = ui.add(
            egui::Label::new(
                egui::RichText::new(lines.join("\n"))
                    .font(egui::FontId::monospace(
                        crate::ui::loudness_indicator::LOUDNESS_FONT_SIZE,
                    ))
                    .color(ui.visuals().text_color()),
            )
            .wrap_mode(egui::TextWrapMode::Extend),
        );
        response.on_hover_text("Rolling window: last 600 samples. FPS p95/p99 show the lower 5th/1st percentiles (slow-frame tail); CPU/GPU show upper 95th/99th percentiles. CPU: elapsed render-thread frame preparation through command encoding; excludes surface acquisition, queue submission, presentation/vsync, and audio-thread work. Includes any waits within frame preparation. GPU: all encoded frame work; asynchronous samples may lag or skip frames. N/A means timestamp queries are unsupported.");
    }
}
struct Gpu {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    pending: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    period: f64,
}
pub struct Performance {
    fps: Samples,
    cpu: Samples,
    gpu_samples: Samples,
    previous: Option<Instant>,
    gpu: Option<Gpu>,
}
impl Performance {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let gpu = device
            .features()
            .contains(
                wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
            )
            .then(|| Gpu {
                queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("frame timing"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                }),
                resolve: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("timing resolve"),
                    size: 16,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                readback: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("timing readback"),
                    size: 16,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                pending: None,
                period: queue.get_timestamp_period() as f64,
            });
        Self {
            fps: Samples::default(),
            cpu: Samples::default(),
            gpu_samples: Samples::default(),
            previous: None,
            gpu,
        }
    }
    pub fn begin(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) -> bool {
        profiling::function_scope!();
        let now = Instant::now();
        if let Some(previous) = self.previous.replace(now) {
            self.fps
                .push(1.0 / now.duration_since(previous).as_secs_f64());
        }
        let _ = device.poll(wgpu::PollType::Poll);
        if let Some(gpu) = &mut self.gpu {
            if let Some(result) = gpu.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
                gpu.pending = None;
                if result.is_ok() {
                    let mapped = gpu.readback.slice(..).get_mapped_range();
                    let start = u64::from_ne_bytes(mapped[0..8].try_into().unwrap());
                    let end = u64::from_ne_bytes(mapped[8..16].try_into().unwrap());
                    if end >= start {
                        self.gpu_samples
                            .push((end - start) as f64 * gpu.period / 1_000_000.0);
                    }
                    drop(mapped);
                    gpu.readback.unmap();
                }
            }
            if gpu.pending.is_none() {
                encoder.write_timestamp(&gpu.queries, 0);
                return true;
            }
        }
        false
    }
    pub fn end_gpu(&self, encoder: &mut wgpu::CommandEncoder, measured: bool) {
        if measured {
            if let Some(gpu) = &self.gpu {
                encoder.write_timestamp(&gpu.queries, 1);
                encoder.resolve_query_set(&gpu.queries, 0..2, &gpu.resolve, 0);
                encoder.copy_buffer_to_buffer(&gpu.resolve, 0, &gpu.readback, 0, 16);
            }
        }
    }
    pub fn submitted(&mut self, measured: bool, cpu_ms: f64) {
        self.cpu.push(cpu_ms);
        if measured {
            if let Some(gpu) = &mut self.gpu {
                let (tx, rx) = mpsc::channel();
                gpu.readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        let _ = tx.send(result);
                    });
                gpu.pending = Some(rx);
            }
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        profiling::scope!("performance.percentiles");
        Snapshot {
            fps: self.fps.fps_stats(),
            cpu: self.cpu.stats(),
            gpu: self.gpu_samples.stats(),
            gpu_supported: self.gpu.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentiles_use_nearest_rank_and_current_is_latest() {
        let mut samples = Samples::default();
        assert!(samples.stats().is_none());
        for value in 1..=100 {
            samples.push(value as f64);
        }
        samples.push(0.0);
        assert_eq!(samples.stats(), Some([0.0, 95.0, 99.0]));
    }
    #[test]
    fn fps_percentiles_show_the_slow_tail() {
        let mut samples = Samples::default();
        for value in 1..=100 {
            samples.push(value as f64);
        }
        assert_eq!(samples.fps_stats(), Some([100.0, 5.0, 1.0]));
        assert_eq!(samples.stats(), Some([100.0, 95.0, 99.0]));
    }
    #[test]
    fn rolling_window_drops_old_samples() {
        let mut samples = Samples::default();
        samples.push(10_000.0);
        for _ in 0..WINDOW {
            samples.push(2.0);
        }
        assert_eq!(samples.0.len(), WINDOW);
        assert_eq!(samples.stats(), Some([2.0; 3]));
    }
}
