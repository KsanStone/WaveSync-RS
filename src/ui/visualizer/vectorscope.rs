use super::vectorscope_bloom::Bloom;
use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::ui::aa_line::{AaLines, Segment, Target};
use crate::ui::plot::{Axis, PlotData, XYPlotData};
use crate::ui::visualizer::vectorscope::egui::Ui;
use crate::ui::visualizer::visualizer_widget::{PostEquiRender, RenderArgs, Visualizer};
use crate::ui::{FULL_SCREEN_QUAD, VERTEX_2D_BUFFER_LAYOUT, create_pipeline, viewport};
use crate::wavesync::WaveSyncVisuals;
use crate::{WaveSyncAppData, create_shader, deref_arc, impl_settings};
use egui;
use egui::{PaintCallback, PaintCallbackInfo, Rect, Slider};
use egui_wgpu::wgpu;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use wgpu::util::DeviceExt;
use wgpu::{
    BufferAddress, CommandBuffer, CommandEncoder, Device, PrimitiveTopology, Queue, RenderPass,
    StorageTextureAccess, StoreOp, TextureFormat, TextureViewDimension,
};

const MAX_LINE_SEGMENTS: usize = 4096;
const REFERENCE_SAMPLE_RATE: f32 = 48_000.0;
const REFERENCE_ENERGY: f32 = 0.5;

/// Preserve deposited energy when the per-frame geometry budget drops old samples.
/// Account for their age as well, so lowering the frame rate does not raise exposure.
fn retained_sample_compensation(
    total: usize,
    retained: usize,
    sample_rate: f32,
    lambda: f32,
) -> f32 {
    if retained == 0 || total <= retained {
        return 1.0;
    }
    let time_step = lambda / sample_rate.max(1.0);
    (-time_step * total as f32).exp_m1() / (-time_step * retained as f32).exp_m1()
}

deref_arc!(VectorscopeVisualizer);

pub struct Inner {
    audio_service: AudioService,
    settings_open: AtomicBool,
    data: Arc<RwLock<WaveSyncAppData>>,
    last_written: AtomicU64,
    last_draw: Mutex<Instant>,
    render_resources: Mutex<Option<RenderResources>>,
    lines: Mutex<VectorscopeLines>,
    uniform_uploads: Mutex<crate::ui::uniform_upload::UniformUploads>,
}

#[derive(Default)]
struct VectorscopeLines {
    left: Vec<f32>,
    right: Vec<f32>,
    segments: Vec<Segment>,
}

impl VectorscopeVisualizer {
    pub fn new(audio_service: AudioService, data: Arc<RwLock<WaveSyncAppData>>) -> Self {
        let initial_written = audio_service.get_samples_written();
        Self(Arc::new(Inner {
            audio_service,
            settings_open: Default::default(),
            data,
            last_written: AtomicU64::new(initial_written),
            last_draw: Mutex::new(Instant::now()),
            render_resources: Default::default(),
            lines: Default::default(),
            uniform_uploads: Default::default(),
        }))
    }
}

impl Visualizer for VectorscopeVisualizer {
    fn min_canvas_size(&self) -> egui::Vec2 { egui::vec2(48.0, 48.0) }

    fn get_plot_data(&self) -> PlotData {
        let range = self.data.read().unwrap().vectorscope_settings.range;
        let x_axis = Axis::linear(-range, range).always_show_zero(true);
        let y_axis = Axis::linear(-range, range).always_show_zero(true);
        PlotData::XY(XYPlotData::from_axis(x_axis, y_axis))
    }

