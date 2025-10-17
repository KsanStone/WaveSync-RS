use crate::egui::Ui;
use crate::sound::audio_service::AudioService;
use crate::sound::smoothing::FloatArraySmoother;
use crate::sound::smoothing::exponential_falloff_smoother::ExponentialFalloffSmoother;
use crate::sound::smoothing::multiplicative_smoother::MultiplicativeSmoother;
use crate::sound::{AudioChannel, frequency_of_bin, scale_to_db};
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{QUAD_VERTICES, VERTEX_2D_BUFFER_LAYOUT, create_pipeline};
use crate::{
    WaveSyncAppData, WaveSyncVisuals, create_shader, define_resource, deref_arc, impl_settings,
};
use eframe::egui::{Color32, PaintCallback, PaintCallbackInfo, Rect};
use eframe::wgpu::util::DeviceExt;
use eframe::wgpu::{CommandBuffer, CommandEncoder, Device, Queue, RenderPass};
use eframe::{egui, wgpu};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use log::info;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

pub const MAX_BARS: u64 = 4096;
pub const MIN_BAR_WIDTH: f32 = 1.0;

deref_arc!(SpectrumVisualizer);

pub struct Inner {
    audio_service: AudioService,
    plot_data: Mutex<PlotData>,
    data: Arc<RwLock<WaveSyncAppData>>,
    smoother: Mutex<Option<Box<dyn FloatArraySmoother>>>,
    settings_open: AtomicBool,
    last_draw: Mutex<Instant>,
    channel: AudioChannel,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SpectrumVisualizerSettings {
    pub draw_type: SpectrumVisualizerType,
    pub smoother_type: SmootherType,
    pub smoother_factor: f32,
    pub frequency_axis_logarithmic: bool,
}

impl Default for SpectrumVisualizerSettings {
    fn default() -> Self {
        Self {
            draw_type: SpectrumVisualizerType::Bar,
            smoother_type: SmootherType::Multiplicative,
            smoother_factor: 0.7,
            frequency_axis_logarithmic: true,
        }
    }
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub enum SpectrumVisualizerType {
    Bar,
    Line,
}

#[derive(PartialEq, Copy, Clone, Debug, Serialize, Deserialize)]
pub enum SmootherType {
    None,
    Multiplicative,
    ExponentialFalloff,
}

impl SpectrumVisualizer {
    pub fn new(
        audio_service: AudioService,
        channel: AudioChannel,
        data: Arc<RwLock<WaveSyncAppData>>,
    ) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            channel,
            plot_data: Mutex::new(PlotData::from_axis(
                Axis::logarithmic(12.0, 20000.0),
                Axis::linear(-100.0, 0.0),
            )),
            data,
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

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> PaintCallback {
        egui_wgpu::Callback::new_paint_callback(
            rect,
            SpectrumVisualizerCallback::new(self.clone(), visuals),
        )
    }

    impl_settings!("Spectrum Settings", ui, this, {
        let settings = &mut this.data.write().unwrap().spectrum_settings;
        let mut plot_data = this.plot_data.lock().unwrap();

        ui.horizontal(|ui| {
            ui.label("Render mode: ");
            egui::ComboBox::from_id_salt("drawtype")
                .selected_text(format!("{:?}", settings.draw_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut settings.draw_type,
                        SpectrumVisualizerType::Bar,
                        "Bar",
                    );
                    ui.selectable_value(
                        &mut settings.draw_type,
                        SpectrumVisualizerType::Line,
                        "Line",
                    );
                });
        });

        ui.horizontal(|ui| {
            ui.label("Frequency axis: ");
            egui::ComboBox::from_id_salt("freq_axis")
                .selected_text(if settings.frequency_axis_logarithmic {
                    "Logarithmic"
                } else {
                    "Linear"
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(settings.frequency_axis_logarithmic, "Logarithmic")
                        .clicked()
                    {
                        settings.frequency_axis_logarithmic = true;
                    }
                    if ui
                        .selectable_label(!settings.frequency_axis_logarithmic, "Linear")
                        .clicked()
                    {
                        settings.frequency_axis_logarithmic = false;
                    }
                });
            plot_data.x_axis.logarithmic = settings.frequency_axis_logarithmic;
        });

        ui.horizontal(|ui| {
            ui.label("Smoothing: ");
            let before_type = settings.smoother_type;
            egui::ComboBox::from_id_salt("smoothing")
                .selected_text(format!("{:?}", settings.smoother_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut settings.smoother_type, SmootherType::None, "None");
                    ui.selectable_value(
                        &mut settings.smoother_type,
                        SmootherType::Multiplicative,
                        "Multiplicative",
                    );
                    ui.selectable_value(
                        &mut settings.smoother_type,
                        SmootherType::ExponentialFalloff,
                        "Exponential falloff",
                    );
                });

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

        ui.style_mut().spacing.slider_width = 150.0;
        ui.add(
            egui::Slider::new(&mut settings.smoother_factor, 0.0..=1.0)
                .text("Factor")
                .min_decimals(3)
                .step_by(0.005),
        );
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
        Self {
            visualizer,
            color_start: visuals.color_start(),
            color_end: visuals.color_end(),
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    color_end: [f32; 4],
    color_start: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct BarInstanceData {
    height: f32,
    x_1: f32,
    x_2: f32,
}

define_resource!(SpectrumBarsPipeline, wgpu::RenderPipeline);
define_resource!(SpectrumLinePipeline, wgpu::RenderPipeline);
define_resource!(SpectrumLineFillPipeline, wgpu::RenderPipeline);
define_resource!(SpectrumVertexBuffer, wgpu::Buffer);
define_resource!(SpectrumInstanceBuffer, wgpu::Buffer);
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
            resources.insert(SpectrumVertexBuffer(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("spectrum_vertex_buffer"),
                    contents: bytemuck::cast_slice(&QUAD_VERTICES),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            )));

            resources.insert(SpectrumInstanceBuffer(device.create_buffer(
                &wgpu::BufferDescriptor {
                    label: Some("spectrum_instance_buffer"),
                    size: MAX_BARS * 4 * size_of::<f64>() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            )));

            let bar_shader =
                create_shader!(device, "bar shader", "../../../shader/instanced_bars.wgsl");
            let line_shader =
                create_shader!(device, "line shader", "../../../shader/colored_line.wgsl");
            let line_fill_shader =
                create_shader!(device, "lf", "../../../shader/instanced_line_fill.wgsl");

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
                &bar_shader,
                &pipeline_layout,
                Default::default(),
                &[
                    VERTEX_2D_BUFFER_LAYOUT,
                    wgpu::VertexBufferLayout {
                        array_stride: size_of::<BarInstanceData>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                offset: 0,
                                shader_location: 1,
                                format: wgpu::VertexFormat::Float32,
                            },
                            wgpu::VertexAttribute {
                                offset: 4,
                                shader_location: 2,
                                format: wgpu::VertexFormat::Float32,
                            },
                            wgpu::VertexAttribute {
                                offset: 8,
                                shader_location: 3,
                                format: wgpu::VertexFormat::Float32,
                            },
                        ],
                    },
                ],
                "spectrum_bar_pipeline",
            )));

