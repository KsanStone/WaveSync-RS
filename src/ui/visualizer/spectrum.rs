use crate::sound::audio_service::AudioService;
use crate::sound::peaking::{
    PeakComputer, PeakSettings, hold_smooth_falloff::HoldSmoothFalloffPeakComputer,
};
use crate::sound::smoothing::FloatArraySmoother;
use crate::sound::smoothing::exponential_falloff_smoother::ExponentialFalloffSmoother;
use crate::sound::smoothing::multiplicative_smoother::MultiplicativeSmoother;
use crate::sound::{AudioChannel, frequency_of_bin, scale_to_db};
use crate::ui::aa_line::{AaLines, LineScratch, Target};
use crate::ui::plot::{Axis, PlotData, XYPlotData};
use crate::ui::visualizer::visualizer_widget::Visualizer;
use crate::ui::{
    QUAD_VERTICES, VERTEX_2D_BUFFER_LAYOUT, catmull_rom_spline_into, create_pipeline,
    freq_spectrum_select, log_axis_sel,
};
use crate::wavesync::{WaveSyncAppData, WaveSyncVisuals};
use crate::{create_shader, deref_arc, impl_settings};
use egui;
use egui::{Color32, PaintCallback, PaintCallbackInfo, Rect};
use egui::{Slider, Ui};
use egui_wgpu::wgpu;
use egui_wgpu::wgpu::util::DeviceExt;
use egui_wgpu::wgpu::{CommandBuffer, CommandEncoder, Device, Queue, RenderPass};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

pub const MAX_BARS: u64 = 4096;
pub const MIN_BAR_WIDTH: f32 = 1.0;
pub const MIN_SMOOTHED_WIDTH: f32 = 2.0;
const SPLINE_SAMPLES_PER_PIXEL: f32 = 1.0;

deref_arc!(SpectrumVisualizer);

pub struct Inner {
    audio_service: AudioService,
    data: Arc<RwLock<WaveSyncAppData>>,
    smoother: Mutex<Option<Box<dyn FloatArraySmoother>>>,
    settings_open: AtomicBool,
    last_draw: Mutex<Instant>,
    channel: AudioChannel,
    render_resources: Mutex<Option<RenderResources>>,
    scratch: Mutex<SpectrumScratch>,
}

#[derive(PartialEq, Eq)]
struct SpectrumPositionKey {
    sample_rate: u32,
    fft_size: usize,
    skip: usize,
    axis: crate::ui::vec_cache::AxisMappingKey,
    viewport_min_bits: u32,
    viewport_max_bits: u32,
}

#[derive(Default)]
struct SpectrumScratch {
    peak_uniform_uploads: crate::ui::uniform_upload::UniformUploads,
    uniform_uploads: crate::ui::uniform_upload::UniformUploads,
    bin_positions: crate::ui::vec_cache::CachedVec<SpectrumPositionKey, [f32; 2]>,
    fft: Vec<f32>,
    positions: Vec<[f32; 2]>,
    vertices: Vec<[f32; 2]>,
    instances: Vec<BarInstanceData>,
    spline_padding: Vec<[f32; 2]>,
    spline_output: Vec<[f32; 2]>,
    outline: LineScratch,
    peak_computer: HoldSmoothFalloffPeakComputer,
    peak_values: Vec<f32>,
    peak_positions: Vec<[f32; 2]>,
    peak_line: LineScratch,
    peak_key: Option<(u32, usize, u32, u32, bool, u32)>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SpectrumVisualizerSettings {
    #[serde(default)]
    pub peaks: PeakSettings,
    pub draw_type: SpectrumVisualizerType,
    pub smoother_type: SmootherType,
    pub smoother_factor: f32,
    pub frequency_axis_logarithmic: bool,
    pub smooth_line: bool,
    pub freq_min: u32,
    pub freq_max: u32,
}

impl Default for SpectrumVisualizerSettings {
    fn default() -> Self {
        Self {
            peaks: PeakSettings::default(),
            draw_type: SpectrumVisualizerType::Line,
            smoother_type: SmootherType::Multiplicative,
            smoother_factor: 0.7,
            frequency_axis_logarithmic: true,
            smooth_line: true,
            freq_min: 20,
            freq_max: 24000,
        }
    }
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub enum SpectrumVisualizerType {
    Bar,
    Line,
}

#[derive(PartialEq, Copy, Clone, Debug, Serialize, Deserialize)]
pub enum SmootherType {
    None,
    Multiplicative,
    ExponentialFalloff,
}

impl SpectrumVisualizer {
    pub fn new(
        audio_service: AudioService,
        channel: AudioChannel,
        data: Arc<RwLock<WaveSyncAppData>>,
    ) -> Self {
        Self(Arc::new(Inner {
            audio_service,
            channel,
            data,
            smoother: Mutex::new(None),
            settings_open: Default::default(),
            last_draw: Mutex::new(Instant::now()),
            render_resources: Default::default(),
            scratch: Default::default(),
        }))
    }
}

impl Visualizer for SpectrumVisualizer {
    fn min_canvas_size(&self) -> egui::Vec2 {
        egui::vec2(64.0, 48.0)
    }

