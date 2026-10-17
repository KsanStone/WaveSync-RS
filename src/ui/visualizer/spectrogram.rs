use crate::sound::audio_service::{AudioService, CHANNELS};
use crate::sound::{AudioChannel, scale_to_db};
use crate::ui::gradient::Gradient;
use crate::ui::plot::{Axis, PlotData, XYPlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{
    FULL_SCREEN_QUAD, VERTEX_2D_BUFFER_LAYOUT, bind_buff, create_bind_group_with_layout,
    create_pipeline, create_texture, freq_spectrum_select, log_axis_sel, write_1d_texture,
    write_2d_texture_row,
};
use crate::wavesync::{WaveSyncAppData, WaveSyncVisuals};
use crate::{create_shader, deref_arc, impl_settings};
use circular_buffer::CircularBuffer;
use egui;
use egui::Ui;
use egui::{ComboBox, PaintCallback, PaintCallbackInfo, Rect};
use egui_wgpu::wgpu;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;
use wgpu::{
    BindingResource, BufferAddress, BufferBindingType, CommandBuffer, CommandEncoder, Device,
    Queue, RenderPass,
};

/// This does not need to be too large, texture sampling will do the rest.
const GRADIENT_LOOKUP_LENGTH: u32 = 1024;
/// Generous resolution claim, if we size the mapping buffer in accordance with this
/// we wont need to resize it every 5 seconds.
/// Its size will be 32kb so that's fine.
const MAX_RESOLUTION: usize = 8192;
/// How many fft results we can cache before we next send them to the gpu
/// if we run out of space, oh well.
const TEMP_FFT_SIZE: usize = 64;
const GUARD_ROWS: usize = 4;

deref_arc!(SpectrogramVisualizer);

pub struct Inner {
    audio_service: AudioService,
    settings_open: AtomicBool,
    channel: AudioChannel,
    data: Arc<RwLock<WaveSyncAppData>>,
    head_offset: AtomicU32,
    current_gradient: Mutex<Option<Gradient>>,
    fft_send_buffer: Mutex<Box<CircularBuffer<TEMP_FFT_SIZE, Vec<f32>>>>,
    /// Size (freq axis "width") of the current FFT buffer.
    fft_buffer_size: AtomicUsize,
    /// Size (time axis "height") of the current FFT buffer.
    fft_buffer_length: AtomicUsize,
    /// Whether we should resize the FFT buffer in the next frame.
    do_resize_buffers: AtomicBool,
    rendering_resources: Mutex<Option<RenderResources>>,
    scratch: Mutex<SpectrogramScratch>,
}

#[derive(PartialEq, Eq)]
struct SpectrogramMappingKey {
    sample_rate: u32,
    fft_size: usize,
    axis: crate::ui::vec_cache::AxisMappingKey,
    axis_extent_bits: u32,
}

#[derive(Default)]
struct SpectrogramScratch {
    uniform_uploads: crate::ui::uniform_upload::UniformUploads,
    mapping: crate::ui::vec_cache::CachedVec<SpectrogramMappingKey, i32>,
    due_to_send: Vec<Vec<f32>>,
    scroll: super::waterfall_scroll::WaterfallScroll,
    last_paint: Option<Instant>,
}

impl SpectrogramVisualizer {
    pub fn new(
        audio_service: AudioService,
        channel: AudioChannel,
        data: Arc<RwLock<WaveSyncAppData>>,
    ) -> Self {
        let buff_length = compute_buffer_length(&data, &audio_service);
        Self(Arc::new(Inner {
            fft_buffer_size: AtomicUsize::new(audio_service.get_fft_size() / 2),
            fft_buffer_length: AtomicUsize::new(buff_length),
            audio_service,
            channel,
            settings_open: Default::default(),
            data,
            head_offset: Default::default(),
            current_gradient: Mutex::new(None),
            fft_send_buffer: Mutex::new(CircularBuffer::boxed()),
            do_resize_buffers: Default::default(),
            rendering_resources: Default::default(),
            scratch: Default::default(),
        }))
    }

    fn move_head(&self, buf_size: u32) -> u32 {
        let mut val = self.head_offset.load(Ordering::Relaxed);
        val = (val.wrapping_add(1)) % buf_size;
        self.head_offset.store(val, Ordering::Relaxed);
        val
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SpectrogramSettings {
    pub is_vertical: bool,
    pub shown_duration: Duration,
    pub freq_min: u32,
    pub freq_max: u32,
    pub frequency_axis_logarithmic: bool,
}

impl Default for SpectrogramSettings {
    fn default() -> Self {
        Self {
            is_vertical: true,
            shown_duration: Duration::from_secs(10),
            freq_min: 20,
            freq_max: 24000,
            frequency_axis_logarithmic: true,
        }
    }
}

impl Visualizer for SpectrogramVisualizer {
    fn min_canvas_size(&self) -> egui::Vec2 { egui::vec2(64.0, 24.0) }

    fn get_plot_data(&self) -> PlotData {
        let settings = &self.data.read().unwrap().spectrogram_settings;
        let db_axis = Axis::linear(-100.0, 0.0);
        let mut freq_axis = Axis::linear(settings.freq_min as f32, settings.freq_max as f32);
        freq_axis.logarithmic = settings.frequency_axis_logarithmic;
        if settings.is_vertical {
            PlotData::XY(XYPlotData::from_axis(freq_axis, db_axis))
        } else {
            PlotData::XY(XYPlotData::from_axis(db_axis, freq_axis))
        }
    }

    fn accept_fft(&self, fft_data: &[Vec<f32>; CHANNELS], _fft_size: usize) {
        if fft_data.len() <= self.channel.get_index() {
            return;
        }

        let mut buffer = self.fft_send_buffer.lock().unwrap();
        let values = &fft_data[self.channel.get_index()];
        enqueue_fft_row(
            &mut buffer,
            &self.fft_buffer_size,
            &self.do_resize_buffers,
            values,
        );
    }

    fn error_message(&self, canvas_size: egui::Vec2) -> Option<String> {
        if let Some(error) = self.size_error(canvas_size) { return Some(error); }
        if self.audio_service.get_fft_size() > 2_usize.pow(19) {
            return Some(
                "Spectrogram visualizer does not support FFT sizes larger than 65k".to_string(),
            );
        }
        None
    }

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> Option<PaintCallback> {
        Some(egui_wgpu::Callback::new_paint_callback(
            rect,
            SpectrogramVisualizerCallback::new(self.clone(), visuals.spectrogram_gradient()),
        ))
    }

    impl_settings!("Spectrogram Settings", ui, this, {
        let settings = &mut this.data.write().unwrap().spectrogram_settings;

        ui.horizontal(|ui| {
            ui.label("Orientation");

            ComboBox::from_id_salt("spectrogram_orientation")
                .selected_text(if settings.is_vertical {
                    "Vertical"
                } else {
                    "Horizontal"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut settings.is_vertical, true, "Vertical");
                    ui.selectable_value(&mut settings.is_vertical, false, "Horizontal");
                });
        });

        ui.horizontal(|ui| {
            ui.label("Frequency range:");
            let f_max = this.audio_service.get_max_freq();
            freq_spectrum_select(ui, &mut settings.freq_min, &mut settings.freq_max, f_max);
        });

        ui.horizontal(|ui| {
            ui.label("Frequency axis: ");
            log_axis_sel(ui, &mut settings.frequency_axis_logarithmic);
        });
    });
}

pub struct SpectrogramVisualizerCallback {
    visualizer: SpectrogramVisualizer,
    gradient: Gradient,
}

impl SpectrogramVisualizerCallback {
    pub(crate) fn new(visualizer: SpectrogramVisualizer, gradient: Gradient) -> Self {
        Self {
            visualizer,
            gradient,
        }
    }
}

struct RenderResources {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    pos_mapping_buffer: wgpu::Buffer,
    storage_texture: wgpu::Texture,
    gradient_texture: wgpu::Texture,
    queue: Queue,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub width: i32,
    pub height: i32,
    pub head_offset: i32,
    pub buffer_size: i32,
    pub is_vertical: i32,
    pub scroll_offset: f32,
}

impl CallbackTrait for SpectrogramVisualizerCallback {
    fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut CommandEncoder,
        _resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        profiling::scope!("spectrogram.prepare");
        let mut resources = self.visualizer.rendering_resources.lock().unwrap();

        let resize = self
            .visualizer
            .do_resize_buffers
            .swap(false, Ordering::AcqRel);
        if resources.is_none() || resize {
            // In case we are re-creating the pipeline (fft-size changed)
            // we'll need to re-fill the gradient texture.
            self.visualizer.current_gradient.lock().unwrap().take();
            {
                let mut scratch = self.visualizer.scratch.lock().unwrap();
                scratch.mapping.invalidate();
                scratch.scroll = Default::default();
                scratch.last_paint = None;
            }
            self.visualizer.head_offset.store(0, Ordering::Release);
            let buff_length =
                compute_buffer_length(&self.visualizer.data, &self.visualizer.audio_service);
            self.visualizer
                .fft_buffer_length
                .store(buff_length, Ordering::Release);

            let spectrogram_shader =
                create_shader!(device, "spectrogram", "../../../shader/spectrogram.wgsl");

            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spectrogram_uniform_buffer"),
                size: size_of::<Uniforms>() as BufferAddress,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("spectrogram_vertex_buffer"),
                contents: bytemuck::cast_slice(&[FULL_SCREEN_QUAD]),
                usage: wgpu::BufferUsages::VERTEX,
            });

            let pos_mapping_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pos_mapping_buffer"),
                size: ((MAX_RESOLUTION + 1) * size_of::<i32>()) as BufferAddress,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let (storage_texture, storage_tex_view, storage_tex_bind_ty) = create_texture(
                "spectrogram_texture",
                device,
                self.visualizer.fft_buffer_size.load(Ordering::Acquire) as u32 / 4,
                buff_length as u32,
                wgpu::TextureFormat::Rgba8Unorm,
                false,
            );

            let (gradient_texture, gradient_tex_view, gradient_tex_bind_ty) = create_texture(
                "spectrogram_gradient_texture",
                device,
                GRADIENT_LOOKUP_LENGTH,
                1,
                wgpu::TextureFormat::Rgba8Unorm,
                false,
            );

            let (bind_group_layout, uniform_bind_group) = create_bind_group_with_layout(
                device,
                &[
                    (
                        0,
                        &bind_buff(BufferBindingType::Uniform),
                        uniform_buffer.as_entire_binding(),
                    ),
                    (
                        1,
                        &storage_tex_bind_ty,
                        BindingResource::TextureView(&storage_tex_view),
                    ),
                    (
                        2,
                        &gradient_tex_bind_ty,
                        BindingResource::TextureView(&gradient_tex_view),
                    ),
                    (
                        3,
                        &bind_buff(BufferBindingType::Storage { read_only: true }),
                        pos_mapping_buffer.as_entire_binding(),
                    ),
                ],
                "spectrogram",
            );

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("waveform layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            self.visualizer.scratch.lock().unwrap().uniform_uploads = Default::default();
            *resources = Some(RenderResources {
                pipeline: create_pipeline(
                    device,
                    &spectrogram_shader,
                    &pipeline_layout,
                    wgpu::PrimitiveTopology::TriangleList,
                    &[VERTEX_2D_BUFFER_LAYOUT],
                    "spectrogram_pipeline",
                ),
                uniform_buffer,
                uniform_bind_group,
                vertex_buffer,
                pos_mapping_buffer,
                storage_texture,
                gradient_texture,
                queue: queue.clone(),
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
        profiling::scope!("spectrogram.paint");
        let resources = self.visualizer.rendering_resources.lock().unwrap();

        if let Some(resources) = resources.as_ref() {
            let queue = &resources.queue;
            let plot_data = self.visualizer.get_plot_data().expect_xy();
            let source = self.visualizer.audio_service.get_source();
            let mut current_gradient = self.visualizer.current_gradient.lock().unwrap();
            let settings = &self.visualizer.data.read().unwrap().spectrogram_settings;

            let expected_buff_length =
                compute_buffer_length(&self.visualizer.data, &self.visualizer.audio_service);
            if expected_buff_length != self.visualizer.fft_buffer_length.load(Ordering::Relaxed) {
                self.visualizer
                    .do_resize_buffers
                    .store(true, Ordering::Release);
            }

            if current_gradient.is_none() || *current_gradient.as_ref().unwrap() != self.gradient {
                let gradient_data = self
                    .gradient
                    .pre_compute_lookup(GRADIENT_LOOKUP_LENGTH as usize);
                write_1d_texture(queue, &resources.gradient_texture, &gradient_data);
                *current_gradient = Some(self.gradient.clone());
            }

            let mut send_buffer = self.visualizer.fft_send_buffer.lock().unwrap();
            let mut scratch = self.visualizer.scratch.lock().unwrap();
            let SpectrogramScratch {
                uniform_uploads,
                mapping,
                due_to_send,
                scroll,
                last_paint,
            } = &mut *scratch;
            due_to_send.clear();
            while let Some(x) = send_buffer.pop_front() {
                due_to_send.push(x);
            }
            drop(send_buffer); // Do not block the buffer,
            // thus allowing the audio thread to run cleanly.

            let viewport_width = (info.viewport.width() * info.pixels_per_point)
                .round()
                .max(1.0) as usize;
            let viewport_height = (info.viewport.height() * info.pixels_per_point)
                .round()
                .max(1.0) as usize;
            let (frequency_axis_spectrogram_width, frequency_axis, db_axis) =
                if settings.is_vertical {
                    (viewport_width as f32, &plot_data.x_axis, &plot_data.y_axis)
                } else {
                    (viewport_height as f32, &plot_data.y_axis, &plot_data.x_axis)
                };

            let fft_buffer_length = self.visualizer.fft_buffer_length.load(Ordering::Acquire);

            let mut head = self.visualizer.head_offset.load(Ordering::Relaxed);
            let mut uploaded_rows = 0;
            for mut buf in due_to_send.drain(..) {
                if buf.is_empty() {
                    continue;
                }

                buf.iter_mut()
                    .for_each(|x| *x = db_axis.norm_pos(scale_to_db(*x)));

                if buf.len() == resources.storage_texture.width() as usize * 4 {
                    head = self.visualizer.move_head(fft_buffer_length as u32);
                    write_2d_texture_row(queue, &resources.storage_texture, &buf, head);
                    uploaded_rows += 1;
                } else {
                    self.visualizer
                        .fft_buffer_size
                        .store(buf.len(), Ordering::Release);
                    // Resize the FFT buffer on the next frame.
                    // This frame we'll render stale data to avoid a "flash";
                    self.visualizer
                        .do_resize_buffers
                        .store(true, Ordering::Release);
                    break;
                }
            }

            let now = Instant::now();
            let dt = last_paint
                .replace(now)
                .map_or(0.0, |previous| now.duration_since(previous).as_secs_f64());
            let scroll_offset = scroll.advance(
                uploaded_rows,
                dt,
                self.visualizer.audio_service.get_fft_rate() as f64,
            );
            crate::upload_uniform_if_changed!(
                *uniform_uploads,
                "spectrogram.uniform_buffer",
                queue,
                &resources.uniform_buffer,
                bytemuck::cast_slice(&[Uniforms {
                    width: viewport_width as i32,
                    height: viewport_height as i32,
                    head_offset: head as i32,
                    buffer_size: fft_buffer_length as i32,
                    is_vertical: settings.is_vertical as i32,
                    scroll_offset,
                }])
            );

            let fft_size = self.visualizer.audio_service.get_fft_size();
            let key = SpectrogramMappingKey {
                sample_rate: source.sample_rate,
                fft_size,
                axis: frequency_axis.into(),
                axis_extent_bits: frequency_axis_spectrogram_width.to_bits(),
            };
            let mapping_changed = {
                profiling::scope!("spectrogram.mapping_cache");
                mapping.update(
                    key,
                    (frequency_axis_spectrogram_width as usize).min(MAX_RESOLUTION) + 1,
                    |i| {
                        let count = (frequency_axis_spectrogram_width as usize).min(MAX_RESOLUTION);
                        let freq = frequency_axis.norm_pos_to_val(
                            i.min(count.saturating_sub(1)) as f32
                                / count.saturating_sub(1).max(1) as f32,
                        );
                        source.bin_of_frequency(freq, fft_size) as i32
                    },
                )
            };
            if mapping_changed && !mapping.as_slice().is_empty() {
                crate::profile_upload!(
                    "upload.mapping",
                    "spectrogram.pos_mapping_buffer",
                    queue.write_buffer(
                        &resources.pos_mapping_buffer,
                        0,
                        bytemuck::cast_slice(mapping.as_slice()),
                    )
                );
            }

            render_pass.set_vertex_buffer(0, resources.vertex_buffer.slice(..));
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.pipeline);
            render_pass.draw(0..6, 0..1);
        }
    }
}

fn compute_buffer_length(
    data: &Arc<RwLock<WaveSyncAppData>>,
    audio_service: &AudioService,
) -> usize {
    let settings = &data.read().unwrap().spectrogram_settings;
    let fft_rate = audio_service.get_fft_rate();
    let shown_duration = settings.shown_duration;
    ((shown_duration.as_secs_f64() * fft_rate as f64).ceil() as usize).clamp(2, 2048 - GUARD_ROWS)
        + GUARD_ROWS
}

// Compare magnitude counts with magnitude counts, not full transform sizes.
fn enqueue_fft_row(
    buffer: &mut CircularBuffer<TEMP_FFT_SIZE, Vec<f32>>,
    size: &AtomicUsize,
    resize: &AtomicBool,
    values: &[f32],
) {
    if values.is_empty() {
        return;
    }
    if size.swap(values.len(), Ordering::AcqRel) != values.len() {
        buffer.clear();
        resize.store(true, Ordering::Release);
    }
    buffer.push_back(values.to_vec());
}

#[cfg(test)]
mod fft_queue_tests {
    use super::*;
    #[test]
    fn batched_fft_rows_survive_until_render_and_size_changes_flush_old_rows() {
        let mut queue = CircularBuffer::new();
        let size = AtomicUsize::new(4);
        let resize = AtomicBool::new(false);
        for value in 0..3 {
            enqueue_fft_row(&mut queue, &size, &resize, &[value as f32; 4]);
        }
        assert_eq!(queue.len(), 3);
        assert!(!resize.load(Ordering::Acquire));
        assert_eq!(queue.front().unwrap(), &vec![0.0; 4]);
        enqueue_fft_row(&mut queue, &size, &resize, &[4.0; 2]);
        assert_eq!(queue.len(), 1);
        assert!(resize.load(Ordering::Acquire));
        enqueue_fft_row(&mut queue, &size, &resize, &[5.0; 2]);
        assert_eq!(queue.len(), 2);
    }
}
