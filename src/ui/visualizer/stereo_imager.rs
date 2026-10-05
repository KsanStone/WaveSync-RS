use crate::{
    deref_arc, impl_settings,
    sound::{
        AudioChannel,
        audio_service::{self, AudioService},
        scale_to_db,
        smoothing::{FloatArraySmoother, multiplicative_smoother::MultiplicativeSmoother},
    },
    ui::{
        VERTEX_2D_BUFFER_LAYOUT, create_pipeline,
        plot::{Axis, PlotData},
        uniform_bindings,
        visualizer::visualizer_widget::{PostEquiRender, Visualizer},
    },
    wavesync::{WaveSyncAppData, WaveSyncVisuals},
};
use egui::Ui;
use egui::{Color32, PaintCallback, PaintCallbackInfo, Rect, Slider};
use egui_wgpu::{
    CallbackTrait,
    wgpu::{self, BufferAddress, util::DeviceExt},
};
use std::{
    f32::consts::SQRT_2,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64},
    },
    time::Instant,
};
use std::{ops::Add, sync::atomic::Ordering};

deref_arc!(StereoImagerVisualizer);

const DIRECTION_BINS: usize = 128;
const MAX_SAMPLES_TO_READ_PER_FRAME: usize = 8192;

pub struct Inner {
    audio_service: AudioService,
    settings_open: AtomicBool,
    data: Arc<RwLock<WaveSyncAppData>>,
    last_written: AtomicU64,
    last_draw: Mutex<Instant>,
    render_resources: Mutex<Option<RenderResources>>,
    smoother: Mutex<Option<Box<dyn FloatArraySmoother>>>,
}

struct RenderResources {
    queue: wgpu::Queue,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl StereoImagerVisualizer {
    pub fn new(audio_service: AudioService, data: Arc<RwLock<WaveSyncAppData>>) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            settings_open: Default::default(),
            data,
            last_written: Default::default(),
            last_draw: Mutex::new(Instant::now()),
            smoother: Mutex::new(Some(Box::new(MultiplicativeSmoother::new()))),
            render_resources: Default::default(),
        }))
    }
}

impl Visualizer for StereoImagerVisualizer {
    fn get_plot_data(&self) -> PlotData {
        let x_axis = Axis::linear(-1.0, 1.0).always_show_zero(true);
        let y_axis = Axis::linear(0.0, 1.0).always_show_zero(true);
        PlotData::from_axis(x_axis, y_axis).y_axis_shown(false).x_axis_grid_lines_shown(false)
    }

    fn error_message(&self) -> Option<String> {
        if self.audio_service.get_active_audio_channels() < 2 {
            Some("StereoImager requires at least 2 channels".into())
        } else {
            None
        }
    }

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> Option<PaintCallback> {
        Some(egui_wgpu::Callback::new_paint_callback(
            rect,
            StereoImagerVisualizerCallback {
                visualizer: self.clone(),
                color: visuals.wave_color(),
            },
        ))
    }

    impl_settings!("StereoImager Settings", ui, this, {});
}

struct StereoImagerVisualizerCallback {
    visualizer: StereoImagerVisualizer,
    color: Color32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    color: [f32; 4],
}

impl CallbackTrait for StereoImagerVisualizerCallback {
    fn prepare(
        &self,
        device: &egui_wgpu::wgpu::Device,
        queue: &egui_wgpu::wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut egui_wgpu::wgpu::CommandEncoder,
        _callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<egui_wgpu::wgpu::CommandBuffer> {
        let mut resources = self.visualizer.render_resources.lock().unwrap();

        if resources.is_none() {
            let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("stereo_imager_vertex_buffer"),
                size: ((DIRECTION_BINS * 2) * size_of::<f32>() * 2) as BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("line shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../../../shader/colored_line.wgsl").into(),
                ),
            });

