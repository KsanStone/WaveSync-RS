use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::sound::loudness::rms::calc_rms;
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{Ui, VERTEX_2D_BUFFER_LAYOUT, create_pipeline, uniform_bindings};
use crate::wavesync::{WaveSyncAppData, WaveSyncVisuals};
use crate::{deref_arc, impl_settings};
use egui::{PaintCallback, PaintCallbackInfo, Rect, Slider};
use egui_wgpu::wgpu::util::DeviceExt;
use egui_wgpu::wgpu::{BufferAddress, CommandBuffer, CommandEncoder, Device, Queue, RenderPass};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor, wgpu};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};

deref_arc!(ExtendedWaveformVisualizer);

const MAX_RESOLUTION: usize = 8192;

pub struct Inner {
    audio_service: AudioService,
    settings_open: AtomicBool,
    channel: AudioChannel,
    data: Arc<RwLock<WaveSyncAppData>>,
    render_resources: Mutex<Option<RenderResources>>,
}

pub struct ExtendedWaveformVisualizerCallback {
    visualizer: ExtendedWaveformVisualizer,
    sample_line_color: egui::Color32,
    rms_color: egui::Color32,
}

impl ExtendedWaveformVisualizer {
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
        }))
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExtendedWaveformVisualizerSettings {
    pub range: f32,
}

impl Default for ExtendedWaveformVisualizerSettings {
    fn default() -> Self {
        Self { range: 1.2 }
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    color: [f32; 4],
}

impl Visualizer for ExtendedWaveformVisualizer {
    fn get_plot_data(&self) -> PlotData {
        let range = self.data.read().unwrap().extended_waveform_settings.range;
        PlotData::from_axis(
            Axis::linear(-1.0, 0.0),
            Axis::linear(-range, range).always_show_zero(true),
        )
        .x_axis_shown(false)
    }

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> Option<PaintCallback> {
        Some(egui_wgpu::Callback::new_paint_callback(
            rect,
            ExtendedWaveformVisualizerCallback {
                visualizer: self.clone(),
                sample_line_color: visuals.ex_wave_color(),
                rms_color: visuals.rms_color(),
            },
        ))
    }

    impl_settings!("Extended Waveform Settings", ui, this, {
        let settings = &mut this.data.write().unwrap().extended_waveform_settings;

        ui.horizontal(|ui| {
            ui.label("Range");
            ui.add(Slider::new(&mut settings.range, 0.1..=3.0));
        })
    });
}

impl CallbackTrait for ExtendedWaveformVisualizerCallback {
    fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut CommandEncoder,
        _callback_resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        let mut resources = self.visualizer.render_resources.lock().unwrap();

        if resources.is_none() {
            let vertex_buffer_sample = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ex-wave line vertex buffer 1"),
                size: (size_of::<f32>() * 2 * MAX_RESOLUTION) as BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let vertex_buffer_rms = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ex-wave line vertex buffer 2"),
                size: (size_of::<f32>() * 2 * MAX_RESOLUTION) as BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("line shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../../../shader/colored_line.wgsl").into(),
                ),
            });

            let uniform_buffer_sample =
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("waveform_uniform_buffer_1"),
                    contents: bytemuck::cast_slice(&[Uniforms {
                        color: [1.0, 0.0, 1.0, 1.0],
                    }]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });

            let uniform_buffer_rms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("waveform_uniform_buffer_2"),
                contents: bytemuck::cast_slice(&[Uniforms {
                    color: [1.0, 0.0, 1.0, 1.0],
                }]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let (bind_group_layout, bind_group_sample) =
                uniform_bindings(device, 0, &uniform_buffer_sample, "ex-waveform-1");

            let (_, bind_group_rms) =
                uniform_bindings(device, 0, &uniform_buffer_rms, "ex-waveform-2");

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("extended waveform layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            let pipeline = create_pipeline(
                device,
                &shader,
                &pipeline_layout,
                wgpu::PrimitiveTopology::LineList,
                &[VERTEX_2D_BUFFER_LAYOUT],
                "extended-waveform-pipeline",
            );

            *resources = Some(RenderResources {
                queue: queue.clone(),
                vertex_buffer_sample,
                vertex_buffer_rms,
                uniform_buffer_sample,
                uniform_buffer_rms,
                bind_group_sample,
                bind_group_rms,
                pipeline,
            })
        }

        vec![]
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut RenderPass<'static>,
        _callback_resources: &CallbackResources,
    ) {
        if let Some(resources) = self.visualizer.render_resources.lock().unwrap().as_ref() {
            let width = (info.viewport.width() * info.pixels_per_point) as usize;

            let mut vertices = Vec::with_capacity(width * 2);
            let mut rms_vertices = Vec::with_capacity(width * 2);
            let plot_data = self.visualizer.get_plot_data();

            {
                let buf = &self.visualizer.audio_service.audio_buffer.lock().unwrap()
                    [self.visualizer.channel.get_index()];
                let written = self.visualizer.audio_service.get_samples_written();
                let size = buf.len();
                let chunk_size = size / (width + 1);
                let skip = chunk_size.saturating_sub((written % chunk_size as u64) as usize);

                for i in 0..width {
                    let start = i * chunk_size + skip;
                    let end = start + chunk_size;

                    let min_max = buf.range(start..end).minmax();
                    let (min, max) = min_max.into_option().unwrap_or((&0.0, &0.0));
                    let x = (i as f32) / (width as f32) * 2.0 - 1.0;

                    let rms = calc_rms(buf.range(start..end), chunk_size);
                    let rms_min = (-rms).clamp(*min, *max);
                    let rms_max = rms.clamp(*min, *max);

                    vertices.push([x, plot_data.y_axis.gl_pos(*min)]);
                    vertices.push([x, plot_data.y_axis.gl_pos(*max)]);

                    rms_vertices.push([x, plot_data.y_axis.gl_pos(rms_min)]);
                    rms_vertices.push([x, plot_data.y_axis.gl_pos(rms_max)]);
                }
            }

            resources.queue.write_buffer(
                &resources.vertex_buffer_sample,
                0,
                bytemuck::cast_slice(&vertices),
            );

            resources.queue.write_buffer(
                &resources.uniform_buffer_sample,
                0,
                bytemuck::bytes_of(&Uniforms {
                    color: self.sample_line_color.to_normalized_gamma_f32(),
                }),
            );

            resources.queue.write_buffer(
                &resources.vertex_buffer_rms,
                0,
                bytemuck::cast_slice(&rms_vertices),
            );

            resources.queue.write_buffer(
                &resources.uniform_buffer_rms,
                0,
                bytemuck::bytes_of(&Uniforms {
                    color: self.rms_color.to_normalized_gamma_f32(),
                }),
            );

            render_pass.set_pipeline(&resources.pipeline);

            render_pass.set_bind_group(0, &resources.bind_group_sample, &[]);
            render_pass.set_vertex_buffer(0, resources.vertex_buffer_sample.slice(..));
            render_pass.draw(0..vertices.len() as u32, 0..1);

            render_pass.set_bind_group(0, &resources.bind_group_rms, &[]);
            render_pass.set_vertex_buffer(0, resources.vertex_buffer_rms.slice(..));
            render_pass.draw(0..rms_vertices.len() as u32, 0..1);
        }
    }
}

struct RenderResources {
    queue: Queue,
    vertex_buffer_sample: wgpu::Buffer,
    vertex_buffer_rms: wgpu::Buffer,
    uniform_buffer_sample: wgpu::Buffer,
    uniform_buffer_rms: wgpu::Buffer,
    bind_group_sample: wgpu::BindGroup,
    bind_group_rms: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}
