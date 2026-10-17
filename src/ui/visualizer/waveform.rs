use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::sound::waveform_alignment::{AlignmentMode, Request, WaveformAligner};
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
    aligner: WaveformAligner,
    alignment_source: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WaveformSettings {
    pub align_to_peak: bool,
    #[serde(default)]
    pub alignment_mode: AlignmentMode,
    pub duration: Duration,
    pub range: f32,
}

impl Default for WaveformSettings {
    fn default() -> Self {
        Self {
            align_to_peak: true,
            alignment_mode: AlignmentMode::default(),
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
    fn min_canvas_size(&self) -> egui::Vec2 { egui::vec2(64.0, 24.0) }

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

            let mut scratch = self.visualizer.scratch.lock().unwrap();
            let WaveformScratch {
                uniform_uploads,
                samples: latest_samples,
                vertices,
                line,
                aligner,
                alignment_source,
            } = &mut *scratch;
            let samples_written = audio_service.get_samples_aligned_with_position_into(
                channel,
                to_read,
                0,
                to_read,
                latest_samples,
            );
            if *alignment_source != source.id {
                aligner.reset();
                alignment_source.clone_from(&source.id);
            }
            let peak = audio_service.get_fft_peak(channel);
            let selection = aligner.select(
                latest_samples,
                Request {
                    enabled: settings.align_to_peak,
                    mode: settings.alignment_mode,
                    sample_rate: source.sample_rate,
                    frequency: peak
                        .filter(|p| p.value > 0.0001)
                        .map(|p| p.interpolated_frequency),
                    samples_written,
                    max_waves: (info.viewport.width() as u64 / PIXELS_PER_WAVE).clamp(1, 50)
                        as usize,
                },
            );
            let step = (selection.len / half_buffer_size as usize).max(1);

            crate::resize_scratch!(
                *vertices,
                (buffer_size as usize).min(selection.len),
                [0.0, 0.0]
            );
            let mut vertices_written = 0;
            for i in (0..selection.len).step_by(step) {
                let vertex_index = i / step;
                if vertex_index >= buffer_size as usize {
                    break;
                }
                vertices[vertex_index] = [
                    i as f32 / selection.len as f32 * 2.0 - 1.0,
                    plot_data.y_axis.gl_pos(selection.sample(latest_samples, i)),
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

#[cfg(test)]
mod settings_tests {
    use super::*;
    #[test]
    fn old_settings_load_and_both_alignment_modes_round_trip() {
        let old = "(align_to_peak:true,duration:(secs:0,nanos:150000000),range:1.2)";
        let mut settings: WaveformSettings = ron::from_str(old).unwrap();
        assert_eq!(settings.alignment_mode, AlignmentMode::LocalTrigger);
        for mode in [AlignmentMode::Legacy, AlignmentMode::LocalTrigger] {
            settings.alignment_mode = mode;
            let restored: WaveformSettings =
                ron::from_str(&ron::to_string(&settings).unwrap()).unwrap();
            assert_eq!(restored.alignment_mode, mode);
            assert!(restored.align_to_peak);
        }
    }
}
