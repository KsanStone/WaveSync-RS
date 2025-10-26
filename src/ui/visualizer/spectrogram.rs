use crate::sound::audio_service::{AudioService, CHANNELS};
use crate::sound::{AudioChannel, scale_to_db};
use crate::ui::gradient::{Gradient, Stop};
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{
    FULL_SCREEN_QUAD, VERTEX_2D_BUFFER_LAYOUT, bind_buff, create_bind_group_with_layout,
    create_pipeline, create_texture, write_1d_texture, write_2d_texture_row,
};
use crate::wavesync::{WaveSyncAppData, WaveSyncVisuals};
use crate::{create_shader, define_resource, deref_arc, impl_settings};
use circular_buffer::CircularBuffer;
use egui::{ComboBox, PaintCallback, PaintCallbackInfo, Rect};
use egui::epaint::Color32;
use egui_wgpu::wgpu;
use wgpu::util::DeviceExt;
use wgpu::{
    BindingResource, BufferAddress, BufferBindingType, CommandBuffer, CommandEncoder, Device,
    Queue, RenderPass,
};
use egui;
use egui::Ui;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use log::info;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// This does not need to be too large, texture sampling will do the rest.
const GRADIENT_LOOKUP_LENGTH: u32 = 1024;
/// Generous resolution claim, if we size the mapping buffer in accordance with this
/// we wont need to resize it every 5 seconds.
/// Its size will be 32kb so that's fine.
const MAX_RESOLUTION: usize = 8192;
/// How many fft results we can cache before we next send them to the gpu
/// if we run out of space, oh well.
const TEMP_FFT_SIZE: usize = 8;

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
}

impl Default for SpectrogramSettings {
    fn default() -> Self {
        Self {
            is_vertical: true,
            shown_duration: Duration::from_secs(10),
        }
    }
}

impl Visualizer for SpectrogramVisualizer {
    fn get_plot_data(&self) -> PlotData {
        let settings = &self.data.read().unwrap().spectrogram_settings;
        let db_axis = Axis::linear(-100.0, 0.0);
        let freq_axis = Axis::logarithmic(12.0, 20000.0);
        if settings.is_vertical {
            PlotData::from_axis(freq_axis, db_axis)
        } else {
            PlotData::from_axis(db_axis, freq_axis)
        }
    }


    fn error_message(&self) -> Option<String> {
        if self.audio_service.get_fft_size() > 2_usize.pow(15) {
            return Some("Spectrogram visualizer does not support FFT sizes larger than 16k".to_string())
        }
        None
    }

    fn accept_fft(&self, fft_data: &[Vec<f32>; CHANNELS], fft_size: usize) {
        if fft_data.len() <= self.channel.get_index() {
            return;
        }

        let mut buffer = self.fft_send_buffer.lock().unwrap();
        if self.fft_buffer_size.load(Ordering::Relaxed) != fft_size {
            buffer.clear();
            // We update the fft size later
            // As of rn it'll be an indicator that it has changed when
            // the Vec<f32>'s size doesn't match the fft size.
        }

        buffer.push_back(fft_data.get(self.channel.get_index()).unwrap().clone());
    }

    fn get_draw_callback(&self, rect: Rect, _visuals: &WaveSyncVisuals) -> Option<PaintCallback> {
        Some(egui_wgpu::Callback::new_paint_callback(
            rect,
            SpectrogramVisualizerCallback::new(self.clone()),
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
    });
}

pub struct SpectrogramVisualizerCallback {
    visualizer: SpectrogramVisualizer,
}

impl SpectrogramVisualizerCallback {
    pub(crate) fn new(visualizer: SpectrogramVisualizer) -> Self {
        Self { visualizer }
    }
}

define_resource!(SpectrogramPipeline, wgpu::RenderPipeline);
define_resource!(SpectrogramUniformBuffer, wgpu::Buffer);
define_resource!(SpectrogramUniformBindGroup, wgpu::BindGroup);
define_resource!(SpectrogramVertexBuffer, wgpu::Buffer);
define_resource!(SpectrogramPosMappingBuffer, wgpu::Buffer);
define_resource!(SpectrogramStorageTexture, wgpu::Texture);
define_resource!(SpectrogramGradientTexture, wgpu::Texture);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub size: [i32; 2],
    pub head_offset: i32,
    pub buffer_size: i32,
    pub is_vertical: i32,
    pub _padding: i32,
}

