use crate::egui::Ui;
use crate::sound::audio_service::AudioService;
use crate::sound::smoothing::FloatArraySmoother;
use crate::sound::smoothing::exponential_falloff_smoother::ExponentialFalloffSmoother;
use crate::sound::smoothing::multiplicative_smoother::MultiplicativeSmoother;
use crate::sound::{AudioChannel, frequency_of_bin, scale_to_db};
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{create_pipeline, quad_to_triangles};
use crate::{WaveSyncVisuals, define_resource, deref_arc, impl_settings};
use eframe::egui::{Color32, PaintCallback, PaintCallbackInfo, Rect, Visuals};
use eframe::wgpu::util::DeviceExt;
use eframe::wgpu::{CommandBuffer, CommandEncoder, Device, Queue, RenderPass};
use eframe::{egui, wgpu};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use log::info;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const MAX_BARS: u64 = 4096;
pub const MIN_BAR_WIDTH: f32 = 1.0;

deref_arc!(SpectrumVisualizer);

pub struct Inner {
    audio_service: AudioService,
    plot_data: Mutex<PlotData>,
    settings: Mutex<SpectrumVisualizerSettings>,
    smoother: Mutex<Option<Box<dyn FloatArraySmoother>>>,
    settings_open: AtomicBool,
    last_draw: Mutex<Instant>,
}

pub struct SpectrumVisualizerSettings {
    pub channel: AudioChannel,
    pub draw_type: SpectrumVisualizerType,
    pub smoother_type: SmootherType,
    pub smoother_factor: f32,
    pub frequency_axis_logarithmic: bool,
}

#[derive(PartialEq)]
pub enum SpectrumVisualizerType {
    Bar,
    Line,
}

#[derive(PartialEq, Copy, Clone)]
pub enum SmootherType {
    None,
    Multiplicative,
    ExponentialFalloff,
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
                smoother_type: SmootherType::Multiplicative,
                smoother_factor: 0.6,
                frequency_axis_logarithmic: true,
            }),
            smoother: Mutex::new(Some(Box::new(MultiplicativeSmoother::new(0.6)))),
            settings_open: Default::default(),
            last_draw: Mutex::new(Instant::now()),
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

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> PaintCallback {
        egui_wgpu::Callback::new_paint_callback(rect, SpectrumVisualizerCallback::new(self.clone(), visuals))
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

            ui.radio_value(
                &mut settings.frequency_axis_logarithmic,
                true,
                "Logarithmic",
            );
            ui.radio_value(&mut settings.frequency_axis_logarithmic, false, "Linear");

            plot_data.x_axis.logarithmic = settings.frequency_axis_logarithmic;
        });
        ui.horizontal(|ui| {
            ui.label("Smoothing: ");
            let before_type = settings.smoother_type;
            ui.radio_value(&mut settings.smoother_type, SmootherType::None, "None");
            ui.radio_value(
                &mut settings.smoother_type,
                SmootherType::Multiplicative,
                "Multiplicative",
            );
            ui.radio_value(
                &mut settings.smoother_type,
                SmootherType::ExponentialFalloff,
                "Exponential falloff",
            );
            if before_type != settings.smoother_type {
                let mut smoother = this.smoother.lock().unwrap();
                match settings.smoother_type {
                    SmootherType::None => smoother.take(),
                    SmootherType::Multiplicative => {
                        smoother.replace(Box::new(MultiplicativeSmoother::new(0.1)))
                    }
                    SmootherType::ExponentialFalloff => {
                        smoother.replace(Box::new(ExponentialFalloffSmoother::new()))
                    }
                };
            }
        });
        ui.add(egui::Slider::new(&mut settings.smoother_factor, 0.0..=1.0).text("Factor"));
        if let Some(smoother) = this.smoother.lock().unwrap().as_mut() {
            smoother.set_factor(settings.smoother_factor);
        }
    });
}

pub struct SpectrumVisualizerCallback {
    visualizer: SpectrumVisualizer,
    color_start: Color32,
    color_end: Color32,
}

impl SpectrumVisualizerCallback {
    pub(crate) fn new(visualizer: SpectrumVisualizer, visuals: &WaveSyncVisuals) -> Self {
        Self { visualizer, color_start: visuals.wave_color(), color_end: visuals.wave_color() }
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

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    color_start: [f32; 4],
    color_end: [f32; 4],
}

define_resource!(SpectrumBarsPipeline, wgpu::RenderPipeline);
define_resource!(SpectrumLinePipeline, wgpu::RenderPipeline);
define_resource!(SpectrumVertexBuffer, wgpu::Buffer);
define_resource!(SpectrumUniformBuffer, wgpu::Buffer);
define_resource!(SpectrumBindGroup, wgpu::BindGroup);

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
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../../../shader/colored_line.wgsl").into(),
                ),
            });

            let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("spectrum_uniform_buffer"),
                contents: bytemuck::cast_slice(&[Uniforms {
                    color_start: [1.0, 1.0, 1.0, 1.0],
                    color_end: [1.0, 1.0, 1.0, 1.0],
                }]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("spectrum_uniform_buffer_bind_group_layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Spectrum Uniform Bind Group"),
                layout: &bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("spectrum layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            resources.insert(SpectrumBindGroup(bind_group));
            resources.insert(SpectrumUniformBuffer(uniform_buffer));

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

    // TODO use instancing for bar draw
    // TODO color bars based on height
    // TODO fill under the line nigger
    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        if !callback_resources.contains::<SpectrumBarsPipeline>() {
            return;
        }
        let delta_t = self.visualizer.last_draw.lock().unwrap().elapsed();
        *self.visualizer.last_draw.lock().unwrap() = Instant::now();

        let bar_pipeline = callback_resources.get::<SpectrumBarsPipeline>().unwrap();
        let line_pipeline = callback_resources.get::<SpectrumLinePipeline>().unwrap();
        let uniform_buffer = callback_resources.get::<SpectrumUniformBuffer>().unwrap();
        let bind_group = callback_resources.get::<SpectrumBindGroup>().unwrap();

        let plot_data = self.visualizer.plot_data.lock().unwrap();
        let vertex_buffer = callback_resources.get::<SpectrumVertexBuffer>().unwrap();
        let queue = callback_resources.get::<Queue>().unwrap();
        let current_source = self.visualizer.audio_service.get_source();
        let settings = self.visualizer.settings.lock().unwrap();

        let fft_target = self.visualizer.audio_service.get_fft(settings.channel);
        let mut fft_data = fft_target.as_slice();
        let mut smoother = self.visualizer.smoother.lock().unwrap();

        if let Some(smoother) = smoother.as_mut() {
            fft_data = smoother.smooth_data(delta_t.as_secs_f32(), &fft_target);
        }

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
        for (i, sample) in fft_data.iter().enumerate().skip(skip).take(bars_to_draw) {
            let sample = scale_to_db(*sample);
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
        queue.write_buffer(
            uniform_buffer,
            0,
            bytemuck::bytes_of(&Uniforms {
                color_start: self.color_start.to_normalized_gamma_f32(),
                color_end: self.color_end.to_normalized_gamma_f32(),
            }),
        );
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_bind_group(0, &bind_group.0, &[]);

        if settings.draw_type == SpectrumVisualizerType::Line {
            render_pass.set_pipeline(line_pipeline);
            render_pass.draw(0..bars_drawn as u32, 0..(bars_drawn - 1) as u32);
        } else {
            render_pass.set_pipeline(bar_pipeline);
            render_pass.draw(0..(bars_drawn * 2 * 3) as u32, 0..(bars_drawn * 2) as u32);
        }
    }
}
