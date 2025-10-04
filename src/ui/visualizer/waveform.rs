use std::ops::Deref;
use std::sync::{Arc, Mutex};
use eframe::epaint::PaintCallbackInfo;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

#[derive(Clone)]
pub struct WaveformVisualizer(Arc<Inner>);

impl Deref for WaveformVisualizer {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

pub struct Inner {
    buffer: Mutex<Option<wgpu::Buffer>>,
}

impl WaveformVisualizer {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            buffer: Mutex::new(None),
        }))
    }
}

pub struct WaveformVisualizerCallback {
    visualizer: WaveformVisualizer
}

impl WaveformVisualizerCallback {
    pub(crate) fn new(visualizer: WaveformVisualizer) -> Self {
        Self {
            visualizer
        }
    }

    fn create_vertex_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        let vertices: [[f32; 2]; 3] = [
            [-0.5, -0.5],
            [ 0.5,  0.5],
            [ 0.5,  0.9],
        ];

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Line Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }
}

impl CallbackTrait for WaveformVisualizerCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<wgpu::RenderPipeline>().is_none() {
            println!("Creating pipeline");
            self.visualizer.buffer.lock().unwrap().replace(WaveformVisualizerCallback::create_vertex_buffer(device));
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("line shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../../../shader/line.wgsl").into()),
            });

            let pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
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

            resources.insert(pipeline);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        // Assume we already created pipeline elsewhere (e.g. in resources)
        // Here we'll just set a pipeline and draw
        if let Some(pipeline) = resources.get::<wgpu::RenderPipeline>() {
            let locked_buffer = self.visualizer.buffer.lock().unwrap();
            if locked_buffer.is_some() {
                let buffer = locked_buffer.as_ref().unwrap();
                let nums = buffer.size() as usize / size_of::<f32>();
                let points = (nums / 2) as u32;
                let lines = points - 1;
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.set_pipeline(pipeline);
                pass.draw(0..points, 0..lines);
            }
        }
    }
}