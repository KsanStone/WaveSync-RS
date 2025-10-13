use crate::egui::Ui;
use crate::sound::audio_service::AudioService;
use crate::sound::{AudioChannel, db_scale_magnitudes, frequency_of_bin};
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{create_pipeline, quad_to_triangles};
use crate::{define_resource, deref_arc, impl_settings};
use eframe::egui::{PaintCallback, PaintCallbackInfo, Rect};
use eframe::wgpu::{CommandBuffer, CommandEncoder, Device, Queue, RenderPass};
use eframe::{egui, wgpu};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use log::info;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_BARS: u64 = 4096;
pub const MIN_BAR_WIDTH: f32 = 2.0;

deref_arc!(SpectrumVisualizer);

pub struct Inner {
    audio_service: AudioService,
    plot_data: Mutex<PlotData>,
    settings: Mutex<SpectrumVisualizerSettings>,
    settings_open: AtomicBool,
}

pub struct SpectrumVisualizerSettings {
    pub channel: AudioChannel,
    pub draw_type: SpectrumVisualizerType,
    pub frequency_axis_logarithmic: bool,
}

#[derive(PartialEq)]
pub enum SpectrumVisualizerType {
    Bar,
    Line,
}

impl SpectrumVisualizer {
    pub fn new(audio_service: AudioService) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            plot_data: Mutex::new(PlotData::from_axis(
                Axis::logarithmic(12.0, 20000.0),
                Axis::linear(-90.0, 0.0),
            )),
            settings: Mutex::new(SpectrumVisualizerSettings {
                channel: AudioChannel::Master,
                draw_type: SpectrumVisualizerType::Bar,
                frequency_axis_logarithmic: true,
            }),
            settings_open: Default::default(),
        }))
    }
}

impl Visualizer for SpectrumVisualizer {
    fn get_plot_data(&self) -> PlotData {
        self.plot_data.lock().unwrap().clone()
    }

    fn set_plot_data(&self, plot_data: PlotData) {
        *self.plot_data.lock().unwrap() = plot_data
    }

    fn get_draw_callback(&self, rect: Rect) -> PaintCallback {
        egui_wgpu::Callback::new_paint_callback(rect, SpectrumVisualizerCallback::new(self.clone()))
    }

    impl_settings!("test", ui, this, {
        let mut settings = this.settings.lock().unwrap();
        let mut plot_data = this.plot_data.lock().unwrap();
        
        ui.horizontal(|ui| {
            ui.label("Render mode: ");

            ui.radio_value(&mut settings.draw_type, SpectrumVisualizerType::Bar, "Bar");
            ui.radio_value(
                &mut settings.draw_type,
                SpectrumVisualizerType::Line,
                "Line",
            );
        });
        ui.horizontal(|ui| {
            ui.label("Frequency axis: ");

            ui.radio_value(&mut settings.frequency_axis_logarithmic, true, "Logarithmic");
            ui.radio_value(&mut settings.frequency_axis_logarithmic, false, "Linear");
            
            plot_data.x_axis.logarithmic = settings.frequency_axis_logarithmic;
        })
    });
}

pub struct SpectrumVisualizerCallback {
    visualizer: SpectrumVisualizer,
}

impl SpectrumVisualizerCallback {
    pub(crate) fn new(visualizer: SpectrumVisualizer) -> Self {
        Self { visualizer }
    }

    fn create_vertex_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spectrum_vertex_buffer"),
            // Each bar is 2 triangles, each triangle has 3 vertices each has x and y
            size: MAX_BARS * 2 * 3 * 2 * size_of::<f32>() as u64, // bytes
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
}

define_resource!(SpectrumBarsPipeline, wgpu::RenderPipeline);
define_resource!(SpectrumLinePipeline, wgpu::RenderPipeline);
define_resource!(SpectrumVertexBuffer, wgpu::Buffer);