            resources.insert(SpectrumLineFillPipeline(create_pipeline(
                device,
                &line_fill_shader,
                &pipeline_layout,
                Default::default(),
                &[
                    VERTEX_2D_BUFFER_LAYOUT,
                    wgpu::VertexBufferLayout {
                        array_stride: 4 * size_of::<f32>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        }],
                    },
                ],
                "spectrum_line_fill_pipeline",
            )));

            resources.insert(SpectrumLinePipeline(create_pipeline(
                device,
                &line_shader,
                &pipeline_layout,
                wgpu::PrimitiveTopology::LineList,
                &[VERTEX_2D_BUFFER_LAYOUT],
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
        let delta_t = self.visualizer.last_draw.lock().unwrap().elapsed();
        *self.visualizer.last_draw.lock().unwrap() = Instant::now();

        let channel = self.visualizer.channel;
        let bar_pipeline = callback_resources.get::<SpectrumBarsPipeline>().unwrap();
        let line_pipeline = callback_resources.get::<SpectrumLinePipeline>().unwrap();
        let line_fill_pipeline = callback_resources
            .get::<SpectrumLineFillPipeline>()
            .unwrap();
        let uniform_buffer = callback_resources.get::<SpectrumUniformBuffer>().unwrap();
        let bind_group = callback_resources.get::<SpectrumBindGroup>().unwrap();

        let plot_data = self.visualizer.plot_data.lock().unwrap();
        let instance_buffer = callback_resources.get::<SpectrumInstanceBuffer>().unwrap();
        let vertex_buffer = callback_resources.get::<SpectrumVertexBuffer>().unwrap();
        let queue = callback_resources.get::<Queue>().unwrap();
        let current_source = self.visualizer.audio_service.get_source();
        let settings = &self.visualizer.data.read().unwrap().spectrum_settings;

        let fft_target = self.visualizer.audio_service.get_fft(channel);
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
        let mut instance_data = Vec::with_capacity(bars_to_draw);
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

            instance_data.push(BarInstanceData {
                height: plot_data.y_axis.gl_pos(v),
                x_1: gl_pos,
                x_2: gl_next_pos,
            });

            bars_drawn += 1;
        }

        queue.write_buffer(
            uniform_buffer,
            0,
            bytemuck::bytes_of(&Uniforms {
                color_end: self.color_end.to_normalized_gamma_f32(),
                color_start: self.color_start.to_normalized_gamma_f32(),
            }),
        );

        render_pass.set_bind_group(0, &bind_group.0, &[]);

        if bars_drawn == 0 {
            return;
        }

        if settings.draw_type == SpectrumVisualizerType::Line {
            let vertex_array: Vec<_> = instance_data
                .windows(2)
                .flat_map(|w| [[w[0].x_1, w[0].height], [w[1].x_1, w[1].height]])
                .collect();

            queue.write_buffer(instance_buffer, 0, bytemuck::cast_slice(&vertex_array));
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, instance_buffer.slice(..));
            render_pass.set_pipeline(line_fill_pipeline);
            render_pass.draw(0..6u32, 0..(bars_drawn - 1));

            render_pass.set_vertex_buffer(0, instance_buffer.slice(..));
            render_pass.set_pipeline(line_pipeline);
            render_pass.draw(0..bars_drawn * 2u32, 0..(bars_drawn - 1));
        } else {
            queue.write_buffer(instance_buffer, 0, bytemuck::cast_slice(&instance_data));
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, instance_buffer.slice(..));
            render_pass.set_pipeline(bar_pipeline);
            render_pass.draw(0..6u32, 0..bars_drawn);
        }
    }
}
