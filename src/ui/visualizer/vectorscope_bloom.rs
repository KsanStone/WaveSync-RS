//! Half-resolution, separable phosphor bloom. GPU objects only; no CPU image data.
use egui_wgpu::wgpu;

pub(super) struct Bloom {
    _horizontal_texture: wgpu::Texture,
    _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    horizontal_pipeline: wgpu::ComputePipeline,
    vertical_pipeline: wgpu::ComputePipeline,
    horizontal_bind_group: wgpu::BindGroup,
    vertical_bind_group: wgpu::BindGroup,
    size: [u32; 2],
}
impl Bloom {
    pub fn new(
        device: &wgpu::Device,
        intensity: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Self {
        let size = [width.div_ceil(2), height.div_ceil(2)];
        let descriptor = wgpu::TextureDescriptor {
            label: Some("vectorscope bloom"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let horizontal = device.create_texture(&descriptor);
        let horizontal_view = horizontal.create_view(&Default::default());
        let texture = device.create_texture(&descriptor);
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vectorscope bloom interpolation"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vectorscope bloom shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shader/vectorscope_bloom.wgsl").into(),
            ),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let horizontal_pipeline = pipeline("horizontal");
        let vertical_pipeline = pipeline("vertical");
        let horizontal_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vectorscope bloom horizontal bindings"),
            layout: &horizontal_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(intensity),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&horizontal_view),
                },
            ],
        });
        let vertical_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vectorscope bloom vertical bindings"),
            layout: &vertical_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&horizontal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        Self {
            _horizontal_texture: horizontal,
            _texture: texture,
            view,
            sampler,
            horizontal_pipeline,
            vertical_pipeline,
            horizontal_bind_group,
            vertical_bind_group,
            size,
        }
    }
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        profiling::scope!("vectorscope.bloom_encode");
        for (label, pipeline, bindings) in [
            (
                "vectorscope bloom horizontal",
                &self.horizontal_pipeline,
                &self.horizontal_bind_group,
            ),
            (
                "vectorscope bloom vertical",
                &self.vertical_pipeline,
                &self.vertical_bind_group,
            ),
        ] {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bindings, &[]);
            pass.dispatch_workgroups(self.size[0].div_ceil(8), self.size[1].div_ceil(8), 1);
        }
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    #[test]
    #[ignore = "requires a GL adapter; run explicitly with --ignored"]
    fn moderate_sources_bloom_and_brighter_sources_spread_further() {
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
                .request_device(&wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                    ..Default::default()
                })
                .await
                .unwrap();
            device.push_error_scope(wgpu::ErrorFilter::Validation);
            let source = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bloom regression source"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let bloom = Bloom::new(&device, &source.create_view(&Default::default()), 64, 64);
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256 * 32,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut fractions = Vec::new();
            for intensity in [0.5f32, 16.0] {
                let mut pixels = vec![0.0f32; 64 * 64];
                for y in 32..34 {
                    for x in 32..34 {
                        pixels[y * 64 + x] = intensity;
                    }
                }
                queue.write_texture(
                    source.as_image_copy(),
                    bytemuck::cast_slice(&pixels),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(64),
                    },
                    source.size(),
                );
                let mut encoder = device.create_command_encoder(&Default::default());
                bloom.encode(&mut encoder);
                encoder.copy_texture_to_buffer(
                    bloom._texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(32),
                        },
                    },
                    bloom._texture.size(),
                );
                queue.submit(Some(encoder.finish()));
                let (tx, rx) = std::sync::mpsc::channel();
                readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        tx.send(result).unwrap();
                    });
                device.poll(wgpu::PollType::Wait).unwrap();
                rx.recv().unwrap().unwrap();
                let mapped = readback.slice(..).get_mapped_range();
                let mut total = 0.0f32;
                let mut outer = 0.0f32;
                for y in 0..32 {
                    for x in 0..32 {
                        let offset = y * 256 + x * 8;
                        let mut value = 0.0;
                        for channel in 0..3 {
                            let i = offset + channel * 2;
                            let bits = u16::from_le_bytes([mapped[i], mapped[i + 1]]);
                            let exponent = ((bits >> 10) & 31) as i32;
                            let fraction = (bits & 1023) as f32;
                            value += if exponent == 0 {
                                fraction * 2.0f32.powi(-24)
                            } else {
                                (1.0 + fraction / 1024.0) * 2.0f32.powi(exponent - 15)
                            };
                        }
                        total += value;
                        if (x as f32 - 16.0).hypot(y as f32 - 16.0) > 4.0 {
                            outer += value;
                        }
                    }
                }
                assert!(
                    total > 0.1,
                    "even a source below intensity 1 must create visible bloom energy"
                );
                fractions.push(outer / total);
                drop(mapped);
                readback.unmap();
            }
            assert!(
                fractions[1] > fractions[0] * 2.0,
                "bright sources must spread a larger fraction of light to distant pixels: {fractions:?}"
            );
            assert!(device.pop_error_scope().await.is_none());
        });
    }
}
