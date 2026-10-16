use super::extended_waveform_cache::{ColumnCache, SampleScroll, Summary};
use crate::sound::AudioChannel;
use crate::sound::audio_service::AudioService;
use crate::ui::plot::{Axis, PlotData, XYPlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{
    FULL_SCREEN_QUAD, Ui, VERTEX_2D_BUFFER_LAYOUT, bind_buff, create_bind_group_with_layout,
    create_pipeline,
};
use crate::wavesync::{WaveSyncAppData, WaveSyncVisuals};
use crate::{deref_arc, impl_settings};
use egui::{PaintCallback, PaintCallbackInfo, Rect, Slider};
use egui_wgpu::wgpu::util::DeviceExt;
use egui_wgpu::wgpu::{BufferAddress, CommandBuffer, CommandEncoder, Device, Queue, RenderPass};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor, wgpu};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

deref_arc!(ExtendedWaveformVisualizer);

const MAX_RESOLUTION: usize = 8192;
const GUARD_COLUMNS: usize = 256;

pub struct Inner {
    audio_service: AudioService,
    settings_open: AtomicBool,
    channel: AudioChannel,
    data: Arc<RwLock<WaveSyncAppData>>,
    render_resources: Mutex<Option<RenderResources>>,
    waveform_cache: Mutex<WaveformCache>,
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
            waveform_cache: Default::default(),
        }))
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExtendedWaveformVisualizerSettings {
    pub range: f32,
}

