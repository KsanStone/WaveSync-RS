use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_trait::Visualizer;
use crate::{define_resource, deref_arc};
use eframe::epaint::PaintCallbackInfo;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use log::info;
use std::mem::size_of;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_LINE_SEGMENTS: usize = 1000;
const PIXELS_PER_WAVE: u64 = 200;
const MIN_DISPLAYED_SAMPLES: u64 = 10;

deref_arc!(WaveformVisualizer);

pub struct Inner {
    audio_service: AudioService,
    plot_data: Mutex<PlotData>,
    settings: Mutex<WaveformSettings>,
}

pub struct WaveformSettings {
    pub channel: AudioChannel,
    pub align_to_peak: bool,
    pub duration: Duration
}

impl WaveformVisualizer {
    pub fn new(audio_service: AudioService) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            plot_data: Mutex::new(PlotData::from_axis(Axis::linear(0.0, 1.0), Axis::linear(-1.0, 1.0)).x_axis_shown(false)),
            settings: Mutex::new(WaveformSettings {
                channel: AudioChannel::Master,
                align_to_peak: true,
                duration: Duration::from_millis(150),
            }),
        }))
    }
}

impl Visualizer for WaveformVisualizer {
    fn get_plot_data(&self) -> PlotData {
        self.plot_data.lock().unwrap().clone()
    }

    fn set_plot_data(&self, plot_data: PlotData) {
        *self.plot_data.lock().unwrap() = plot_data;
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
            resources.insert(WaveformVertexBuffer(
                WaveformVisualizerCallback::create_vertex_buffer(device),
            ));

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
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });

            resources.insert(WaveformPipeline(pipeline));
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let settings = self.visualizer.settings.lock().unwrap();
        let source = self.visualizer.audio_service.get_source();
        let audio_service = &self.visualizer.audio_service;
        if let Some(pipeline) = resources.get::<WaveformPipeline>() {
            let queue = resources.get::<wgpu::Queue>().unwrap();
            let mut locked_buffer = resources.get::<WaveformVertexBuffer>();
            if let Some(buffer) = locked_buffer.as_mut() {
                let nums = buffer.size() as usize / size_of::<f32>();
                let half_buffer_size = (nums / 4) as u32; // 2 floats per vertex
                let buffer_size = half_buffer_size * 2;


                let to_read = (settings.duration.as_secs_f32() * source.sample_rate as f32) as usize;
                let to_read = floor_to_nearest(to_read, half_buffer_size as usize);

                let mut drop = 0;
                let mut take = to_read;
                let peak = audio_service.get_fft_peak(settings.channel);
                if settings.align_to_peak
                    && let Some(peak) = peak
                {
                    let to_read = to_read as u64;
                    let max_waves = (info.viewport.width() as u64 / PIXELS_PER_WAVE).clamp(1, 50);
                    let wave_size = source.wave_length(peak.interpolated_frequency.floor()) as f64;
                    drop = (wave_size - audio_service.get_samples_written() as f64 % wave_size)
                        .max(0.0)
                        .min((to_read - MIN_DISPLAYED_SAMPLES) as f64) as u64;
                    take = (to_read - wave_size as u64)
                        .max(1)
                        .min(wave_size as u64 * max_waves)
                        .min(to_read - drop) as usize;
                }

                let latest_samples = audio_service.get_samples_aligned(
                    settings.channel,
                    to_read,
                    drop as usize,
                    take,
                );
                let step = (latest_samples.len() / half_buffer_size as usize).max(1);

                let mut vertices = vec![[0.0, 0.0]; (buffer_size as usize).min(latest_samples.len())];
                let mut vertices_written = 0;
                for (i, sample) in latest_samples.iter().enumerate().step_by(step) {
                    let vertex_index = i / step;
                    if vertex_index >= buffer_size as usize {
                        break;
                    }
                    vertices[vertex_index] = [i as f32 / latest_samples.len() as f32 * 2.0 - 1.0, *sample];
                    vertices_written += 1;
                }

                queue.write_buffer(buffer, 0, bytemuck::cast_slice(&vertices));

                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.set_pipeline(pipeline);
                pass.draw(0..vertices_written, 0..vertices_written.saturating_sub(1));
            }
        }
    }
}

fn floor_to_nearest(x: usize, n: usize) -> usize {
    (x / n) * n
}
