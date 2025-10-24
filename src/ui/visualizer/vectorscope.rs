use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_widget::{PostEquiRender, RenderArgs, Visualizer};
use crate::ui::{
    FULL_SCREEN_QUAD, VERTEX_2D_BUFFER_LAYOUT, create_pipeline, create_pipeline_color,
    uniform_bindings, viewport,
};
use crate::wavesync::WaveSyncVisuals;
use crate::{WaveSyncAppData, create_shader, define_resource, deref_arc};
use eframe::egui::{PaintCallback, PaintCallbackInfo, Rect};
use eframe::wgpu::util::DeviceExt;
use eframe::wgpu::{
    BufferAddress, CommandBuffer, CommandEncoder, Device, PrimitiveTopology, Queue, RenderPass,
    StorageTextureAccess, StoreOp, TextureFormat, TextureViewDimension,
};
use eframe::{egui, wgpu};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use log::warn;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, RwLock};

const MAX_LINE_SEGMENTS: usize = 1000;

deref_arc!(VectorscopeVisualizer);

pub struct Inner {
    audio_service: AudioService,
    settings_open: AtomicBool,
    data: Arc<RwLock<WaveSyncAppData>>,
    last_written: AtomicU64
}

impl VectorscopeVisualizer {
    pub fn new(audio_service: AudioService, data: Arc<RwLock<WaveSyncAppData>>) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            settings_open: Default::default(),
            data,
            last_written: Default::default(),
        }))
    }
}

impl Visualizer for VectorscopeVisualizer {
    fn get_plot_data(&self) -> PlotData {
        let x_axis = Axis::linear(-1.0, 1.0);
        let y_axis = Axis::linear(-1.0, 1.0);
        PlotData::from_axis(x_axis, y_axis)
    }

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> Option<PaintCallback> {
        Some(egui_wgpu::Callback::new_paint_callback(
            rect,
            VectorscopeVisualizerCallback {
                visualizer: self.clone(),
                color: visuals.wave_color(),
                rect,
            },
        ))
    }

    fn get_post_egui_render(
        &self,
        rect: Rect,
        visuals: &WaveSyncVisuals,
    ) -> Option<Box<dyn PostEquiRender>> {
        Some(Box::new(VectorscopeVisualizerCallback {
            visualizer: self.clone(),
            color: visuals.wave_color(),
            rect,
        }))
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
    _padding: u64,
}

define_resource!(VectorscopeLinePipeline, wgpu::RenderPipeline);
define_resource!(VectorscopeBlitPipeline, wgpu::RenderPipeline);
define_resource!(VectorscopeVertexBuffer, wgpu::Buffer);
define_resource!(VectorscopeIntensityTexture, wgpu::Texture);
define_resource!(VectorscopeIntensityTextureView, wgpu::TextureView);
define_resource!(VectorscopeQuadBuffer, wgpu::Buffer);
define_resource!(VectorscopeUniformBuffer, wgpu::Buffer);
define_resource!(VectorscopeBlitBindGroup, wgpu::BindGroup);
define_resource!(VectorscopeLineBindGroup, wgpu::BindGroup);
define_resource!(VectorscopeComputeBindGroup, wgpu::BindGroup);
define_resource!(VectorscopeDecayPipeline, wgpu::ComputePipeline);
define_resource!(VectorscopeVertexInstanceBuffer, wgpu::Buffer);

impl CallbackTrait for VectorscopeVisualizerCallback {
    fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        let mut re_size = false;
        let width = self.rect.width() as u32;
        let height = self.rect.height() as u32;

        if let Some(tx) = resources.get::<VectorscopeIntensityTexture>() {
            re_size = tx.width() != width || tx.height() != height;
        }