impl CallbackTrait for SpectrogramVisualizerCallback {
    fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        if resources.get::<SpectrogramPipeline>().is_none()
            || self
                .visualizer
                .do_resize_buffers
                .compare_exchange(true, false, Ordering::Acquire, Ordering::Acquire)
                .is_ok()
        {
            info!("Creating spectrogram pipeline");
            // In case we are re-creating the pipeline (fft-size changed)
            // we'll need to re-fill the gradient texture.
            self.visualizer.current_gradient.lock().unwrap().take();
            resources.insert(queue.clone());
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
                size: (MAX_RESOLUTION * size_of::<i32>()) as BufferAddress,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let (storage_tex, storage_tex_view, storage_tex_bind_ty) = create_texture(
                "spectrogram_texture",
                device,
                self.visualizer.fft_buffer_size.load(Ordering::Acquire) as u32,
                buff_length as u32,
                wgpu::TextureFormat::R32Float,
                false,
            );

            let (gradient_tex, gradient_tex_view, gradient_tex_bind_ty) = create_texture(
                "spectrogram_gradient_texture",
                device,
                GRADIENT_LOOKUP_LENGTH,
                1,
                wgpu::TextureFormat::Rgba32Float,
                false,
            );

            let (bind_group_layout, bind_group) = create_bind_group_with_layout(
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

            resources.insert(SpectrogramPosMappingBuffer(pos_mapping_buffer));
            resources.insert(SpectrogramStorageTexture(storage_tex));
            resources.insert(SpectrogramGradientTexture(gradient_tex));
            resources.insert(SpectrogramVertexBuffer(vertex_buffer));
            resources.insert(SpectrogramUniformBindGroup(bind_group));
            resources.insert(SpectrogramUniformBuffer(uniform_buffer));
            resources.insert(SpectrogramPipeline(create_pipeline(
                device,
                &spectrogram_shader,
                &pipeline_layout,
                wgpu::PrimitiveTopology::TriangleList,
                &[VERTEX_2D_BUFFER_LAYOUT],
                "spectrogram_pipeline",
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
        let pipeline = callback_resources.get::<SpectrogramPipeline>().unwrap();
        let uniform_buffer = callback_resources
            .get::<SpectrogramUniformBuffer>()
            .unwrap();
        let vertex_buffer = callback_resources.get::<SpectrogramVertexBuffer>().unwrap();
        let storage_tex = callback_resources
            .get::<SpectrogramStorageTexture>()
            .unwrap();
        let mapping_buffer = callback_resources
            .get::<SpectrogramPosMappingBuffer>()
            .unwrap();
        let bind_group = callback_resources
            .get::<SpectrogramUniformBindGroup>()
            .unwrap();
        let gradient_tex = callback_resources
            .get::<SpectrogramGradientTexture>()
            .unwrap();
        let queue = callback_resources.get::<Queue>().unwrap();
        let plot_data = self.visualizer.get_plot_data();
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

        let gradient = Gradient::new(vec![
            Stop::new(0.00, Color32::from_rgb(10, 10, 30)), // deep blue-black
            Stop::new(0.15, Color32::from_rgb(0, 30, 120)), // dark blue
            Stop::new(0.35, Color32::from_rgb(0, 180, 255)), // cyan-blue
            Stop::new(0.55, Color32::from_rgb(0, 255, 100)), // greenish
            Stop::new(0.75, Color32::from_rgb(255, 255, 0)), // bright yellow
            Stop::new(0.90, Color32::from_rgb(255, 120, 0)), // orange
            Stop::new(1.00, Color32::from_rgb(255, 0, 30)), // red peak
        ])
        .unwrap();
        if current_gradient.is_none() || *current_gradient.as_ref().unwrap() != gradient {
            let gradient_data = gradient.pre_compute_lookup(GRADIENT_LOOKUP_LENGTH as usize);
            write_1d_texture(queue, gradient_tex, &gradient_data);
            *current_gradient = Some(gradient);
        }

        let mut send_buffer = self.visualizer.fft_send_buffer.lock().unwrap();
        let mut due_to_send = Vec::with_capacity(send_buffer.len());
        while let Some(x) = send_buffer.pop_front() {
            due_to_send.push(x);
        }
        drop(send_buffer); // Do not block the buffer,
        // thus allowing the audio thread to run cleanly.

        let (frequency_axis_spectrogram_width, frequency_axis, db_axis) = if settings.is_vertical {
            (info.viewport.width(), &plot_data.x_axis, &plot_data.y_axis)
        } else {
            (info.viewport.height(), &plot_data.y_axis, &plot_data.x_axis)
        };

        let fft_buffer_length = self.visualizer.fft_buffer_length.load(Ordering::Acquire);

        let mut head = self.visualizer.head_offset.load(Ordering::Relaxed);
        for mut buf in due_to_send {
            if buf.is_empty() {
                continue;
            }

            head = self.visualizer.move_head(fft_buffer_length as u32);
            buf.iter_mut()
                .for_each(|x| *x = db_axis.norm_pos(scale_to_db(*x)));

            if buf.len() == self.visualizer.fft_buffer_size.load(Ordering::Acquire) {
                write_2d_texture_row(queue, storage_tex, &buf, head);
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

        queue.write_buffer(
            uniform_buffer,
            0,
            bytemuck::cast_slice(&[Uniforms {
                size: [info.viewport.width() as i32, info.viewport.height() as i32],
                head_offset: head as i32,
                buffer_size: fft_buffer_length as i32,
                is_vertical: settings.is_vertical as i32,
                _padding: 0,
            }]),
        );

        let mut pos_map = Vec::with_capacity(frequency_axis_spectrogram_width as usize);
        let fft_size = self.visualizer.audio_service.get_fft_size();

        for i in 0..frequency_axis_spectrogram_width as usize {
            let freq = frequency_axis.norm_pos_to_val(i as f32 / frequency_axis_spectrogram_width);
            let position_in_buffer = source.bin_of_frequency(freq, fft_size);
            pos_map.push(position_in_buffer as i32);
        }
        queue.write_buffer(mapping_buffer, 0, bytemuck::cast_slice(&pos_map));

        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_bind_group(0, &bind_group.0, &[]);
        render_pass.set_pipeline(pipeline);
        render_pass.draw(0..6, 0..1);
    }
}

fn compute_buffer_length(
    data: &Arc<RwLock<WaveSyncAppData>>,
    audio_service: &AudioService,
) -> usize {
    let settings = &data.read().unwrap().spectrogram_settings;
    let fft_rate = audio_service.get_fft_rate();
    let shown_duration = settings.shown_duration;
    (shown_duration.as_secs_f32() * fft_rate as f32) as usize
}