    fn get_plot_data(&self) -> PlotData {
        let settings = &self.data.read().unwrap().spectrum_settings;
        let mut data = XYPlotData::from_axis(
            Axis::linear(settings.freq_min as f32, settings.freq_max as f32),
            Axis::linear(-100.0, 0.0),
        );
        data.x_axis.logarithmic = settings.frequency_axis_logarithmic;
        PlotData::XY(data)
    }

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> Option<PaintCallback> {
        Some(egui_wgpu::Callback::new_paint_callback(
            rect,
            SpectrumVisualizerCallback::new(self.clone(), visuals),
        ))
    }

    impl_settings!("Spectrum Settings", ui, this, {
        let settings = &mut this.data.write().unwrap().spectrum_settings;
        settings.peaks.draw_ui(ui);
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Render mode: ");
            egui::ComboBox::from_id_salt("drawtype")
                .selected_text(format!("{:?}", settings.draw_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut settings.draw_type,
                        SpectrumVisualizerType::Bar,
                        "Bar",
                    );
                    ui.selectable_value(
                        &mut settings.draw_type,
                        SpectrumVisualizerType::Line,
                        "Line",
                    );
                });
        });

        ui.horizontal(|ui| {
            ui.label("Smooth line: ");
            ui.checkbox(&mut settings.smooth_line, "");
        });

        ui.horizontal(|ui| {
            ui.label("Frequency axis: ");
            log_axis_sel(ui, &mut settings.frequency_axis_logarithmic);
        });

        ui.horizontal(|ui| {
            ui.label("Smoothing: ");
            egui::ComboBox::from_id_salt("smoothing")
                .selected_text(format!("{:?}", settings.smoother_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut settings.smoother_type, SmootherType::None, "None");
                    ui.selectable_value(
                        &mut settings.smoother_type,
                        SmootherType::Multiplicative,
                        "Multiplicative",
                    );
                    ui.selectable_value(
                        &mut settings.smoother_type,
                        SmootherType::ExponentialFalloff,
                        "Exponential falloff",
                    );
                });
        });

        ui.style_mut().spacing.slider_width = 150.0;
        ui.add(
            Slider::new(&mut settings.smoother_factor, 0.0..=1.0)
                .text("Factor")
                .min_decimals(3)
                .step_by(0.005),
        );

        ui.horizontal(|ui| {
            ui.label("Frequency range:");
            let f_max = this.audio_service.get_max_freq();
            freq_spectrum_select(ui, &mut settings.freq_min, &mut settings.freq_max, f_max);
        });
    });
}

pub struct SpectrumVisualizerCallback {
    visualizer: SpectrumVisualizer,
    color_start: Color32,
    color_end: Color32,
    highlight_color: Color32,
}

