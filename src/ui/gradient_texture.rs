//! CPU-baked gradient LUT. A one-row texture keeps logical 1D lookup portable to GL.
use super::gradient::Gradient;
use egui_wgpu::wgpu;
use std::sync::Mutex;

pub const GRADIENT_RESOLUTION: u32 = 256;

pub struct GradientTexture {
    texture: wgpu::Texture,
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    cached: Mutex<Option<Gradient>>,
}

#[cfg(test)]
mod tests {
    use super::super::gradient::Stop;
    use super::*;
    use egui::Color32;

    #[test]
    fn all_gradient_fill_shaders_validate() {
        for source in [
            include_str!("../../shader/instanced_bars.wgsl"),
            include_str!("../../shader/instanced_line_fill.wgsl"),
            include_str!("../../shader/radial_gradient_fill.wgsl"),
            include_str!("../../shader/spectrogram.wgsl"),
        ] {
            let module =
                wgpu::naga::front::wgsl::parse_str(&GradientTexture::shader_source(source))
                    .unwrap();
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    #[ignore = "requires a GL adapter; run explicitly with --ignored"]
    fn gradient_cache_and_fill_pipelines_render_both_modes() {
        use wgpu::util::DeviceExt;
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
            let gradient = GradientTexture::new(&device, GRADIENT_RESOLUTION);
            let palettes = [
                Gradient::new(vec![
                    Stop::new(0.0, Color32::from_rgb(70, 130, 240)),
                    Stop::new(1.0, Color32::from_rgb(70, 130, 240)),
                ])
                .unwrap(),
                Gradient::new(vec![
                    Stop::new(0.0, Color32::RED),
                    Stop::new(0.5, Color32::GREEN),
                    Stop::new(1.0, Color32::BLUE),
                ])
                .unwrap(),
            ];
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 128,
                    height: 128,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 128 * 128 * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut preview = image::RgbaImage::new(128 * 3, 128 * 2);
            for kind in 0..3 {
                let radial = kind == 2;
                let vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(if radial {
                        &crate::ui::FULL_SCREEN_QUAD
                    } else {
                        &crate::ui::QUAD_VERTICES
                    }),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let instance_data: Vec<f32> = if kind == 0 {
                    (0..16)
                        .flat_map(|i| {
                            [
                                -0.9 + i as f32 / 15.0 * 1.8,
                                -1.0 + i as f32 / 8.0,
                                -1.0 + (i + 1) as f32 / 8.0 - 0.02,
                            ]
                        })
                        .collect()
                } else {
                    vec![-1.0, -0.6, 1.0, 0.9]
                };
                let inst = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&instance_data),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&[0.0f32, 0.0, 1.0, 0.0]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let (layout, bind) =
                    crate::ui::uniform_bindings(&device, 0, &uniform, "radial test");
                let layouts = if radial {
                    vec![&layout, &gradient.layout]
                } else {
                    vec![&gradient.layout]
                };
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: None,
                        bind_group_layouts: &layouts,
                        push_constant_ranges: &[],
                    });
                let source = [
                    include_str!("../../shader/instanced_bars.wgsl"),
                    include_str!("../../shader/instanced_line_fill.wgsl"),
                    include_str!("../../shader/radial_gradient_fill.wgsl"),
                ][kind];
                let shader = GradientTexture::shader_at(
                    &device,
                    "gradient fill test",
                    source,
                    if radial { 1 } else { 0 },
                );
                let bar_attributes = wgpu::vertex_attr_array![1=>Float32,2=>Float32,3=>Float32];
                let fill_attributes = wgpu::vertex_attr_array![1=>Float32x4];
                let mut buffers = vec![crate::ui::VERTEX_2D_BUFFER_LAYOUT];
                if !radial {
                    buffers.push(wgpu::VertexBufferLayout {
                        array_stride: if kind == 0 { 12 } else { 16 },
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: if kind == 0 {
                            &bar_attributes
                        } else {
                            &fill_attributes
                        },
                    });
                }
                let pipeline = crate::ui::create_pipeline_color(
                    &device,
                    &shader,
                    &pipeline_layout,
                    wgpu::PrimitiveTopology::TriangleList,
                    &buffers,
                    "gradient fill test",
                    wgpu::TextureFormat::Rgba8Unorm,
                    wgpu::BlendState::ALPHA_BLENDING,
                );
                let mut first = None;
                for (mode, palette) in palettes.iter().enumerate() {
                    assert!(gradient.update(&queue, palette));
                    assert!(!gradient.update(&queue, palette));
                    let mut encoder = device.create_command_encoder(&Default::default());
                    let view = target.create_view(&Default::default());
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
                        pass.set_vertex_buffer(0, vertex.slice(..));
                        if radial {
                            pass.set_bind_group(0, &bind, &[]);
                            pass.set_bind_group(1, &gradient.bind_group, &[]);
                        } else {
                            pass.set_bind_group(0, &gradient.bind_group, &[]);
                            pass.set_vertex_buffer(1, inst.slice(..));
                        }
                        pass.draw(0..6, 0..if kind == 0 { 16 } else { 1 });
                    }
                    encoder.copy_texture_to_buffer(
                        target.as_image_copy(),
                        wgpu::TexelCopyBufferInfo {
                            buffer: &readback,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(512),
                                rows_per_image: Some(128),
                            },
                        },
                        target.size(),
                    );
                    queue.submit(Some(encoder.finish()));
                    let (tx, rx) = std::sync::mpsc::channel();
                    readback
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
                    device.poll(wgpu::PollType::Wait).unwrap();
                    rx.recv().unwrap().unwrap();
                    let data = readback.slice(..).get_mapped_range();
                    assert!(data.chunks(4).any(|pixel| pixel[3] > 0));
                    if let Some(ref first) = first {
                        assert_ne!(&data[..], first);
                    } else {
                        first = Some(data.to_vec());
                    }
                    let img = image::RgbaImage::from_raw(128, 128, data.to_vec()).unwrap();
                    image::imageops::replace(
                        &mut preview,
                        &img,
                        kind as i64 * 128,
                        mode as i64 * 128,
                    );
                    drop(data);
                    readback.unmap();
                }
            }
            preview.save("target/graph-gradients.png").unwrap();
            assert!(device.pop_error_scope().await.is_none());
        });
    }
}

