use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{VERTEX_2D_BUFFER_LAYOUT, create_pipeline};
use crate::{WaveSyncAppData, WaveSyncVisuals, create_shader, define_resource, deref_arc};
use eframe::egui::{PaintCallback, PaintCallbackInfo, Rect};
use eframe::wgpu::{
    BufferAddress, CommandBuffer, CommandEncoder, Device, PrimitiveTopology, Queue, RenderPass,
    StorageTextureAccess, TextureFormat, TextureViewDimension,
};
use eframe::{egui, wgpu};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

const MAX_LINE_SEGMENTS: usize = 1000;

deref_arc!(VectorscopeVisualizer);

pub struct Inner {
    audio_service: AudioService,
    settings_open: AtomicBool,
    data: Arc<RwLock<WaveSyncAppData>>,
}

impl VectorscopeVisualizer {
    pub fn new(audio_service: AudioService, data: Arc<RwLock<WaveSyncAppData>>) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            settings_open: Default::default(),
            data,
        }))
    }
}

impl Visualizer for VectorscopeVisualizer {
    fn get_plot_data(&self) -> PlotData {
        let x_axis = Axis::linear(-1.0, 1.0);
        let y_axis = Axis::linear(-1.0, 1.0);
        PlotData::from_axis(x_axis, y_axis)
    }

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> PaintCallback {
        egui_wgpu::Callback::new_paint_callback(
            rect,
            VectorscopeVisualizerCallback {
                visualizer: self.clone(),
                color: visuals.wave_color(),
                rect,
            },
        )
    }
}

struct VectorscopeVisualizerCallback {
    visualizer: VectorscopeVisualizer,
    color: egui::Color32,
    rect: Rect,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    decay_factor: f32,
    write_factor: f32,
    fill_color: [f32; 4],
    _padding: u64
}

define_resource!(VectorscopeLinePipeline, wgpu::RenderPipeline);
define_resource!(VectorscopeBlitPipeline, wgpu::RenderPipeline);
define_resource!(VectorscopeVertexBuffer, wgpu::Buffer);
define_resource!(VectorscopeUniformBuffer, wgpu::Buffer);
define_resource!(VectorscopeBindGroup, wgpu::BindGroup);

impl CallbackTrait for VectorscopeVisualizerCallback {
    fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        if resources.get::<VectorscopeLinePipeline>().is_none() {
            let width = self.rect.width() as u32;
            let height = self.rect.height() as u32;
            resources.insert(queue.clone());

            let line_snatch_shader =
                create_shader!(device, "decay", "../../../shader/vectorscope_lines.wgsl");

            // let line_blit_shader =
            //     create_shader!(device, "lblit", "../../../shader/vectorscope_blit.wgsl");

            let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vectorscope_vertex_buffer"),
                size: ((MAX_LINE_SEGMENTS + 1) * size_of::<f32>() * 2) as BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vectorscope uniform buffer"),
                size: size_of::<Uniforms>() as BufferAddress,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let aux_tex_desc = wgpu::TextureDescriptor {
                label: Some("vectorscope_intensity_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TextureFormat::R32Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            };
            let tex = device.create_texture(&aux_tex_desc);
            let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_ty = wgpu::BindingType::StorageTexture {
                access: StorageTextureAccess::ReadWrite,
                format: TextureFormat::R32Float,
                view_dimension: TextureViewDimension::D2,
            };

            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: bind_ty,
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                    label: Some("vectorscope_storage_texture_layout"),
                });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vectorscope_storage_tx_bind_group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&tex_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                ],
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("vectorscope layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            resources.insert(VectorscopeUniformBuffer(uniform_buffer));
            resources.insert(VectorscopeVertexBuffer(vertex_buffer));
            resources.insert(VectorscopeBindGroup(bind_group));
            resources.insert(VectorscopeLinePipeline(create_pipeline(
                device,
                &line_snatch_shader,
                &pipeline_layout,
                PrimitiveTopology::LineStrip,
                &[VERTEX_2D_BUFFER_LAYOUT],
                "vectorscope_line_pipeline",
            )));
        }
        vec![]
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let line_pipeline = callback_resources.get::<VectorscopeLinePipeline>().unwrap();
        // let blit_pipeline = callback_resources.get::<VectorscopeBlitPipeline>().unwrap();
        let vertex_buffer = callback_resources.get::<VectorscopeVertexBuffer>().unwrap();
        let uniform_buffer = callback_resources
            .get::<VectorscopeUniformBuffer>()
            .unwrap();
        let bind_group = callback_resources.get::<VectorscopeBindGroup>().unwrap();
        let queue = callback_resources.get::<Queue>().unwrap();
        let audio_service = &self.visualizer.audio_service;

        if audio_service.get_active_audio_channels() < 2 {
            return;
        }

        let to_read = 1000;
        let left_data = audio_service.get_samples(AudioChannel::Left, to_read);
        let right_data = audio_service.get_samples(AudioChannel::Right, to_read);

        if (left_data.len() != to_read) || (right_data.len() != to_read) {
            return;
        }

        let mut vertex_data = vec![];
        for i in 0..to_read {
            vertex_data.push([left_data[i], right_data[i]]);
        }

        queue.write_buffer(
            uniform_buffer,
            0,
            bytemuck::bytes_of(&Uniforms {
                decay_factor: 0.98,
                write_factor: 1.0,
                fill_color: self.color.to_normalized_gamma_f32(),
                _padding: 0,
            }),
        );
        queue.write_buffer(vertex_buffer, 0, bytemuck::cast_slice(&vertex_data));
        render_pass.set_pipeline(line_pipeline);
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_bind_group(0, &bind_group.0, &[]);
        render_pass.draw(0..vertex_data.len() as u32, 0..(vertex_data.len() as u32 - 1));



    }
}