impl SpectrumVisualizerCallback {
    pub(crate) fn new(visualizer: SpectrumVisualizer, visuals: &WaveSyncVisuals) -> Self {
        Self {
            visualizer,
            color_start: visuals.color_start(),
            color_end: visuals.color_end(),
            highlight_color: visuals.highlight_color(),
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    color_end: [f32; 4],
    color_start: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct BarInstanceData {
    height: f32,
    x_1: f32,
    x_2: f32,
}

struct RenderResources {
    bars_pipeline: wgpu::RenderPipeline,
    line_fill_pipeline: wgpu::RenderPipeline,
    outline: AaLines,
    peak_line: AaLines,
    vertex_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    queue: Queue,
}

impl CallbackTrait for SpectrumVisualizerCallback {
    fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut CommandEncoder,
        _resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        profiling::scope!("spectrum.prepare");
        let mut resources = self.visualizer.render_resources.lock().unwrap();

        if resources.is_none() {
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("spectrum_vertex_buffer"),
                contents: bytemuck::cast_slice(&QUAD_VERTICES),
                usage: wgpu::BufferUsages::VERTEX,
            });

            let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spectrum_instance_buffer"),
                size: MAX_BARS * 4 * size_of::<f64>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let bar_shader =
                create_shader!(device, "bar shader", "../../../shader/instanced_bars.wgsl");
            let line_fill_shader =
                create_shader!(device, "lf", "../../../shader/instanced_line_fill.wgsl");

            let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("spectrum_uniform_buffer"),
                contents: bytemuck::cast_slice(&[Uniforms {
                    color_start: [1.0, 1.0, 1.0, 1.0],
                    color_end: [1.0, 1.0, 1.0, 1.0],
                }]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("spectrum_uniform_buffer_bind_group_layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Spectrum Uniform Bind Group"),
                layout: &bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("spectrum layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            self.visualizer.scratch.lock().unwrap().uniform_uploads = Default::default();
            self.visualizer.scratch.lock().unwrap().peak_uniform_uploads = Default::default();
            *resources = Some(RenderResources {
                outline: AaLines::new(device, MAX_BARS as usize * 8, Target::Color),
                peak_line: AaLines::new(device, MAX_BARS as usize * 8, Target::Color),
                bars_pipeline: create_pipeline(
                    device,
                    &bar_shader,
                    &pipeline_layout,
                    Default::default(),
                    &[
                        VERTEX_2D_BUFFER_LAYOUT,
                        wgpu::VertexBufferLayout {
                            array_stride: size_of::<BarInstanceData>() as wgpu::BufferAddress,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &[
                                wgpu::VertexAttribute {
                                    offset: 0,
                                    shader_location: 1,
                                    format: wgpu::VertexFormat::Float32,
                                },
                                wgpu::VertexAttribute {
                                    offset: 4,
                                    shader_location: 2,
                                    format: wgpu::VertexFormat::Float32,
                                },
                                wgpu::VertexAttribute {
                                    offset: 8,
                                    shader_location: 3,
                                    format: wgpu::VertexFormat::Float32,
                                },
                            ],
                        },
                    ],
                    "spectrum_bar_pipeline",
                ),
                line_fill_pipeline: create_pipeline(
                    device,
                    &line_fill_shader,
                    &pipeline_layout,
                    Default::default(),
                    &[
                        VERTEX_2D_BUFFER_LAYOUT,
                        wgpu::VertexBufferLayout {
                            array_stride: 4 * size_of::<f32>() as wgpu::BufferAddress,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &[wgpu::VertexAttribute {
                                offset: 0,
                                shader_location: 1,
                                format: wgpu::VertexFormat::Float32x4,
                            }],
                        },
                    ],
                    "spectrum_line_fill_pipeline",
                ),
                vertex_buffer,
                instance_buffer,
                uniform_buffer,
                bind_group,
                queue: queue.clone(),
            })
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut RenderPass<'static>,
        _callback_resources: &CallbackResources,
    ) {
        profiling::scope!("spectrum.paint");
        let resources = self.visualizer.render_resources.lock().unwrap();
        let plot_data = self.visualizer.get_plot_data().expect_xy();
        let smoother_factor = {
            self.visualizer
                .data
                .read()
                .unwrap()
                .spectrum_settings
                .smoother_factor
        };
        let smoother_type = {
            self.visualizer
                .data
                .read()
                .unwrap()
                .spectrum_settings
                .smoother_type
        };

        if let Some(resources) = resources.as_ref() {
            let mut smoother = self.visualizer.smoother.lock().unwrap();
            let mut curr_type = SmootherType::None;
            if let Some(smoother) = smoother.as_ref() {
                curr_type = smoother.get_type()
            }

            if curr_type != smoother_type {
                *smoother = match smoother_type {
                    SmootherType::ExponentialFalloff => {
                        Some(Box::new(ExponentialFalloffSmoother::new()))
                    }
                    SmootherType::Multiplicative => Some(Box::new(MultiplicativeSmoother::new())),
                    SmootherType::None => None,
                }
            }

            if let Some(smoother) = smoother.as_mut() {
                smoother.set_factor(smoother_factor);
            }

            let delta_t = self.visualizer.last_draw.lock().unwrap().elapsed();
            *self.visualizer.last_draw.lock().unwrap() = Instant::now();

            let channel = self.visualizer.channel;
            let current_source = self.visualizer.audio_service.get_source();
            let settings = &self.visualizer.data.read().unwrap().spectrum_settings;

            let mut scratch = self.visualizer.scratch.lock().unwrap();
            let SpectrumScratch {
                uniform_uploads,
                peak_uniform_uploads,
                bin_positions,
                fft: fft_target,
                positions: position_data,
                vertices: vertex_array,
                instances: instance_data,
                spline_padding,
                spline_output,
                outline,
                peak_computer,
                peak_values,
                peak_positions,
                peak_line,
                peak_key,
            } = &mut *scratch;
            self.visualizer
                .audio_service
                .get_fft_into(channel, fft_target);
            let mut fft_data = fft_target.as_slice();

            if let Some(smoother) = smoother.as_mut() {
                fft_data = smoother.smooth_data(delta_t.as_secs_f32(), fft_target);
            }

            let fft_output_size = fft_data.len();
            let fft_size = fft_output_size * 2;
            let skip = current_source
                .calculate_buffer_beginning_skip_for(plot_data.x_axis.min, fft_size)
                .saturating_sub(1);
            let displayed_bins = current_source
                .bin_of_frequency(plot_data.x_axis.max, fft_size)
                .min(fft_output_size);
            let bars_to_draw = displayed_bins.saturating_sub(skip);
            position_data.clear();
            peak_values.clear();
            let key = SpectrumPositionKey {
                sample_rate: current_source.sample_rate,
                fft_size,
                skip,
                axis: (&plot_data.x_axis).into(),
                viewport_min_bits: info.viewport.min.x.to_bits(),
                viewport_max_bits: info.viewport.max.x.to_bits(),
            };
            {
                profiling::scope!("spectrum.bin_position_cache");
                bin_positions.update(key, bars_to_draw + 1 + skip, |i| {
                    if i < skip {
                        return [0.0, 0.0];
                    }
                    let bin_freq =
                        frequency_of_bin(i, current_source.sample_rate as usize, fft_size);
                    [
                        plot_data.x_axis.gl_pos(bin_freq),
                        plot_data.x_axis.val_to_pos(
                            bin_freq,
                            info.viewport.min.x,
                            info.viewport.max.x,
                        ),
                    ]
                });
            }
            let position_array = bin_positions.as_slice();

            let mut last_px_pos: Option<[f32; 2]> = None;
            let mut acc = 0.0;
            let mut coerced_bins = 0;

            let mut bars_drawn = 0;
            for (i, sample) in fft_data.iter().enumerate().skip(skip).take(bars_to_draw) {
                let sample = scale_to_db(*sample).clamp(-150.0, 5.0);
                let [gl_pos, px_pos] = position_array[i];
                let [_gl_next_pos, px_next_pos] = position_array[i + 1];
                let mut gl_pos = gl_pos;
                let mut v = sample;

                if (px_pos - px_next_pos).abs() < MIN_BAR_WIDTH {
                    if let Some([gl_prev_pos, px_prev_pos]) = last_px_pos.as_ref() {
                        let last_px_pos: f32 = *px_prev_pos;
                        let last_gl_pos: f32 = *gl_prev_pos;
                        acc += sample;
                        coerced_bins += 1;

                        if (px_next_pos - last_px_pos).abs() >= MIN_BAR_WIDTH {
                            v = acc / coerced_bins as f32;
                            gl_pos = last_gl_pos;
                        } else {
                            continue;
                        }
                    } else {
                        acc = sample;
                        coerced_bins = 1;
                        last_px_pos = Some([gl_pos, px_pos]);
                        continue;
                    }
                }
                last_px_pos = None;

                position_data.push([gl_pos, plot_data.y_axis.gl_pos(v)]);
                if settings.peaks.enabled {
                    peak_values.push(v);
                }

                bars_drawn += 1;
            }

            peak_positions.clear();
            if settings.peaks.enabled {
                let identity = (
                    current_source.sample_rate,
                    fft_size,
                    settings.freq_min,
                    settings.freq_max,
                    settings.frequency_axis_logarithmic,
                    info.viewport.width().to_bits(),
                );
                if *peak_key != Some(identity) {
                    peak_computer.reset();
                    *peak_key = Some(identity);
                }
                settings.peaks.configure(peak_computer);
                let peaks = peak_computer.compute_peaks(delta_t.as_secs_f32(), peak_values);
                peak_positions.extend(
                    position_data
                        .iter()
                        .zip(peaks)
                        .map(|(p, value)| [p[0], plot_data.y_axis.gl_pos(*value)]),
                );
            } else {
                peak_computer.reset();
                *peak_key = None;
            }

            crate::upload_uniform_if_changed!(
                *uniform_uploads,
                "spectrum.uniform_buffer",
                resources.queue,
                &resources.uniform_buffer,
                bytemuck::bytes_of(&Uniforms {
                    color_end: self.color_end.to_normalized_gamma_f32(),
                    color_start: self.color_start.to_normalized_gamma_f32(),
                })
            );

            render_pass.set_bind_group(0, &resources.bind_group, &[]);

            if bars_drawn == 0 {
                return;
            }

            if settings.draw_type == SpectrumVisualizerType::Line {
                let px_width = 2.0 / (info.viewport.width() * info.pixels_per_point);
                if settings.smooth_line {
                    catmull_rom_spline_into(
                        position_data,
                        SPLINE_SAMPLES_PER_PIXEL,
                        px_width,
                        MIN_SMOOTHED_WIDTH * px_width,
                        spline_padding,
                        spline_output,
                    );
                    std::mem::swap(position_data, spline_output);
                }

                crate::refill_vec!(
                    *vertex_array,
                    position_data
                        .windows(2)
                        .flat_map(|w| [[w[0][0], w[0][1]], [w[1][0], w[1][1]]])
                );
                let count = (position_data.len() as u32).saturating_sub(1);

                crate::profile_upload!(
                    "upload.geometry",
                    "spectrum.instance_buffer.line_fill",
                    resources.queue.write_buffer(
                        &resources.instance_buffer,
                        0,
                        bytemuck::cast_slice(vertex_array),
                    )
                );
                render_pass.set_vertex_buffer(0, resources.vertex_buffer.slice(..));
                render_pass.set_vertex_buffer(1, resources.instance_buffer.slice(..));
                render_pass.set_pipeline(&resources.line_fill_pipeline);
                render_pass.draw(0..6u32, 0..count);

                if position_data.len() >= 2 {
                    let viewport = [
                        info.viewport.width() * info.pixels_per_point,
                        info.viewport.height() * info.pixels_per_point,
                    ];
                    outline.thin_polyline(position_data, viewport);
                    resources
                        .outline
                        .upload(&resources.queue, &outline.segments);
                    resources.outline.configure(
                        &resources.queue,
                        uniform_uploads,
                        self.color_end.to_normalized_gamma_f32(),
                        viewport,
                    );
                    resources.outline.draw(render_pass, outline.segments.len());
                }
            } else {
                crate::refill_vec!(
                    *instance_data,
                    position_data.windows(2).map(|w| BarInstanceData {
                        height: w[0][1],
                        x_1: w[0][0],
                        x_2: w[1][0],
                    })
                );
                let count = instance_data.len() as u32;

                crate::profile_upload!(
                    "upload.geometry",
                    "spectrum.instance_buffer.bars",
                    resources.queue.write_buffer(
                        &resources.instance_buffer,
                        0,
                        bytemuck::cast_slice(instance_data),
                    )
                );
                render_pass.set_vertex_buffer(0, resources.vertex_buffer.slice(..));
                render_pass.set_vertex_buffer(1, resources.instance_buffer.slice(..));
                render_pass.set_pipeline(&resources.bars_pipeline);
                render_pass.draw(0..6u32, 0..count);
            }
            if settings.peaks.enabled && peak_positions.len() >= 2 {
                let viewport = [
                    info.viewport.width() * info.pixels_per_point,
                    info.viewport.height() * info.pixels_per_point,
                ];
                if settings.draw_type == SpectrumVisualizerType::Line {
                    if settings.smooth_line {
                        let px_width = 2.0 / viewport[0];
                        catmull_rom_spline_into(
                            peak_positions,
                            SPLINE_SAMPLES_PER_PIXEL,
                            px_width,
                            MIN_SMOOTHED_WIDTH * px_width,
                            spline_padding,
                            spline_output,
                        );
                        std::mem::swap(peak_positions, spline_output);
                    }
                    peak_line.thin_polyline(peak_positions, viewport);
                } else {
                    peak_line.segments.clear();
                    peak_line
                        .segments
                        .extend(peak_positions.windows(2).map(|w| {
                            crate::ui::aa_line::Segment::horizontal_cap(w[0][0], w[1][0], w[0][1])
                        }));
                }
                resources
                    .peak_line
                    .upload(&resources.queue, &peak_line.segments);
                resources.peak_line.configure(
                    &resources.queue,
                    peak_uniform_uploads,
                    self.highlight_color.to_normalized_gamma_f32(),
                    viewport,
                );
                resources
                    .peak_line
                    .draw(render_pass, peak_line.segments.len());
            }
        }
    }
}