impl GradientTexture {
    pub fn new(device: &wgpu::Device, resolution: u32) -> Self {
        assert!(resolution >= 2);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gradient LUT"),
            size: wgpu::Extent3d {
                width: resolution,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gradient LUT layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gradient LUT bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            }],
        });
        Self {
            texture,
            layout,
            bind_group,
            cached: Mutex::new(None),
        }
    }

    /// Returns true only when the CPU table is regenerated and uploaded.
    pub fn update(&self, queue: &wgpu::Queue, gradient: &Gradient) -> bool {
        let mut cached = self.cached.lock().unwrap();
        if cached.as_ref() == Some(gradient) {
            return false;
        }
        let pixels: Vec<[u8; 4]> = gradient
            .pre_compute_lookup(self.texture.width() as usize)
            .into_iter()
            .map(|color| color.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect();
        queue.write_texture(
            self.texture.as_image_copy(),
            bytemuck::cast_slice(&pixels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.texture.width() * 4),
                rows_per_image: Some(1),
            },
            self.texture.size(),
        );
        *cached = Some(gradient.clone());
        true
    }

    pub fn shader_source(source: &str) -> String {
        format!("{}\n{source}", include_str!("../../shader/gradient.wgsl"))
    }

    pub fn shader(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
        Self::shader_at(device, label, source, 1)
    }

    pub fn shader_at(
        device: &wgpu::Device,
        label: &str,
        source: &str,
        group: u32,
    ) -> wgpu::ShaderModule {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(
                Self::shader_source(source)
                    .replace("@group(1)", &format!("@group({group})"))
                    .into(),
            ),
        })
    }
}