impl CallbackTrait for SpectrumVisualizerCallback {
    fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        if resources.get::<SpectrumBarsPipeline>().is_none() {
            info!("Creating spectrum pipeline");
            resources.insert(queue.clone());
            resources.insert(SpectrumVertexBuffer(
                SpectrumVisualizerCallback::create_vertex_buffer(device),
            ));

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("line shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../../../shader/line.wgsl").into()),
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("spectrum layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });

            resources.insert(SpectrumBarsPipeline(create_pipeline(
                device,
                &shader,
                &pipeline_layout,
                wgpu::PrimitiveTopology::TriangleList,
                "spectrum_bar_pipeline",
            )));

            resources.insert(SpectrumLinePipeline(create_pipeline(
                device,
                &shader,
                &pipeline_layout,
                wgpu::PrimitiveTopology::LineStrip,
                "spectrum_line_pipeline",
            )));
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        if !callback_resources.contains::<SpectrumBarsPipeline>() {
            return;
        }

        let bar_pipeline = callback_resources.get::<SpectrumBarsPipeline>().unwrap();
        let line_pipeline = callback_resources.get::<SpectrumLinePipeline>().unwrap();

        let plot_data = self.visualizer.plot_data.lock().unwrap();
        let vertex_buffer = callback_resources.get::<SpectrumVertexBuffer>().unwrap();
        let queue = callback_resources.get::<Queue>().unwrap();
        let current_source = self.visualizer.audio_service.get_source();
        let settings = self.visualizer.settings.lock().unwrap();

        let mut fft_data = self.visualizer.audio_service.get_fft(AudioChannel::Master);
        db_scale_magnitudes(&mut fft_data);

        let fft_output_size = fft_data.len();
        let fft_size = fft_output_size * 2;
        let skip = current_source
            .calculate_buffer_beginning_skip_for(plot_data.x_axis.min, fft_size)
            .saturating_sub(1);
        let displayed_bins = current_source
            .bin_of_frequency(plot_data.x_axis.max, fft_size)
            .min(fft_output_size);
        let bars_to_draw = displayed_bins - skip;
        let mut vertex_array = Vec::with_capacity(bars_to_draw * 2 * 3);
        let mut position_array = vec![[0.0, 0.0]; bars_to_draw + 1 + skip];

        for (i, item) in position_array.iter_mut().enumerate().skip(skip) {
            let bin_freq = frequency_of_bin(i, current_source.sample_rate as usize, fft_size);
            *item = [
                plot_data.x_axis.gl_pos(bin_freq),
                plot_data
                    .x_axis
                    .val_to_pos(bin_freq, info.viewport.min.x, info.viewport.max.x),
            ];
        }

        let mut last_px_pos: Option<[f32; 2]> = None;
        let mut acc = 0.0;
        let mut coerced_bins = 0;

        let mut bars_drawn = 0;
        for (i, sample) in fft_data
            .into_iter()
            .enumerate()
            .skip(skip)
            .take(bars_to_draw)
        {
            let [gl_pos, px_pos] = position_array[i];
            let [gl_next_pos, px_next_pos] = position_array[i + 1];
            let mut gl_pos = gl_pos;
            let mut v = sample;

            if (px_pos - px_next_pos).abs() < MIN_BAR_WIDTH {
                if let Some([gl_prev_pos, px_prev_pos]) = last_px_pos.as_ref() {
                    let last_px_pos: f32 = *px_prev_pos;
                    let last_gl_pos: f32 = *gl_prev_pos;
                    acc += sample;
                    coerced_bins += 1;

                    if (px_next_pos - last_px_pos).abs() >= MIN_BAR_WIDTH {
                        v = acc / coerced_bins as f32;
                        gl_pos = last_gl_pos;
                    } else {
                        continue;
                    }
                } else {
                    acc = sample;
                    coerced_bins = 1;
                    last_px_pos = Some([gl_pos, px_pos]);
                    continue;
                }
            }
            last_px_pos = None;

            if settings.draw_type == SpectrumVisualizerType::Line {
                vertex_array.push([gl_pos, plot_data.y_axis.gl_pos(v)]);
            } else {
                vertex_array.extend_from_slice(&quad_to_triangles(
                    gl_pos,
                    -1.0,
                    gl_next_pos,
                    plot_data.y_axis.gl_pos(v),
                ));
            }

            bars_drawn += 1;
        }
        queue.write_buffer(vertex_buffer, 0, bytemuck::cast_slice(&vertex_array));
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));

        if settings.draw_type == SpectrumVisualizerType::Line {
            render_pass.set_pipeline(line_pipeline);
            render_pass.draw(0..bars_drawn as u32, 0..(bars_drawn - 1) as u32);
        } else {
            render_pass.set_pipeline(bar_pipeline);
            render_pass.draw(0..(bars_drawn * 2 * 3) as u32, 0..(bars_drawn * 2) as u32);
        }
    }
}
