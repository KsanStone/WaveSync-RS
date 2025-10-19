use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::ui::{create_pipeline, VERTEX_2D_BUFFER_LAYOUT};
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::{WaveSyncAppData, WaveSyncVisuals, create_shader, define_resource, deref_arc};
use eframe::egui::{PaintCallback, PaintCallbackInfo, Rect};
use eframe::wgpu::{BufferAddress, CommandBuffer, CommandEncoder, Device, PrimitiveTopology, Queue, RenderPass, StorageTextureAccess, TextureFormat, TextureViewDimension};
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

define_resource!(VectorscopeLinePipeline, wgpu::RenderPipeline);
define_resource!(VectorscopeBlitPipeline, wgpu::RenderPipeline);
define_resource!(VectorscopeVertexBuffer, wgpu::Buffer);
define_resource!(VectorscopeBindGroup, wgpu::BindGroup);

impl CallbackTrait for VectorscopeVisualizerCallback {
    fn prepare(
        &self,
        device: &Device,
        _queue: &Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        if resources.get::<VectorscopeLinePipeline>().is_none() {
            let width = self.rect.width() as u32;
            let height = self.rect.height() as u32;

            let vectorscope_shader =
                create_shader!(device, "decay", "../../../shader/vectorscope.wgsl");

            let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vectorscope_vertex_buffer"),
                size: ((MAX_LINE_SEGMENTS + 1) * size_of::<f32>() * 2) as BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            
            let render_target_desc = wgpu::TextureDescriptor {
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
            let tex = device.create_texture(&render_target_desc);
            let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_ty = wgpu::BindingType::StorageTexture {
                access: StorageTextureAccess::ReadWrite,
                format: TextureFormat::R32Float,
                view_dimension: TextureViewDimension::D2,
            };

            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: bind_ty,
                        count: None,
                    }],
                    label: Some("vectorscope_storage_texture_layout"),
                });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vectorscope_storage_tx_bind_group"),
                layout: &bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&tex_view),
                }],
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("vectorscope layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            resources.insert(VectorscopeVertexBuffer(vertex_buffer));
            resources.insert(VectorscopeBindGroup(bind_group));
            resources.insert(VectorscopeLinePipeline(create_pipeline(
                device,
                &vectorscope_shader,
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
    }
}