impl Default for ExtendedWaveformVisualizerSettings {
    fn default() -> Self {
        Self { range: 1.0 }
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    sample_color: [f32; 4],
    rms_color: [f32; 4],
    viewport: [f32; 2],
    count: u32,
    head: u32,
    scroll: f32,
    range: f32,
    grid_origin: f32,
    guard_columns: u32,
}

impl Visualizer for ExtendedWaveformVisualizer {
    fn get_plot_data(&self) -> PlotData {
        let range = self.data.read().unwrap().extended_waveform_settings.range;
        PlotData::XY(
            XYPlotData::from_axis(
                Axis::linear(-1.0, 0.0),
                Axis::linear(-range, range).always_show_zero(true),
            )
            .x_axis_shown(false),
        )
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
        profiling::scope!("extended_waveform.prepare");
        let mut resources = self.visualizer.render_resources.lock().unwrap();

        if resources.is_none() {
            // Fixed allocations: viewport changes never resize or clear these buffers.
            let columns = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("extended waveform GPU history"),
                size: (MAX_RESOLUTION * size_of::<[f32; 4]>()) as BufferAddress,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("extended waveform quad"),
                contents: bytemuck::cast_slice(&FULL_SCREEN_QUAD),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("extended waveform uniforms"),
                size: size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let (layout, bind_group) = create_bind_group_with_layout(
                device,
                &[
                    (
                        0,
                        &bind_buff(wgpu::BufferBindingType::Uniform),
                        uniforms.as_entire_binding(),
                    ),
                    (
                        1,
                        &bind_buff(wgpu::BufferBindingType::Storage { read_only: true }),
                        columns.as_entire_binding(),
                    ),
                ],
                "extended waveform",
            );
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("extended waveform envelope shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../../../shader/extended_waveform.wgsl").into(),
                ),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("extended waveform layout"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
            let pipeline = create_pipeline(
                device,
                &shader,
                &pipeline_layout,
                wgpu::PrimitiveTopology::TriangleList,
                &[VERTEX_2D_BUFFER_LAYOUT],
                "extended waveform",
            );
            {
                let mut cache = self.visualizer.waveform_cache.lock().unwrap();
                cache.columns.invalidate();
                cache.uniform_uploads = Default::default();
            }
            *resources = Some(RenderResources {
                queue: queue.clone(),
                columns,
                vertices,
                uniforms,
                bind_group,
                pipeline,
            });
        }

        vec![]
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut RenderPass<'static>,
        _callback_resources: &CallbackResources,
    ) {
        profiling::scope!("extended_waveform.paint");
        if let Some(resources) = self.visualizer.render_resources.lock().unwrap().as_ref() {
            let mut cache = self.visualizer.waveform_cache.lock().unwrap();
            let source = self.visualizer.audio_service.get_source();
            let range = self
                .visualizer
                .data
                .read()
                .unwrap()
                .extended_waveform_settings
                .range
                .max(0.0001);
            let (written, size, changed) = {
                let buffer = {
                    profiling::scope!("extended_waveform.audio_lock_wait");
                    self.visualizer.audio_service.audio_buffer.lock().unwrap()
                };
                let buf = &buffer[self.visualizer.channel.get_index()];
                let written = self
                    .visualizer
                    .audio_service
                    .get_channel_samples_written(self.visualizer.channel);
                profiling::scope!("extended_waveform.update_gpu_history");
                let changed = cache
                    .columns
                    .update(MAX_RESOLUTION, buf.len(), written, |range| {
                        profiling::scope!("extended_waveform.scan_new_column");
                        Summary::scan(buf.range(range))
                    });
                (written, buf.len(), changed)
            };
            if cache.columns.len <= GUARD_COLUMNS {
                return;
            }
            if changed {
                let columns = &cache.columns;
                let first_count = columns
                    .pending
                    .len()
                    .min(columns.len - columns.upload_start);
                crate::profile_upload!(
                    "upload.geometry",
                    "extended_waveform.new_columns",
                    resources.queue.write_buffer(
                        &resources.columns,
                        (columns.upload_start * size_of::<[f32; 4]>()) as u64,
                        bytemuck::cast_slice(&columns.pending[..first_count]),
                    )
                );
                if first_count < columns.pending.len() {
                    crate::profile_upload!(
                        "upload.geometry",
                        "extended_waveform.wrapped_columns",
                        resources.queue.write_buffer(
                            &resources.columns,
                            0,
                            bytemuck::cast_slice(&columns.pending[first_count..]),
                        )
                    );
                }
            }
            let key = (size, source.sample_rate);
            if cache.clock_key != Some(key) {
                cache.clock_key = Some(key);
                cache.scroll = Default::default();
                cache.last_paint = None;
            }
            let now = Instant::now();
            let dt = cache
                .last_paint
                .replace(now)
                .map_or(0.0, |last| now.duration_since(last).as_secs_f64());
            let chunk = cache.columns.chunk_size();
            let offset = cache.scroll.offset(
                written,
                chunk,
                source.sample_rate,
                self.visualizer.audio_service.get_last_frame_size(),
                dt,
                GUARD_COLUMNS,
            );
            let viewport_width = (info.viewport.width() * info.pixels_per_point).max(1.0);
            let grid_step =
                ((cache.columns.len - GUARD_COLUMNS - 1) as f64 / viewport_width as f64).max(1.0);
            let grid_origin = ((written / chunk as u64) as f64).rem_euclid(grid_step) as f32;
            crate::upload_uniform_if_changed!(
                cache.uniform_uploads,
                "extended_waveform.uniforms",
                resources.queue,
                &resources.uniforms,
                bytemuck::bytes_of(&Uniforms {
                    sample_color: self.sample_line_color.to_normalized_gamma_f32(),
                    rms_color: self.rms_color.to_normalized_gamma_f32(),
                    viewport: [
                        (info.viewport.width() * info.pixels_per_point).max(1.0),
                        (info.viewport.height() * info.pixels_per_point).max(1.0)
                    ],
                    count: cache.columns.len as u32,
                    head: cache.columns.head as u32,
                    scroll: offset,
                    range,
                    grid_origin,
                    guard_columns: GUARD_COLUMNS as u32,
                })
            );
            render_pass.set_pipeline(&resources.pipeline);
            render_pass.set_bind_group(0, &resources.bind_group, &[]);
            render_pass.set_vertex_buffer(0, resources.vertices.slice(..));
            render_pass.draw(0..6, 0..1);
        }
    }
}

#[derive(Default)]
struct WaveformCache {
    columns: ColumnCache,
    scroll: SampleScroll,
    last_paint: Option<Instant>,
    clock_key: Option<(usize, u32)>,
    uniform_uploads: crate::ui::uniform_upload::UniformUploads,
}
struct RenderResources {
    queue: Queue,
    columns: wgpu::Buffer,
    vertices: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

#[cfg(test)]
mod shader_tests {
    use super::*;
    #[test]
    fn envelope_shader_validates() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!(
            "../../../shader/extended_waveform.wgsl"
        ))
        .unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<Uniforms>(), 64);
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    #[test]
    #[ignore = "requires a GL adapter; run explicitly with --ignored"]
    fn fixed_gpu_ring_wraps_and_partial_updates_preserve_downsampled_peaks() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::GL,
                ..Default::default()
            });
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .unwrap();
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .unwrap();
            device.push_error_scope(wgpu::ErrorFilter::Validation);
            let columns = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (8192 * 16) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let (layout, bind_group) = create_bind_group_with_layout(
                &device,
                &[
                    (
                        0,
                        &bind_buff(wgpu::BufferBindingType::Uniform),
                        uniforms.as_entire_binding(),
                    ),
                    (
                        1,
                        &bind_buff(wgpu::BufferBindingType::Storage { read_only: true }),
                        columns.as_entire_binding(),
                    ),
                ],
                "envelope GPU test",
            );
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../../../shader/extended_waveform.wgsl").into(),
                ),
            });
            let pipeline = create_pipeline(
                &device,
                &shader,
                &pipeline_layout,
                wgpu::PrimitiveTopology::TriangleList,
                &[VERTEX_2D_BUFFER_LAYOUT],
                "envelope GPU test",
            );
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&FULL_SCREEN_QUAD),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 17,
                    height: 33,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256 * 33,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let base = [-0.2f32, 0.2, 0.01, 0.0];
            let mut reference = None;
            let step = 47.0f32 / 17.0;
            let start = 13.0 * step - 39.5;
            let mut temporal_alpha = Vec::new();
            for (head, remove_peak, scroll, origin, peak, temporal) in [
                (0, false, -1.25, 0.0, 38, false),
                (63, false, -1.25, 0.0, 38, false),
                (63, true, -1.25, 0.0, 38, false),
                // Advance the history origin while keeping the display clock
                // fixed: the rendered image must remain exactly the same.
                (1, false, -2.25, 1.0, 37, false),
                (0, false, start, 0.0, 38, true),
                (0, false, start + step * 0.5, 0.0, 38, true),
                (0, false, start + step, 0.0, 38, true),
            ] {
                if !remove_peak {
                    let mut data = [base; 64];
                    data[(head + peak) % 64] = [-0.9, 0.9, 0.09, 0.0];
                    queue.write_buffer(&columns, 0, bytemuck::cast_slice(&data));
                } else {
                    // Modify one GPU slot, without resending or clearing history.
                    queue.write_buffer(
                        &columns,
                        ((head + 38) % 64 * 16) as u64,
                        bytemuck::bytes_of(&base),
                    );
                }
                queue.write_buffer(
                    &uniforms,
                    0,
                    bytemuck::bytes_of(&Uniforms {
                        sample_color: [0.7, 0.4, 0.9, 1.0],
                        rms_color: [0.4, 0.8, 0.9, 1.0],
                        viewport: [17.0, 33.0],
                        count: 64,
                        head: head as u32,
                        scroll,
                        range: 1.0,
                        grid_origin: origin,
                        guard_columns: 16,
                    }),
                );
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &bind_group, &[]);
                    pass.set_vertex_buffer(0, vertices.slice(..));
                    pass.draw(0..6, 0..1);
                }
                encoder.copy_texture_to_buffer(
                    target.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(33),
                        },
                    },
                    wgpu::Extent3d {
                        width: 17,
                        height: 33,
                        depth_or_array_layers: 1,
                    },
                );
                queue.submit(Some(encoder.finish()));
                let (tx, rx) = std::sync::mpsc::channel();
                readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    tx.send(r).unwrap();
                });
                device.poll(wgpu::PollType::Wait).unwrap();
                rx.recv().unwrap().unwrap();
                let mapped = readback.slice(..).get_mapped_range();
                let pixels: Vec<u8> = (0..33)
                    .flat_map(|y| mapped[y * 256..y * 256 + 17 * 4].iter().copied())
                    .collect();
                if temporal {
                    temporal_alpha.push(pixels[(3 * 17 + 8) * 4 + 3] as f32);
                } else if remove_peak {
                    assert_eq!(pixels[(3 * 17 + 8) * 4 + 3], 0);
                } else {
                    assert!(
                        pixels[(3 * 17 + 8) * 4 + 3] > 0,
                        "a one-column transient must survive downsampling"
                    );
                    assert_eq!(pixels[(3 * 17) * 4 + 3], 0);
                    if let Some(previous) = reference.as_ref() {
                        assert_eq!(&pixels, previous, "ring wrap must not alter the image");
                    }
                    reference = Some(pixels);
                }
                drop(mapped);
                readback.unmap();
            }
            assert!(temporal_alpha[0] < 2.0);
            assert!(temporal_alpha[2] > 253.0);
            assert!(
                (temporal_alpha[1] - (temporal_alpha[0] + temporal_alpha[2]) * 0.5).abs() <= 2.0,
                "subpixel phases must crossfade coverage linearly: {temporal_alpha:?}"
            );
            assert!(device.pop_error_scope().await.is_none());
        });
    }
}