    fn error_message(&self, canvas_size: egui::Vec2) -> Option<String> {
        if let Some(error) = self.size_error(canvas_size) { return Some(error); }
        if self.audio_service.get_active_audio_channels() < 2 {
            Some("Vectorscope requires at least 2 channels".into())
        } else {
            None
        }
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

    impl_settings!("Vectorscope Settings", ui, this, {
        let settings = &mut this.data.write().unwrap().vectorscope_settings;
        ui.horizontal(|ui| {
            ui.label("Range");
            ui.add(Slider::new(&mut settings.range, 0.1..=2.0));
        });
        ui.horizontal(|ui| {
            ui.label("Brightness");
            ui.add(Slider::new(&mut settings.brightness, 1.0..=100.0).logarithmic(true));
        });
        ui.checkbox(&mut settings.bleed_enabled, "Bloom");
        ui.add_enabled_ui(settings.bleed_enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Bloom strength");
                ui.add(Slider::new(&mut settings.bleed, 0.0..=1.0));
            });
        });
    });
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct VectorscopeSettings {
    pub range: f32,
    pub brightness: f32,
    pub bleed: f32,
    pub bleed_enabled: bool,
}

impl Default for VectorscopeSettings {
    fn default() -> Self {
        Self {
            range: 1.0,
            brightness: 10.0,
            bleed: 0.05,
            bleed_enabled: true,
        }
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
    fill_color: [f32; 4],
    decay_factor: f32,
    write_factor: f32,
    bleed_strength: f32,
    _padding: f32,
}

struct RenderResources {
    line: AaLines,
    bloom: Bloom,
    blit_pipeline: wgpu::RenderPipeline,
    intensity_texture: wgpu::Texture,
    intensity_texture_view: wgpu::TextureView,
    quad_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    blit_bind_group: wgpu::BindGroup,
    compute_bind_group: wgpu::BindGroup,
    decay_pipeline: wgpu::ComputePipeline,
    queue: Queue,
}

impl CallbackTrait for VectorscopeVisualizerCallback {
    fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut CommandEncoder,
        _resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        profiling::scope!("vectorscope.prepare");
        let mut re_size = false;
        let width = (self.rect.width() * screen_descriptor.pixels_per_point)
            .round()
            .max(1.0) as u32;
        let height = (self.rect.height() * screen_descriptor.pixels_per_point)
            .round()
            .max(1.0) as u32;

        let mut resources = self.visualizer.render_resources.lock().unwrap();

        if let Some(resources) = resources.as_ref() {
            let tx = &resources.intensity_texture;
            re_size = tx.width() != width || tx.height() != height;
        }

        if resources.is_none() || re_size {
            let line_blit_shader =
                create_shader!(device, "lblit", "../../../shader/vectorscope_blit.wgsl");

            let decay_shader = create_shader!(device, "decay", "../../../shader/decay.wgsl");

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
            let intensity_texture = device.create_texture(&aux_tex_desc);
            let intensity_texture_view =
                intensity_texture.create_view(&wgpu::TextureViewDescriptor::default());

            let bloom = Bloom::new(device, &intensity_texture_view, width, height);

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
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                    label: Some("vectorscope_storage_texture_layout"),
                });

            let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vectorscope_storage_tx_bind_group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&intensity_texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&bloom.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&bloom.sampler),
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
                        resource: wgpu::BindingResource::TextureView(&intensity_texture_view),
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

            let compute_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Compute Pipeline Layout"),
                    bind_group_layouts: &[&compute_bind_group_layout],
                    push_constant_ranges: &[],
                });

            let decay_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Multiply Pipeline"),
                layout: Some(&compute_pipeline_layout),
                module: &decay_shader,
                entry_point: Some("cs_main"),
                compilation_options: Default::default(),
                cache: None,
            });

            *self.visualizer.uniform_uploads.lock().unwrap() = Default::default();
            *resources = Some(RenderResources {
                line: AaLines::new(device, MAX_LINE_SEGMENTS, Target::Intensity),
                bloom,
                blit_pipeline: create_pipeline(
                    device,
                    &line_blit_shader,
                    &pipeline_layout,
                    PrimitiveTopology::TriangleList,
                    &[VERTEX_2D_BUFFER_LAYOUT],
                    "vectorscope_blit_pipeline",
                ),
                decay_pipeline,
                intensity_texture,
                intensity_texture_view,
                quad_buffer,
                uniform_buffer,
                blit_bind_group,
                compute_bind_group,
                queue: queue.clone(),
            })
        }
        vec![]
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        _render_pass: &mut RenderPass<'static>,
        _callback_resources: &CallbackResources,
    ) {
        profiling::scope!("vectorscope.paint");
        // Noop, we'll do the actual rendering in the post egui render pass,
        // as we require additional render-passes to do the render.
    }
}

