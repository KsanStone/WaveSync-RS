use crate::sound::audio_service::AudioService;
use crate::sound::{AudioChannel, db_scale_magnitudes, frequency_of_bin};
use crate::ui::plot::{Axis, PlotData};
use crate::ui::quad_to_triangles;
use crate::ui::visualizer::visualizer_trait::Visualizer;
use crate::{define_resource, deref_arc};
use eframe::egui::PaintCallbackInfo;
use eframe::wgpu;
use eframe::wgpu::{CommandBuffer, CommandEncoder, Device, Queue, RenderPass};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use log::{info, warn};
use std::sync::{Arc, Mutex};

pub const MAX_BARS: u64 = 4096;

deref_arc!(SpectrumVisualizer);

pub struct Inner {
    audio_service: AudioService,
    plot_data: Mutex<PlotData>,
}

impl SpectrumVisualizer {
    pub fn new(audio_service: AudioService) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            plot_data: Mutex::new(PlotData::from_axis(
                Axis::logarithmic(12.0, 20000.0),
                Axis::linear(-90.0, 0.0),
            )),
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

define_resource!(SpectrumPipeline, wgpu::RenderPipeline);
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
        if resources.get::<SpectrumPipeline>().is_none() {
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

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("spectrum pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Option::from("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        }],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Option::from("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Bgra8Unorm,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

            resources.insert(SpectrumPipeline(pipeline));
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        if let Some(pipeline) = callback_resources.get::<SpectrumPipeline>() {
            let plot_data = self.visualizer.plot_data.lock().unwrap();
            let vertex_buffer = callback_resources.get::<SpectrumVertexBuffer>().unwrap();
            let queue = callback_resources.get::<Queue>().unwrap();
            let current_source = self.visualizer.audio_service.get_source();

            let mut fft_data = self.visualizer.audio_service.get_fft(AudioChannel::Master);
            db_scale_magnitudes(&mut fft_data);
            if fft_data.len() > MAX_BARS as usize {
                warn!("FFT data is too large, skipping painting, TODO: implement");
                return;
            }

            let fft_output_size = fft_data.len();
            let fft_size = fft_output_size * 2;
            let skip = current_source.calculate_buffer_beginning_skip_for(plot_data.x_axis.min, fft_size).saturating_sub(1);
            let bars_to_draw = fft_output_size - skip;
            let mut vertex_array = Vec::with_capacity(bars_to_draw * 2 * 3);
            let mut position_array = vec![0.0; fft_output_size + 1];

            for (i, item) in position_array.iter_mut().enumerate().skip(skip) {
                *item = plot_data.x_axis.gl_pos(frequency_of_bin(
                    i,
                    current_source.sample_rate as usize,
                    fft_size,
                ));
            }

            for (i, sample) in fft_data.into_iter().enumerate().skip(skip) {
                let y = plot_data.y_axis.gl_pos(sample);
                vertex_array.extend_from_slice(&quad_to_triangles(
                    position_array[i],
                    -1.0,
                    position_array[i + 1],
                    y,
                ));
            }
            queue.write_buffer(vertex_buffer, 0, bytemuck::cast_slice(&vertex_array));
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_pipeline(pipeline);
            render_pass.draw(0..(bars_to_draw * 2 * 3) as u32, 0..(bars_to_draw * 2) as u32);
        }
    }
}
