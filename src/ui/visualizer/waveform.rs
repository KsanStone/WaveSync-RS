use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::ui::plot::PlotData;
use crate::{define_resource, deref_arc};
use eframe::epaint::PaintCallbackInfo;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use log::info;
use std::mem::size_of;
use std::sync::{Arc, Mutex};

const MAX_LINE_SEGMENTS: usize = 512;

deref_arc!(WaveformVisualizer);

pub struct Inner {
    audio_service: AudioService,
}

impl WaveformVisualizer {
    pub fn new(audio_service: AudioService) -> Self {
        Self(Arc::new(Inner {
            audio_service,
        }))
    }

    pub fn update_axis(&self, plot_data: &mut PlotData) {
        plot_data.x_axis = crate::ui::plot::Axis {
            min: 0.0,
            max: 1.0,
            logarithmic: false,
        };
        plot_data.y_axis = crate::ui::plot::Axis {
            min: -1.0,
            max: 1.0,
            logarithmic: false,
        };
    }
}

pub struct WaveformVisualizerCallback {
    visualizer: WaveformVisualizer,
}

impl WaveformVisualizerCallback {
    pub(crate) fn new(visualizer: WaveformVisualizer) -> Self {
        Self { visualizer }
    }

    fn create_vertex_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        let vertices: [[f32; 2]; MAX_LINE_SEGMENTS] = [[0.0, 0.0]; MAX_LINE_SEGMENTS];

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Line Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        })
    }
}

define_resource!(WaveformPipeline, wgpu::RenderPipeline);
define_resource!(WaveformVertexBuffer, wgpu::Buffer);

impl CallbackTrait for WaveformVisualizerCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<WaveformPipeline>().is_none() {
            info!("Creating waveform pipeline");
            resources.insert(queue.clone());
            resources.insert(WaveformVertexBuffer(WaveformVisualizerCallback::create_vertex_buffer(device)));

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("line shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../../../shader/line.wgsl").into()),
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("waveform layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("waveform pipeline"),
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
                    topology: wgpu::PrimitiveTopology::LineStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

            resources.insert(WaveformPipeline(pipeline));
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(pipeline) = resources.get::<WaveformPipeline>() {
            let queue = resources.get::<wgpu::Queue>().unwrap();
            let mut locked_buffer = resources.get::<WaveformVertexBuffer>();
            if let Some(buffer) = locked_buffer.as_mut() {
                let nums = buffer.size() as usize / size_of::<f32>();
                let points = (nums / 2) as u32;

                let to_read = 48000;
                let to_read = floor_to_nearest(to_read, points as usize);

                let latest_samples = self
                    .visualizer
                    .audio_service
                    .get_samples(AudioChannel::Master, to_read);
                let step = (latest_samples.len() / points as usize).max(1);


                let mut vertices = vec![[0.0, 0.0]; (points as usize).min(latest_samples.len())];
                for (i, sample) in latest_samples.iter().enumerate().step_by(step) {
                    let j = i / step;
                    if j >= points as usize {
                        break;
                    }
                    vertices[j] = [i as f32 / latest_samples.len() as f32 * 2.0 - 1.0, *sample];
                }

                queue.write_buffer(buffer, 0, bytemuck::cast_slice(&vertices));

                let vertices = points.min(latest_samples.len() as u32);
                
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.set_pipeline(pipeline);
                pass.draw(0..vertices, 0..vertices.saturating_sub(1));
            }
        }
    }
}

fn floor_to_nearest(x: usize, n: usize) -> usize {
    (x / n) * n
}
