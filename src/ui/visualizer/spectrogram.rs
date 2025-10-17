use crate::sound::audio_service::AudioService;
use crate::sound::{AudioChannel, scale_to_db, bin_of_frequency};
use crate::ui::gradient::{Gradient, Stop};
use crate::ui::plot::{Axis, PlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{
    FULL_SCREEN_QUAD, VERTEX_2D_BUFFER_LAYOUT, bind_buff, create_bind_group_with_layout,
    create_pipeline, create_texture, uniform_bindings, write_1d_texture, write_2d_texture_row,
};
use crate::{WaveSyncAppData, WaveSyncVisuals, create_shader, define_resource, deref_arc};
use eframe::egui::{PaintCallback, PaintCallbackInfo, Rect};
use eframe::epaint::Color32;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use eframe::wgpu::{
    BindingResource, BufferAddress, BufferBindingType, CommandBuffer, CommandEncoder, Device,
    Queue, RenderPass,
};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use log::{info, warn};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// This does not need to be too large, texture sampling will do the rest.
const GRADIENT_LOOKUP_LENGTH: u32 = 1024;
/// Generous resolution claim, if we size the mapping buffer in accordance with this
/// we wont need to resize it every 5 seconds. Its size will be 32kb so thats fine.
const MAX_RESOLUTION: usize = 8192;

deref_arc!(SpectrogramVisualizer);

pub struct Inner {
    audio_service: AudioService,
    plot_data: Mutex<PlotData>,
    settings_open: AtomicBool,
    channel: AudioChannel,
    data: Arc<RwLock<WaveSyncAppData>>,
    head_offset: AtomicU32,
}

impl SpectrogramVisualizer {
    pub fn new(
        audio_service: AudioService,
        channel: AudioChannel,
        data: Arc<RwLock<WaveSyncAppData>>,
    ) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            channel,
            plot_data: Mutex::new(PlotData::from_axis(
                Axis::logarithmic(12.0, 20000.0),
                Axis::linear(-100.0, 0.0),
            )),
            settings_open: Default::default(),
            data,
            head_offset: Default::default(),
        }))
    }

    fn move_head(&self, buf_size: u32) -> u32 {
        self.head_offset.fetch_add(1, Ordering::Relaxed) % buf_size
    }
}

impl Visualizer for SpectrogramVisualizer {
    fn get_plot_data(&self) -> PlotData {
        self.plot_data.lock().unwrap().clone()
    }

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> PaintCallback {
        egui_wgpu::Callback::new_paint_callback(
            rect,
            SpectrogramVisualizerCallback::new(self.clone()),
        )
    }
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
        if resources.get::<SpectrogramPipeline>().is_none() {
            info!("Creating spectrogram pipeline");
            resources.insert(queue.clone());

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
                8192,
                600,
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
        let plot_data = self.visualizer.plot_data.lock().unwrap();
        let source = self.visualizer.audio_service.get_source();

        let gradient = Gradient::new(vec![
            Stop::new(0.00, Color32::from_rgb(10, 10, 30)),    // deep blue-black
            Stop::new(0.15, Color32::from_rgb(0, 30, 120)),    // dark blue
            Stop::new(0.35, Color32::from_rgb(0, 180, 255)),   // cyan-blue
            Stop::new(0.55, Color32::from_rgb(0, 255, 100)),   // greenish
            Stop::new(0.75, Color32::from_rgb(255, 255, 0)),   // bright yellow
            Stop::new(0.90, Color32::from_rgb(255, 120, 0)),   // orange
            Stop::new(1.00, Color32::from_rgb(255, 0, 30)),    // red peak
        ])
        .unwrap();
        let gradient_data = gradient.pre_compute_lookup(GRADIENT_LOOKUP_LENGTH as usize);
        write_1d_texture(queue, gradient_tex, &gradient_data);

        let head = self.visualizer.move_head(600);
        let fft_row = self
            .visualizer
            .audio_service
            .get_fft(self.visualizer.channel)
            .into_iter()
            .map(|x| plot_data.y_axis.norm_pos(scale_to_db(x)))
            .collect::<Vec<_>>();

        if fft_row.len() == 8192 {
            write_2d_texture_row(
                queue,
                storage_tex,
                &fft_row,
                head,
            );
        } else if fft_row.len() != 0 {
            warn!(
                "FFT row length {} is not 8192, TODO implement proper resizing",
                fft_row.len()
            );
        }

        queue.write_buffer(
            uniform_buffer,
            0,
            bytemuck::cast_slice(&[Uniforms {
                size: [info.viewport.width() as i32, info.viewport.height() as i32],
                head_offset: head as i32,
                buffer_size: 600,
                is_vertical: 0,
                _padding: 0,
            }]),
        );

        let mut pos_map = Vec::with_capacity(info.viewport.height() as usize);
        for i in 0..info.viewport.height() as usize {
            let freq = plot_data.x_axis.norm_pos_to_val(i as f32 / info.viewport.height());
            let position_in_buffer = source.bin_of_frequency(freq, self.visualizer.audio_service.get_fft_size());
            pos_map.push(position_in_buffer as i32);
        }
        queue.write_buffer(mapping_buffer, 0, bytemuck::cast_slice(&pos_map));

        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_bind_group(0, &bind_group.0, &[]);
        render_pass.set_pipeline(&pipeline);
        render_pass.draw(0..6, 0..1);
    }
}
