use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::ui::aa_line::{AaLines, LineScratch, Target};
use crate::ui::plot::{Axis, PlotData, XYPlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::wavesync::{WaveSyncAppData, WaveSyncVisuals};
use crate::{deref_arc, impl_settings};
use egui;
use egui::Ui;
use egui::epaint::PaintCallbackInfo;
use egui::{PaintCallback, Rect, Slider};
use egui_wgpu::wgpu;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const MAX_LINE_SEGMENTS: usize = 2000;
const PIXELS_PER_WAVE: u64 = 200;
const MIN_DISPLAYED_SAMPLES: u64 = 10;

deref_arc!(WaveformVisualizer);

pub struct Inner {
    audio_service: AudioService,
    settings_open: AtomicBool,
    channel: AudioChannel,
    data: Arc<RwLock<WaveSyncAppData>>,
    render_resources: Mutex<Option<RenderResources>>,
    scratch: Mutex<WaveformScratch>,
}

#[derive(Default)]
struct WaveformScratch {
    uniform_uploads: crate::ui::uniform_upload::UniformUploads,
    samples: Vec<f32>,
    vertices: Vec<[f32; 2]>,
    line: LineScratch,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WaveformSettings {
    pub align_to_peak: bool,
    pub duration: Duration,
    pub range: f32,
}

impl Default for WaveformSettings {
    fn default() -> Self {
        Self {
            align_to_peak: true,
            duration: Duration::from_millis(150),
            range: 1.2,
        }
    }
}

impl WaveformVisualizer {
    pub fn new(
        audio_service: AudioService,
        channel: AudioChannel,
        data: Arc<RwLock<WaveSyncAppData>>,
    ) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            channel,
            settings_open: Default::default(),
            data,
            render_resources: Default::default(),
            scratch: Default::default(),
        }))
    }
}

impl Visualizer for WaveformVisualizer {
    fn get_plot_data(&self) -> PlotData {
        let range = self.data.read().unwrap().waveform_settings.range;
        PlotData::XY(
            XYPlotData::from_axis(
                Axis::linear(0.0, 1.0),
                Axis::linear(-range, range).always_show_zero(true),
            )
            .x_axis_shown(false),
        )
    }

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> Option<PaintCallback> {
        Some(egui_wgpu::Callback::new_paint_callback(
            rect,
            WaveformVisualizerCallback::new(self.clone(), visuals),
        ))
    }

    impl_settings!("Waveform Settings", ui, this, {
        let settings = &mut this.data.write().unwrap().waveform_settings;

        ui.horizontal(|ui| {
            ui.checkbox(&mut settings.align_to_peak, "Align to peak");
        });

        ui.horizontal(|ui| {
            ui.label("Duration");
            let mut duration_ms = settings.duration.as_millis() as u64;
            ui.add(Slider::new(&mut duration_ms, 50..=500));
            settings.duration = Duration::from_millis(duration_ms);
            ui.label("ms");
        });

        ui.horizontal(|ui| {
            ui.label("Range");
            ui.add(Slider::new(&mut settings.range, 0.1..=3.0));
        })
    });
}

pub struct WaveformVisualizerCallback {
    visualizer: WaveformVisualizer,
    color: egui::Color32,
}

impl WaveformVisualizerCallback {
    pub(crate) fn new(visualizer: WaveformVisualizer, visuals: &WaveSyncVisuals) -> Self {
        Self {
            visualizer,
            color: visuals.wave_color(),
        }
    }
}

struct RenderResources {
    queue: wgpu::Queue,
    line: AaLines,
}

impl CallbackTrait for WaveformVisualizerCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        _resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        profiling::scope!("waveform.prepare");
        let mut resources = self.visualizer.render_resources.lock().unwrap();

        if resources.is_none() {
            self.visualizer.scratch.lock().unwrap().uniform_uploads = Default::default();
            *resources = Some(RenderResources {
                queue: queue.clone(),
                line: AaLines::new(device, MAX_LINE_SEGMENTS, Target::Color),
            });
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _resources: &CallbackResources,
    ) {
        profiling::scope!("waveform.paint");
        let channel = self.visualizer.channel;
        let settings = &self.visualizer.data.read().unwrap().waveform_settings;
        let source = self.visualizer.audio_service.get_source();
        let audio_service = &self.visualizer.audio_service;
        let plot_data = self.visualizer.get_plot_data().expect_xy();
        let resources = self.visualizer.render_resources.lock().unwrap();
        if let Some(resources) = resources.as_ref() {
            let queue = &resources.queue;
            let half_buffer_size = (MAX_LINE_SEGMENTS / 2) as u32;
            let buffer_size = MAX_LINE_SEGMENTS as u32;

            let to_read = (settings.duration.as_secs_f32() * source.sample_rate as f32) as usize;
            let to_read = floor_to_nearest(to_read, half_buffer_size as usize);

            let mut drop = 0;
            let mut take = to_read;
            let peak = audio_service.get_fft_peak(channel);

            if settings.align_to_peak
                && let Some(peak) = peak
            {
                let align_low_pass =
                    (1.2f32 / (to_read as f32 / source.sample_rate as f32)).max(1.0);
                let should_do_alignement = peak.value > 0.0001
                    && peak.interpolated_frequency <= 20000.0
                    && peak.interpolated_frequency >= align_low_pass;

                if should_do_alignement {
                    let to_read = to_read as u64;
                    let max_waves = (info.viewport.width() as u64 / PIXELS_PER_WAVE).clamp(1, 50);
                    let wave_size = source.wave_length(peak.interpolated_frequency.floor()) as f64;

                    drop = (wave_size - audio_service.get_samples_written() as f64 % wave_size)
                        .max(0.0)
                        .min((to_read - MIN_DISPLAYED_SAMPLES) as f64)
                        as u64;
                    take = to_read
                        .saturating_sub(wave_size as u64)
                        .max(1)
                        .min(wave_size as u64 * max_waves)
                        .min(to_read - drop) as usize;
                }
            }

            let mut scratch = self.visualizer.scratch.lock().unwrap();
            let WaveformScratch {
                uniform_uploads,
                samples: latest_samples,
                vertices,
                line,
            } = &mut *scratch;
            audio_service.get_samples_aligned_into(
                channel,
                to_read,
                drop as usize,
                take,
                latest_samples,
            );
            let step = (latest_samples.len() / half_buffer_size as usize).max(1);

            crate::resize_scratch!(
                *vertices,
                (buffer_size as usize).min(latest_samples.len()),
                [0.0, 0.0]
            );
            let mut vertices_written = 0;
            for (i, sample) in latest_samples.iter().enumerate().step_by(step) {
                let vertex_index = i / step;
                if vertex_index >= buffer_size as usize {
                    break;
                }
                vertices[vertex_index] = [
                    i as f32 / latest_samples.len() as f32 * 2.0 - 1.0,
                    plot_data.y_axis.gl_pos(*sample),
                ];
                vertices_written += 1;
            }

            vertices.truncate(vertices_written as usize);
            let viewport = [
                info.viewport.width() * info.pixels_per_point,
                info.viewport.height() * info.pixels_per_point,
            ];
            line.polyline(vertices, viewport);
            resources.line.upload(queue, &line.segments);
            resources.line.configure(
                queue,
                uniform_uploads,
                self.color.to_normalized_gamma_f32(),
                viewport,
            );
            resources.line.draw(render_pass, line.segments.len());
        }
    }
}

fn floor_to_nearest(x: usize, n: usize) -> usize {
    (x / n) * n
}