        if resources.get::<VectorscopeLinePipeline>().is_none() || re_size {
            resources.insert(queue.clone());

            let line_snatch_shader =
                create_shader!(device, "decay", "../../../shader/vectorscope_lines.wgsl");

            let line_blit_shader =
                create_shader!(device, "lblit", "../../../shader/vectorscope_blit.wgsl");

            let decay_shader = create_shader!(device, "decay", "../../../shader/decay.wgsl");

            let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vectorscope_vertex_buffer"),
                size: ((MAX_LINE_SEGMENTS * 2) * size_of::<f32>() * 2) as BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let vertex_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vectorscope_vertex_instance_buffer"),
                size: ((MAX_LINE_SEGMENTS) * size_of::<f32>()) as BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let quad_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vectorscope_quad_buffer"),
                contents: bytemuck::cast_slice(&[FULL_SCREEN_QUAD]),
                usage: wgpu::BufferUsages::VERTEX,
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
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            };
            let tex = device.create_texture(&aux_tex_desc);
            let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::StorageTexture {
                                access: StorageTextureAccess::ReadOnly,
                                format: TextureFormat::R32Float,
                                view_dimension: TextureViewDimension::D2,
                            },
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

            let compute_bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::StorageTexture {
                                access: StorageTextureAccess::ReadWrite,
                                format: TextureFormat::R32Float,
                                view_dimension: TextureViewDimension::D2,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                    label: Some("vectorscope_compute_layout"),
                });

            let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vectorscope_compute_bind_group"),
                layout: &compute_bind_group_layout,
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

            let (line_bind_group_layout, line_bind_group) =
                uniform_bindings(device, 0, &uniform_buffer, "line uniforms");

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("vectorscope layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            let line_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("vectorscope layout 2"),
                    bind_group_layouts: &[&line_bind_group_layout],
                    push_constant_ranges: &[],
                });

            let compute_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Compute Pipeline Layout"),
                    bind_group_layouts: &[&compute_bind_group_layout],
                    push_constant_ranges: &[],
                });

            let compute_pipeline =
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Multiply Pipeline"),
                    layout: Some(&compute_pipeline_layout),
                    module: &decay_shader,
                    entry_point: Some("cs_main"),
                    compilation_options: Default::default(),
                    cache: None,
                });

            resources.insert(VectorscopeVertexInstanceBuffer(vertex_instance_buffer));
            resources.insert(VectorscopeDecayPipeline(compute_pipeline));
            resources.insert(VectorscopeComputeBindGroup(compute_bind_group));
            resources.insert(VectorscopeLineBindGroup(line_bind_group));
            resources.insert(VectorscopeIntensityTextureView(tex_view));
            resources.insert(VectorscopeIntensityTexture(tex));
            resources.insert(VectorscopeUniformBuffer(uniform_buffer));
            resources.insert(VectorscopeVertexBuffer(vertex_buffer));
            resources.insert(VectorscopeBlitBindGroup(bind_group));
            resources.insert(VectorscopeQuadBuffer(quad_buffer));
            resources.insert(VectorscopeLinePipeline(create_pipeline_color(
                device,
                &line_snatch_shader,
                &line_pipeline_layout,
                PrimitiveTopology::LineList,
                &[VERTEX_2D_BUFFER_LAYOUT, wgpu::VertexBufferLayout {
                    array_stride: size_of::<f32>() as BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32,
                        },
                    ],
                },],
                "vectorscope_line_pipeline",
                TextureFormat::R32Float,
                wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                }
            )));
            resources.insert(VectorscopeBlitPipeline(create_pipeline(
                device,
                &line_blit_shader,
                &pipeline_layout,
                PrimitiveTopology::TriangleList,
                &[VERTEX_2D_BUFFER_LAYOUT],
                "vectorscope_blit_pipeline",
            )));
        }
        vec![]
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        _render_pass: &mut RenderPass<'static>,
        _callback_resources: &CallbackResources,
    ) {
        // Noop, we'll do the actual rendering in the post egui render pass,
        // as we require additional render-passes to do the render.
    }
}

