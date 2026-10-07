use crate::{
    deref_arc, impl_settings,
    sound::{
        AudioChannel,
        audio_service::AudioService,
        scale_to_db,
        smoothing::{FloatArraySmoother, multiplicative_smoother::MultiplicativeSmoother},
    },
    ui::{
        VERTEX_2D_BUFFER_LAYOUT, create_pipeline,
        plot::{PlotData, PolarPlotData},
        uniform_bindings,
        visualizer::visualizer_widget::Visualizer,
    },
    wavesync::{WaveSyncAppData, WaveSyncVisuals},
};
use egui::{Color32, PaintCallback, PaintCallbackInfo, Rect};
use egui::{Pos2, Ui};
use egui_wgpu::{
    CallbackTrait,
    wgpu::{self, BufferAddress, util::DeviceExt},
};
use std::{
    f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, SQRT_2},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64},
    },
    time::Instant,
};
use std::{ops::Add, sync::atomic::Ordering};

deref_arc!(StereoImagerVisualizer);

const DIRECTION_BINS: usize = 65;
const MAX_SAMPLES_TO_READ_PER_FRAME: usize = 8192;

pub struct Inner {
    audio_service: AudioService,
    settings_open: AtomicBool,
    #[allow(dead_code)]
    data: Arc<RwLock<WaveSyncAppData>>,
    bin_data: Mutex<[f32; DIRECTION_BINS]>,
    last_written: AtomicU64,
    last_draw: Mutex<Instant>,
    render_resources: Mutex<Option<RenderResources>>,
    smoother: Mutex<Option<Box<dyn FloatArraySmoother>>>,
}

struct RenderResources {
    queue: wgpu::Queue,
    line_vertex_buffer: wgpu::Buffer,
    fill_vertex_buffer: wgpu::Buffer,
    line_uniform_buffer: wgpu::Buffer,
    fill_uniform_buffer: wgpu::Buffer,
    line_bind_group: wgpu::BindGroup,
    fill_bind_group: wgpu::BindGroup,
    line_pipeline: wgpu::RenderPipeline,
    fill_pipeline: wgpu::RenderPipeline,
}

impl StereoImagerVisualizer {
    pub fn new(audio_service: AudioService, data: Arc<RwLock<WaveSyncAppData>>) -> Self {
        let mut smoother = MultiplicativeSmoother::new();
        smoother.set_factor(0.95);
        Self(Arc::new(Inner {
            audio_service,
            settings_open: Default::default(),
            data,
            last_written: Default::default(),
            last_draw: Mutex::new(Instant::now()),
            smoother: Mutex::new(Some(Box::new(smoother))),
            render_resources: Default::default(),
            bin_data: Mutex::new([0.0; DIRECTION_BINS]),
        }))
    }
}

impl Visualizer for StereoImagerVisualizer {
    fn get_plot_data(&self) -> PlotData {
        PlotData::Polar(
            PolarPlotData::new()
                .with_radius(1.0)
                .with_radial_lines(vec![0.0, FRAC_PI_2, FRAC_PI_4, FRAC_PI_2 + FRAC_PI_4, PI])
                .with_viewport(Rect::from_min_max(
                    Pos2::new(-1.0, 0.0),
                    Pos2::new(1.0, 1.0),
                )),
        )
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
                color_start: visuals.color_start(),
                color_end: visuals.color_end(),
            },
        ))
    }

    impl_settings!("StereoImager Settings", ui, this, {});
}

