//! Selective analytic line AA. CPU geometry belongs in visualizer Inner state.
use super::uniform_upload::UniformUploads;
use egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Segment {
    start: [f32; 2],
    end: [f32; 2],
    start_normal: [f32; 2],
    end_normal: [f32; 2],
    weight: f32,
    flags: u32,
    padding: [u32; 2],
}
impl Segment {
    pub fn independent(start: [f32; 2], end: [f32; 2], weight: f32) -> Self {
        Self {
            start,
            end,
            start_normal: [0.0; 2],
            end_normal: [0.0; 2],
            weight,
            flags: 1,
            padding: [0; 2],
        }
    }
}
#[derive(Default)]
pub(crate) struct LineScratch {
    pub segments: Vec<Segment>,
    normals: Vec<[f32; 2]>,
}
fn normal(start: [f32; 2], end: [f32; 2], size: [f32; 2]) -> Option<[f32; 2]> {
    let x = (end[0] - start[0]) * size[0];
    let y = (end[1] - start[1]) * size[1];
    let length = x.hypot(y);
    (length > 1e-6 && length.is_finite()).then(|| [-y / length, x / length])
}
impl LineScratch {
    pub fn polyline(&mut self, points: &[[f32; 2]], viewport: [f32; 2]) {
        profiling::scope!("aa_line.build_polyline");
        self.segments.clear();
        self.normals.clear();
        if points.len() < 2 {
            return;
        }
        for i in 0..points.len() {
            let previous = if i > 0 {
                normal(points[i - 1], points[i], viewport)
            } else {
                None
            };
            let next = if i + 1 < points.len() {
                normal(points[i], points[i + 1], viewport)
            } else {
                None
            };
            let n = match (previous, next) {
                (Some(a), Some(b)) => {
                    let sum = [a[0] + b[0], a[1] + b[1]];
                    let denominator = sum[0] * b[0] + sum[1] * b[1];
                    if denominator > 0.00001 {
                        let miter = [sum[0] / denominator, sum[1] / denominator];
                        let limit = (4.0 / miter[0].hypot(miter[1])).min(1.0);
                        [miter[0] * limit, miter[1] * limit]
                    } else {
                        b
                    }
                }
                (Some(n), None) | (None, Some(n)) => n,
                _ => [0.0, 1.0],
            };
            self.normals.push(n);
        }
        for (i, pair) in points.windows(2).enumerate() {
            if !pair.iter().flatten().all(|x| x.is_finite()) {
                continue;
            }
            self.segments.push(Segment {
                start: pair[0],
                end: pair[1],
                start_normal: self.normals[i],
                end_normal: self.normals[i + 1],
                weight: 1.0,
                flags: if i == 0 { 2 } else { 0 } | if i + 2 == points.len() { 4 } else { 0 },
                padding: [0; 2],
            });
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    color: [f32; 4],
    viewport: [f32; 2],
    half_width: f32,
    padding: f32,
}

pub(crate) enum Target {
    Color,
    Intensity,
}
/// GPU objects only. Instances and upload caches remain in Inner.
pub(crate) struct AaLines {
    instances: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}
impl AaLines {
    pub fn new(device: &wgpu::Device, max_segments: usize, target: Target) -> Self {
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("AA line instances"),
            size: (max_segments * size_of::<Segment>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("AA line uniforms"),
            contents: bytemuck::bytes_of(&Uniforms {
                color: [1.0; 4],
                viewport: [1.0; 2],
                half_width: 0.5,
                padding: 0.0,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("AA line bindings"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("AA line bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("AA line layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("AA line shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shader/aa_line.wgsl").into()),
        });
        let (format, blend, entry) = match target {
            Target::Color => (
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::BlendState::ALPHA_BLENDING,
                "fs_color",
            ),
            Target::Intensity => (
                wgpu::TextureFormat::R32Float,
                wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                },
                "fs_intensity",
            ),
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("AA line pipeline"), layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState { module:&shader, entry_point:Some("vs_main"), compilation_options:Default::default(),
                buffers:&[wgpu::VertexBufferLayout { array_stride:size_of::<Segment>() as u64, step_mode:wgpu::VertexStepMode::Instance,
                    attributes:&wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Float32x2,3=>Float32x2,4=>Float32,5=>Uint32] }],
            },
            fragment:Some(wgpu::FragmentState { module:&shader, entry_point:Some(entry), compilation_options:Default::default(),
                targets:&[Some(wgpu::ColorTargetState { format, blend:Some(blend), write_mask:wgpu::ColorWrites::ALL })] }),
            primitive:wgpu::PrimitiveState::default(), depth_stencil:None, multisample:wgpu::MultisampleState::default(), multiview:None, cache:None,
        });
        Self {
            instances,
            uniforms,
            bind_group,
            pipeline,
        }
    }
    pub fn upload(&self, queue: &wgpu::Queue, segments: &[Segment]) {
        if segments.is_empty() {
            return;
        }
        assert!(std::mem::size_of_val(segments) as u64 <= self.instances.size());
        crate::profile_upload!(
            "upload.geometry",
            "aa_line.segments",
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(segments))
        );
    }
    pub fn configure(
        &self,
        queue: &wgpu::Queue,
        cache: &UniformUploads,
        color: [f32; 4],
        viewport: [f32; 2],
    ) {
        crate::upload_uniform_if_changed!(
            cache,
            "aa_line.uniforms",
            queue,
            &self.uniforms,
            bytemuck::bytes_of(&Uniforms {
                color,
                viewport: [viewport[0].max(1.0), viewport[1].max(1.0)],
                half_width: 0.5,
                padding: 0.0
            })
        );
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, count: usize) {
        if count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..count as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins_are_shared_and_bounded_in_pixel_space() {
        let mut scratch = LineScratch::default();
        scratch.polyline(&[[-1.0, 0.0], [0.0, 0.0], [0.0, 1.0]], [800.0, 400.0]);
        assert_eq!(scratch.segments.len(), 2);
        assert_eq!(
            scratch.segments[0].end_normal,
            scratch.segments[1].start_normal
        );
        assert_eq!(scratch.segments[0].flags, 2);
        assert_eq!(scratch.segments[1].flags, 4);
        assert!(scratch.segments.iter().all(|s| {
            s.start_normal
                .iter()
                .chain(s.end_normal.iter())
                .all(|v| v.is_finite() && v.abs() <= 4.0)
        }));
        scratch.polyline(&[[0.0, 0.0]; 3], [800.0, 400.0]);
        assert!(
            scratch
                .segments
                .iter()
                .all(|s| s.start_normal.iter().all(|v| v.is_finite()))
        );
    }
    #[test]
    fn shader_parses_and_validates() {
        let module =
            wgpu::naga::front::wgsl::parse_str(include_str!("../../shader/aa_line.wgsl")).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<Uniforms>(), 32);
        assert_eq!(size_of::<Segment>(), 48);
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    #[test]
    #[ignore = "requires a GL adapter; run explicitly with --ignored"]
    fn offscreen_color_stroke_has_partial_coverage_and_intensity_pipeline_is_valid() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::GL,
                ..Default::default()
            });
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .expect("GL adapter for AA validation");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                    ..Default::default()
                })
                .await
                .unwrap();
            device.push_error_scope(wgpu::ErrorFilter::Validation);
            let lines = AaLines::new(&device, 4, Target::Color);
            let _intensity = AaLines::new(&device, 4, Target::Intensity);
            let mut scratch = LineScratch::default();
            scratch.polyline(&[[-0.8, 0.03], [0.8, 0.03]], [32.0, 32.0]);
            lines.upload(&queue, &scratch.segments);
            lines.configure(
                &queue,
                &UniformUploads::default(),
                [1.0, 0.0, 0.0, 1.0],
                [32.0, 32.0],
            );
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("AA test target"),
                size: wgpu::Extent3d {
                    width: 32,
                    height: 32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256 * 32,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
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
                lines.draw(&mut pass, scratch.segments.len());
            }
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(32),
                    },
                },
                wgpu::Extent3d {
                    width: 32,
                    height: 32,
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
            let alpha: Vec<_> = (0..32)
                .flat_map(|row| {
                    mapped[row * 256..row * 256 + 128]
                        .chunks_exact(4)
                        .map(|pixel| pixel[3])
                })
                .collect();
            assert!(
                alpha.iter().any(|&a| a > 0 && a < 255),
                "AA must produce partial pixel coverage"
            );
            assert!(
                alpha.iter().filter(|&&a| a != 0).count() < 150,
                "thin strokes should remain narrow"
            );
            drop(mapped);
            readback.unmap();
            assert!(
                device.pop_error_scope().await.is_none(),
                "AA pipelines must validate on the app's GL backend"
            );
        });
    }
}