impl PostEquiRender for VectorscopeVisualizerCallback {
    fn post_egui_render(&self, args: &mut RenderArgs) {
        let line_pipeline = args.resources.get::<VectorscopeLinePipeline>().unwrap();
        let blit_pipeline = args.resources.get::<VectorscopeBlitPipeline>().unwrap();
        let quad_buffer = args.resources.get::<VectorscopeQuadBuffer>().unwrap();
        let vertex_buffer = args.resources.get::<VectorscopeVertexBuffer>().unwrap();
        let decay_pipeline = args.resources.get::<VectorscopeDecayPipeline>().unwrap();
        let decay_bind_group = args.resources.get::<VectorscopeComputeBindGroup>().unwrap();
        let intensity_texture = args.resources.get::<VectorscopeIntensityTexture>().unwrap();
        let line_bind_group = args.resources.get::<VectorscopeLineBindGroup>().unwrap();
        let vertex_instance_buffer = args.resources.get::<VectorscopeVertexInstanceBuffer>().unwrap();
        let texture_view = args
            .resources
            .get::<VectorscopeIntensityTextureView>()
            .unwrap();
        let uniform_buffer = args.resources.get::<VectorscopeUniformBuffer>().unwrap();
        let bind_group = args.resources.get::<VectorscopeBlitBindGroup>().unwrap();
        let queue = args.resources.get::<Queue>().unwrap();
        let audio_service = &self.visualizer.audio_service;
        let last_written = &self.visualizer.last_written;

        queue.write_buffer(
            uniform_buffer,
            0,
            bytemuck::bytes_of(&Uniforms {
                decay_factor: 0.67,
                write_factor: 1.0,
                fill_color: self.color.to_normalized_gamma_f32(),
                _padding: 0,
            }),
        );

        {
            // Decay :O
            let mut pass = args
                .encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Multiply Pass"),
                    timestamp_writes: None,
                });
            let image_width = intensity_texture.width();
            let image_height = intensity_texture.height();
            pass.set_pipeline(&decay_pipeline);
            pass.set_bind_group(0, &decay_bind_group.0, &[]);
            pass.dispatch_workgroups((image_width + 15) / 16, (image_height + 15) / 16, 1);
        }

        {
            // Draw new lines
            let mut pass = args.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("R32Float Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            if audio_service.get_active_audio_channels() < 2 {
                warn!("Not enough audio channels to draw vectorscope");
                return;
            }

            let last = last_written.load(std::sync::atomic::Ordering::Relaxed);
            let curr = audio_service.get_samples_written();
            last_written.store(curr, std::sync::atomic::Ordering::Relaxed);

            let to_read = curr.saturating_sub(last).min(MAX_LINE_SEGMENTS as u64) as usize;
            let left_data = audio_service.get_samples(AudioChannel::Left, to_read);
            let right_data = audio_service.get_samples(AudioChannel::Right, to_read);

            if left_data.len() == to_read && right_data.len() == to_read && to_read > 1 {
                let mut vertex_data = vec![];
                let mut lengths = vec![];
                for i in 0..(to_read - 1) {
                    vertex_data.push([left_data[i], right_data[i]]);
                    vertex_data.push([left_data[i + 1], right_data[i + 1]]);

                    let x = left_data[i + 1] - left_data[i];
                    let y = right_data[i + 1] - right_data[i];
                    lengths.push((1.0 / (x*x + y*y).sqrt().max(0.0000001)).min(1.0));
                }


                queue.write_buffer(vertex_buffer, 0, bytemuck::cast_slice(&vertex_data));
                queue.write_buffer(vertex_instance_buffer, 0, bytemuck::cast_slice(&lengths));
                pass.set_pipeline(line_pipeline);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, vertex_instance_buffer.slice(..));
                pass.set_bind_group(0, &line_bind_group.0, &[]);
                pass.draw(
                    0..vertex_data.len() as u32,
                    0..(vertex_data.len() as u32 / 2),
                );
            }
        }

        {
            // Display the luminosity buffer
            let mut pass = args.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: args.window_surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                label: Some("egui main render pass"),
                occlusion_query_set: None,
            });

            viewport(self.rect, &args.screen_descriptor, &mut pass);

            pass.set_pipeline(blit_pipeline);
            pass.set_bind_group(0, &bind_group.0, &[]);
            pass.set_vertex_buffer(0, quad_buffer.slice(..));
            pass.draw(0..6, 0..1);
        }
    }
}