            let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("waveform_uniform_buffer"),
                contents: bytemuck::cast_slice(&[Uniforms {
                    color: [1.0, 0.0, 1.0, 1.0],
                }]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let (bind_group_layout, bind_group) =
                uniform_bindings(device, 0, &uniform_buffer, "waveform");

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("waveform layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            *resources = Some(RenderResources {
                queue: queue.clone(),
                vertex_buffer,
                uniform_buffer,
                bind_group,
                pipeline: create_pipeline(
                    device,
                    &shader,
                    &pipeline_layout,
                    wgpu::PrimitiveTopology::LineStrip,
                    &[VERTEX_2D_BUFFER_LAYOUT],
                    "stereo_imager_pipeline",
                ),
            });
        }

        vec![]
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut egui_wgpu::wgpu::RenderPass<'static>,
        _callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let resources = self.visualizer.render_resources.lock().unwrap();
        let plot_data = self.visualizer.get_plot_data();
        if let Some(resources) = resources.as_ref() {
            let last_written = self.visualizer.last_written.load(Ordering::Relaxed);
            let written = self.visualizer.audio_service.get_samples_written();
            let queue = &resources.queue;
            let buffer = &resources.vertex_buffer;
            let uniform_buffer = &resources.uniform_buffer;
            let bind_group = &resources.bind_group;
            let pipeline = &resources.pipeline;

            let to_read = written
                .saturating_sub(last_written)
                .min(MAX_SAMPLES_TO_READ_PER_FRAME as u64);
            self.visualizer
                .last_written
                .store(written, Ordering::Relaxed);

            if self.visualizer.audio_service.get_active_audio_channels() == 1 {
                return;
            }

            let mut l_samples = self
                .visualizer
                .audio_service
                .get_samples(AudioChannel::Left, to_read as usize);
            let mut r_samples = self
                .visualizer
                .audio_service
                .get_samples(AudioChannel::Right, to_read as usize);
            let actual_read = l_samples.len().min(r_samples.len());

            // Rotate the samples by 45deg
            for i in 0..actual_read {
                let l = l_samples[i];
                let r = r_samples[i];
                l_samples[i] = (l + r) / SQRT_2; // vertical
                r_samples[i] = (l - r) / SQRT_2; // horizontal
            }

            // Convert to polar coordinates
            let mut bin_data = [0.0; DIRECTION_BINS];
            for i in 0..actual_read {
                let mut m = l_samples[i];
                let mut s = r_samples[i];

                if m < 0.0 {
                    // Flip the point back up, so that we achieve a semicircle instead of a full circle
                    m = -m;
                    s = -s;
                }

                let angle = s.atan2(m); // angle in radians
                let magnitude = (m * m + s * s).sqrt(); // magnitude

                // Compute bin index, -pi/2 <= angle <= pi/2, so we need to map it to 0..DIRECTION_BINS
                // and handle fp32 shenanigans by clamping the value to 0..DIRECTION_BINS-1
                let bin_index = ((angle + std::f32::consts::FRAC_PI_2) / std::f32::consts::PI
                    * DIRECTION_BINS as f32)
                    .floor() as usize;
                let bin_index = bin_index.min(DIRECTION_BINS - 1);

                bin_data[bin_index] += magnitude;
            }

            // Maximum value one bin can have if ALL the samples are in that bin
            // The value can technically be bigger, but we care more about the direction than the actual magnitude, so we scale it up a bit
            // to have the magnitude be weasier to read.
            let window_weight = actual_read as f32 * 0.3;
            let window_weight_inverse = 1.0 / window_weight;
            bin_data
                .iter_mut()
                .for_each(|x| *x = (*x * window_weight_inverse).clamp(0.0, 1.0));

            let delta_t = self.visualizer.last_draw.lock().unwrap().elapsed();
            *self.visualizer.last_draw.lock().unwrap() = Instant::now();

            let mut smoother = self.visualizer.smoother.lock().unwrap();
            let smoother = smoother.as_mut().expect("Smoother should be initialized");
            let smooth_data = smoother.smooth_data(delta_t.as_secs_f32(), &bin_data);

            // Draw the bins as a polar line plot.
            let mut vertices = vec![[0.0, 0.0]; DIRECTION_BINS];

            for i in 0..DIRECTION_BINS {
                let angle = (i as f32 / DIRECTION_BINS as f32) * std::f32::consts::PI
                    - std::f32::consts::PI * 2.0;
                const MIN_DB: f32 = 70.0;
                let magnitude =
                    scale_to_db(smooth_data[i]).clamp(-MIN_DB, 0.0).add(MIN_DB) / MIN_DB;
                let x = magnitude * angle.cos();
                let y = magnitude * angle.sin();
                vertices[i] = [
                    plot_data
                        .x_axis
                        .gl_pos(x),
                    plot_data
                        .y_axis
                        .gl_pos(y),
                ];
            }

            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&vertices));
            queue.write_buffer(
                uniform_buffer,
                0,
                bytemuck::bytes_of(&Uniforms {
                    color: self.color.to_normalized_gamma_f32(),
                }),
            );

            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.set_pipeline(pipeline);
            render_pass.draw(0..DIRECTION_BINS as u32, 0..1);
        }
    }
}