impl PostEquiRender for VectorscopeVisualizerCallback {
    fn post_egui_render(&self, args: &mut RenderArgs) {
        profiling::scope!("vectorscope.post_egui_render");
        let resources = self.visualizer.render_resources.lock().unwrap();
        let plot_data = self.visualizer.get_plot_data().expect_xy();
        if let Some(resources) = resources.as_ref() {
            let queue = &resources.queue;
            let audio_service = &self.visualizer.audio_service;
            let last_written = &self.visualizer.last_written;
            let mut last = self.visualizer.last_draw.lock().unwrap();
            let delta_t = last.elapsed().as_secs_f32().clamp(0.00001, 1.0);
            *last = Instant::now();

            let decay: f32 = 0.8;

            let lambda = -decay.ln() * 60.0;
            let decay_factor = (-lambda * delta_t).exp();

            let settings = self
                .visualizer
                .data
                .read()
                .unwrap()
                .vectorscope_settings
                .clone();
            let sample_rate = audio_service.get_source().sample_rate.max(1) as f32;
            let uniform_uploads = self.visualizer.uniform_uploads.lock().unwrap();
            crate::upload_uniform_if_changed!(
                *uniform_uploads,
                "vectorscope.uniform_buffer",
                queue,
                &resources.uniform_buffer,
                bytemuck::bytes_of(&Uniforms {
                    fill_color: self.color.to_normalized_gamma_f32(),
                    decay_factor,
                    write_factor: 1.0,
                    bleed_strength: if settings.bleed_enabled {
                        settings.bleed
                    } else {
                        0.0
                    },
                    _padding: 0.0,
                })
            );

            {
                profiling::scope!("vectorscope.decay_encode");
                // Decay :O
                let mut pass = args
                    .encoder
                    .begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Multiply Pass"),
                        timestamp_writes: None,
                    });
                let image_width = &resources.intensity_texture.width();
                let image_height = &resources.intensity_texture.height();
                pass.set_pipeline(&resources.decay_pipeline);
                pass.set_bind_group(0, &resources.compute_bind_group, &[]);
                pass.dispatch_workgroups(image_width.div_ceil(16), image_height.div_ceil(16), 1);
            }