struct StereoImagerVisualizerCallback {
    visualizer: StereoImagerVisualizer,
    color_start: Color32,
    color_end: Color32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct FillUniforms {
    end_color: [f32; 4],
    start_color: [f32; 4],
    gradient_center: [f32; 2],
    gradient_radius: f32,
    _padding: f32, // Padding to make the struct size a multiple of 16 bytes
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

            let fill_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("stereo_imager_fill_buffer"),
                size: ((DIRECTION_BINS * 3) * size_of::<f32>() * 2) as BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("line shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../../../shader/colored_line.wgsl").into(),
                ),
            });

            let grad_fill_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("grad fill shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../../../shader/radial_gradient_fill.wgsl").into(),
                ),
            });

            let line_uniform_buffer =
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("waveform_uniform_buffer"),
                    contents: bytemuck::cast_slice(&[Uniforms {
                        color: [1.0, 0.0, 1.0, 1.0],
                    }]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });

            let fill_uniform_buffer =
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("waveform_uniform_buffer"),
                    contents: bytemuck::cast_slice(&[FillUniforms {
                        end_color: [1.0, 0.0, 1.0, 1.0],
                        start_color: [0.0, 1.0, 0.0, 1.0],
                        gradient_center: [0.5, 0.5],
                        gradient_radius: 0.5,
                        _padding: 0.0,
                    }]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });

            let (line_bind_group_layout, line_bind_group) =
                uniform_bindings(device, 0, &line_uniform_buffer, "waveform");

            let (fill_bind_group_layout, fill_bind_group) =
                uniform_bindings(device, 0, &fill_uniform_buffer, "waveform");

            let line_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("stereoimager line layout"),
                    bind_group_layouts: &[&line_bind_group_layout],
                    push_constant_ranges: &[],
                });

            let fill_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("stereoimager fill layout"),
                    bind_group_layouts: &[&fill_bind_group_layout],
                    push_constant_ranges: &[],
                });

            *resources = Some(RenderResources {
                queue: queue.clone(),
                line_vertex_buffer: vertex_buffer,
                fill_vertex_buffer: fill_buffer,
                line_uniform_buffer,
                fill_uniform_buffer,
                line_bind_group,
                fill_bind_group,
                line_pipeline: create_pipeline(
                    device,
                    &line_shader,
                    &line_pipeline_layout,
                    wgpu::PrimitiveTopology::LineStrip,
                    &[VERTEX_2D_BUFFER_LAYOUT],
                    "stereo_imager_pipeline",
                ),
                fill_pipeline: create_pipeline(
                    device,
                    &grad_fill_shader,
                    &fill_pipeline_layout,
                    wgpu::PrimitiveTopology::TriangleList,
                    &[VERTEX_2D_BUFFER_LAYOUT],
                    "stereo_imager_fill_pipeline",
                ),
            });
        }

        vec![]
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut egui_wgpu::wgpu::RenderPass<'static>,
        _callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let resources = self.visualizer.render_resources.lock().unwrap();
        let plot_data = self.visualizer.get_plot_data().expect_polar();
        if let Some(resources) = resources.as_ref() {
            let last_written = self.visualizer.last_written.load(Ordering::Relaxed);
            let written = self.visualizer.audio_service.get_samples_written();
            let queue = &resources.queue;
            let buffer = &resources.line_vertex_buffer;
            let uniform_buffer = &resources.line_uniform_buffer;
            let bind_group = &resources.line_bind_group;
            let pipeline = &resources.line_pipeline;

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
            let mut bin_data = self.visualizer.bin_data.lock().unwrap();
            if actual_read != 0 {
                bin_data.fill(0.0);
            }

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
                // assuming 3 bins,
                // angle = -pi/2 -> bin_index = 0
                // boundry = -pi/6
                // angle = 0 -> bin_index = 1
                // boundry = pi/6
                // angle = pi/2 -> bin_index = 2

                let bin_index = ((angle + std::f32::consts::FRAC_PI_2) / std::f32::consts::PI
                    * DIRECTION_BINS as f32)
                    .floor() as usize;
                let bin_index = bin_index.min(DIRECTION_BINS - 1);

                bin_data[bin_index] += magnitude;
            }

            if actual_read != 0 {
                // Maximum value one bin can have if ALL the samples are in that bin
                // The value can technically be bigger, but we care more about the direction than the actual magnitude, so we scale it up a bit
                // to have the magnitude be weasier to read.
                let window_weight = actual_read as f32 * 0.3;
                let window_weight_inverse = 1.0 / window_weight;
                bin_data
                    .iter_mut()
                    .for_each(|x| *x = (*x * window_weight_inverse).clamp(0.0, 1.0));
            }

            let delta_t = self.visualizer.last_draw.lock().unwrap().elapsed();
            *self.visualizer.last_draw.lock().unwrap() = Instant::now();

            let mut smoother = self.visualizer.smoother.lock().unwrap();
            let smoother = smoother.as_mut().expect("Smoother should be initialized");
            let smooth_data = smoother.smooth_data(delta_t.as_secs_f32(), &*bin_data);
            // test alternaging pattern of 0.5 and 0.7 with DIRECTION_BINS emlements
            // let smooth_data = [0.5, 0.7].iter().cycle().take(DIRECTION_BINS).cloned().collect::<Vec<f32>>();

            // Compute boundry verts
            // The point is to have a pair of verticies for each bin,
            // aligned exactly as described above in the binning process.
            let gl_center = plot_data.gl_center_pos();
            let mut vertices = vec![[0.0, 0.0]; DIRECTION_BINS * 2];

            for i in 0..DIRECTION_BINS {
                let angle_bin_start = (i as f32 / DIRECTION_BINS as f32) * PI;
                let angle_bin_end = ((i + 1) as f32 / DIRECTION_BINS as f32) * PI;
                
                const MIN_DB: f32 = 70.0;
                let magnitude =
                    scale_to_db(smooth_data[i]).clamp(-MIN_DB, 0.0).add(MIN_DB) / MIN_DB;

                let pos_start = plot_data.gl_pos(angle_bin_start, magnitude);
                let pos_end = plot_data.gl_pos(angle_bin_end, magnitude);

                vertices[i * 2] = [pos_start.x, pos_start.y];
                vertices[i * 2 + 1] = [pos_end.x, pos_end.y];
            }

            // Draw the fill as a triangle list.
            let mut tri_vertices = vec![[0.0, 0.0]; DIRECTION_BINS * 3];
            for i in 0..DIRECTION_BINS {
                tri_vertices[i * 3] = [gl_center.x, gl_center.y];
                tri_vertices[i * 3 + 1] = vertices[2 * i];
                tri_vertices[i * 3 + 2] = vertices[2 * i + 1];
            }

            queue.write_buffer(
                &resources.fill_vertex_buffer,
                0,
                bytemuck::cast_slice(&tri_vertices),
            );
            queue.write_buffer(
                &resources.fill_uniform_buffer,
                0,
                bytemuck::bytes_of(&FillUniforms {
                    end_color: self.color_end.to_normalized_gamma_f32(),
                    start_color: self.color_start.to_normalized_gamma_f32(),
                    gradient_center: [gl_center.x, gl_center.y],
                    gradient_radius: plot_data.gl_radius(plot_data.radius),
                    _padding: 0.0,
                }),
            );

            render_pass.set_bind_group(0, &resources.fill_bind_group, &[]);
            render_pass.set_vertex_buffer(0, resources.fill_vertex_buffer.slice(..));
            render_pass.set_pipeline(&resources.fill_pipeline);
            render_pass.draw(0..tri_vertices.len() as u32, 0..1);

            // Draw the bins as a polar line plot.
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&vertices));
            queue.write_buffer(
                uniform_buffer,
                0,
                bytemuck::bytes_of(&Uniforms {
                    color: self.color_end.to_normalized_gamma_f32(),
                }),
            );

            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.set_pipeline(pipeline);
            render_pass.draw(0..DIRECTION_BINS as u32 * 2, 0..1);
        }
    }
}