            {
                profiling::scope!("vectorscope.build_upload_and_draw_lines");
                // Draw new lines
                let mut pass = args.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("R32Float Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &resources.intensity_texture_view,
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
                    return;
                }

                let last = last_written.load(Ordering::Relaxed);
                let curr = audio_service.get_samples_written();
                last_written.store(curr, Ordering::Relaxed);

                // Never turn reopening a hidden scope into an enormous flash.
                let new_samples = curr.saturating_sub(last).min(sample_rate as u64) as usize;
                let retained = new_samples.min(MAX_LINE_SEGMENTS);
                let to_read = retained + 1;
                let exposure = REFERENCE_ENERGY * REFERENCE_SAMPLE_RATE / sample_rate
                    * settings.brightness
                    * retained_sample_compensation(new_samples, retained, sample_rate, lambda);
                let viewport = [
                    resources.intensity_texture.width() as f32,
                    resources.intensity_texture.height() as f32,
                ];
                let mut lines = self.visualizer.lines.lock().unwrap();
                let VectorscopeLines {
                    left: left_data,
                    right: right_data,
                    segments,
                } = &mut *lines;
                audio_service.get_samples_aligned_into(
                    AudioChannel::Left,
                    to_read,
                    0,
                    to_read,
                    left_data,
                );
                audio_service.get_samples_aligned_into(
                    AudioChannel::Right,
                    to_read,
                    0,
                    to_read,
                    right_data,
                );
                if left_data.len() == to_read && right_data.len() == to_read && to_read > 1 {
                    segments.clear();
                    for i in 0..(to_read - 1) {
                        let start = [
                            plot_data.x_axis.gl_pos(left_data[i]),
                            plot_data.y_axis.gl_pos(-right_data[i]),
                        ];
                        let end = [
                            plot_data.x_axis.gl_pos(left_data[i + 1]),
                            plot_data.y_axis.gl_pos(-right_data[i + 1]),
                        ];
                        if start.iter().chain(end.iter()).all(|v| v.is_finite()) {
                            let age = (retained - 1 - i) as f32 / sample_rate;
                            let energy = exposure * (-lambda * age).exp();
                            segments.push(Segment::with_energy(start, end, energy, viewport));
                        }
                    }

                    resources.line.upload(queue, &segments);
                    resources.line.configure(
                        queue,
                        &uniform_uploads,
                        [1.0; 4],
                        [
                            resources.intensity_texture.width() as f32,
                            resources.intensity_texture.height() as f32,
                        ],
                    );
                    resources.line.draw(&mut pass, segments.len());
                }
            }

            if settings.bleed_enabled && settings.bleed > 0.0 {
                resources.bloom.encode(args.encoder);
            }
            {
                profiling::scope!("vectorscope.composite_encode");
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

                viewport(self.rect, args.screen_descriptor, &mut pass);

                pass.set_pipeline(&resources.blit_pipeline);
                pass.set_bind_group(0, &resources.blit_bind_group, &[]);
                pass.set_vertex_buffer(0, resources.quad_buffer.slice(..));
                pass.draw(0..6, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod exposure_tests {
    use super::*;
    #[test]
    fn steady_exposure_is_stable_across_sample_rates_frame_rates_and_budget_limits() {
        let lambda = -0.8f32.ln() * 60.0;
        let expected = REFERENCE_ENERGY * REFERENCE_SAMPLE_RATE / lambda;
        for rate in [22_050.0f32, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for fps in [10.0f32, 30.0, 60.0, 144.0, 240.0] {
                let total = (rate / fps).round() as usize;
                let retained = total.min(MAX_LINE_SEGMENTS);
                let step = -lambda / rate;
                let age_sum = (step * retained as f32).exp_m1() / step.exp_m1();
                let injected = REFERENCE_ENERGY * REFERENCE_SAMPLE_RATE / rate
                    * retained_sample_compensation(total, retained, rate, lambda)
                    * age_sum;
                let steady = injected / -(step * total as f32).exp_m1();
                assert!(
                    (steady / expected - 1.0).abs() < 0.001,
                    "sample rate {rate}, fps {fps}: {steady} vs {expected}"
                );
            }
        }
    }
    #[test]
    fn old_settings_keep_default_brightness_and_bleed() {
        let settings: VectorscopeSettings = ron::from_str("(range: 2.0)").unwrap();
        assert_eq!(settings.range, 2.0);
        let defaults = VectorscopeSettings::default();
        assert_eq!(settings.brightness, defaults.brightness);
        assert_eq!(settings.bleed, defaults.bleed);
        assert!(settings.bleed_enabled);
    }
    #[test]
    fn composite_and_decay_shaders_validate_with_uniform_layout() {
        assert_eq!(size_of::<Uniforms>(), 32);
        for source in [
            include_str!("../../../shader/vectorscope_blit.wgsl"),
            include_str!("../../../shader/decay.wgsl"),
            include_str!("../../../shader/vectorscope_bloom.wgsl"),
        ] {
            let module = wgpu::naga::front::wgsl::parse_str(source).unwrap();
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
