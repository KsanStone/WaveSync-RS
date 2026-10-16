//! Continuous presentation clock, independent of batched FFT delivery.
#[derive(Default)]
pub(super) struct WaterfallScroll {
    uploaded: u64,
    cursor: Option<f64>,
}
impl WaterfallScroll {
    /// Keep two rows of latency to absorb producer/render scheduling jitter.
    /// Four guard rows in the texture protect the oldest edge while interpolating.
    pub fn advance(&mut self, rows: usize, dt: f64, rate: f64) -> f32 {
        self.uploaded = self.uploaded.saturating_add(rows as u64);
        let newest = self.uploaded as f64;
        let target = newest - 2.0;
        let cursor = self.cursor.get_or_insert(target);
        if dt > 0.25 || (target - *cursor).abs() > 3.0 {
            *cursor = target;
        } else {
            let previous = *cursor;
            *cursor += dt.max(0.0) * rate;
            *cursor += (target - *cursor) * (1.0 - (-dt.max(0.0) * 2.0).exp());
            *cursor = cursor.max(previous);
        }
        *cursor = cursor.clamp(newest - 3.0, newest);
        (newest - *cursor) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advances_between_fft_deliveries_without_wrap_jumps() {
        let mut scroll = WaterfallScroll::default();
        let first = scroll.advance(1, 0.0, 60.0);
        let next = scroll.advance(0, 1.0 / 240.0, 60.0);
        assert!(next < first);
        let mut previous_position = 1.0 - next as f64;
        for frame in 1..400 {
            let rows = usize::from(frame % 4 == 0);
            let lag = scroll.advance(rows, 1.0 / 240.0, 60.0);
            assert!((0.0..=3.0).contains(&lag));
            let position = scroll.uploaded as f64 - lag as f64;
            assert!(position >= previous_position - 1e-6);
            assert!(position - previous_position < 0.4);
            previous_position = position;
        }
    }
    #[test]
    fn low_fft_rates_do_not_scroll_backwards_after_a_delivery() {
        for rate in [1, 10, 60, 200] {
            let mut scroll = WaterfallScroll::default();
            let mut previous = -2.0;
            for frame in 0..1200 {
                let rows = ((frame + 1) * rate / 240 - frame * rate / 240) as usize;
                let lag = scroll.advance(rows, 1.0 / 240.0, rate as f64);
                let position = scroll.uploaded as f64 - lag as f64;
                assert!(position >= previous - 1e-6);
                previous = position;
            }
        }
    }
    #[test]
    fn pause_stops_at_latest_row_and_resume_is_bounded() {
        let mut scroll = WaterfallScroll::default();
        scroll.advance(10, 0.0, 60.0);
        for _ in 0..200 {
            scroll.advance(0, 1.0 / 120.0, 60.0);
        }
        assert_eq!(scroll.advance(0, 1.0 / 120.0, 60.0), 0.0);
        assert!((0.0..=3.0).contains(&scroll.advance(64, 1.0, 60.0)));
    }
}

#[cfg(test)]
mod shader_tests {
    use egui_wgpu::wgpu;
    #[test]
    fn waterfall_shader_validates() {
        let module =
            wgpu::naga::front::wgsl::parse_str(include_str!("../../../shader/spectrogram.wgsl"))
                .unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
    #[test]
    #[ignore = "requires a GL adapter; run explicitly with --ignored"]
    fn oldest_edge_does_not_wrap_to_newest_row() {
        use crate::ui::visualizer::spectrogram::Uniforms;
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
            let (history, history_view, history_ty) = crate::ui::create_texture(
                "history test",
                &device,
                1,
                8,
                wgpu::TextureFormat::Rgba8Unorm,
                false,
            );
            let (gradient, gradient_view, gradient_ty) = crate::ui::create_texture(
                "gradient test",
                &device,
                256,
                1,
                wgpu::TextureFormat::Rgba8Unorm,
                false,
            );
            let colors: Vec<_> = (0..256)
                .map(|i| [i as f32 / 255.0, i as f32 / 255.0, i as f32 / 255.0, 1.0])
                .collect();
            crate::ui::write_1d_texture(&queue, &gradient, &colors);
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mapping = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&[0i32; 66]),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let (layout, bind_group) = crate::ui::create_bind_group_with_layout(
                &device,
                &[
                    (
                        0,
                        &crate::ui::bind_buff(wgpu::BufferBindingType::Uniform),
                        uniform.as_entire_binding(),
                    ),
                    (
                        1,
                        &history_ty,
                        wgpu::BindingResource::TextureView(&history_view),
                    ),
                    (
                        2,
                        &gradient_ty,
                        wgpu::BindingResource::TextureView(&gradient_view),
                    ),
                    (
                        3,
                        &crate::ui::bind_buff(wgpu::BufferBindingType::Storage { read_only: true }),
                        mapping.as_entire_binding(),
                    ),
                ],
                "waterfall test",
            );
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../../../shader/spectrogram.wgsl").into(),
                ),
            });
            let pipeline = crate::ui::create_pipeline(
                &device,
                &shader,
                &pipeline_layout,
                wgpu::PrimitiveTopology::TriangleList,
                &[crate::ui::VERTEX_2D_BUFFER_LAYOUT],
                "waterfall test",
            );
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&crate::ui::FULL_SCREEN_QUAD),
                usage: wgpu::BufferUsages::VERTEX,
            });
            for vertical in [false, true] {
                for (width, height) in [(7, 13), (31, 17), (65, 33)] {
                    for head in [0, 1, 7] {
                        let mut rows = [0u8; 32];
                        for age in 0..8 {
                            let row = (head + 8 - age) % 8;
                            rows[row * 4..row * 4 + 4].fill(255 - age as u8 * 32);
                        }
                        queue.write_texture(
                            history.as_image_copy(),
                            &rows,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(4),
                                rows_per_image: Some(8),
                            },
                            wgpu::Extent3d {
                                width: 1,
                                height: 8,
                                depth_or_array_layers: 1,
                            },
                        );
                        queue.write_buffer(
                            &uniform,
                            0,
                            bytemuck::bytes_of(&Uniforms {
                                width,
                                height,
                                head_offset: head as i32,
                                buffer_size: 8,
                                is_vertical: i32::from(vertical),
                                scroll_offset: 0.5,
                            }),
                        );
                        let texture = device.create_texture(&wgpu::TextureDescriptor {
                            label: None,
                            size: wgpu::Extent3d {
                                width: width as u32,
                                height: height as u32,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Bgra8Unorm,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        });
                        let view = texture.create_view(&Default::default());
                        let stride = (width as u32 * 4).div_ceil(256) * 256;
                        let readback = device.create_buffer(&wgpu::BufferDescriptor {
                            label: None,
                            size: (stride * height as u32) as u64,
                            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
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
                            pass.set_pipeline(&pipeline);
                            pass.set_bind_group(0, &bind_group, &[]);
                            pass.set_vertex_buffer(0, vertices.slice(..));
                            pass.draw(0..6, 0..1);
                        }
                        encoder.copy_texture_to_buffer(
                            texture.as_image_copy(),
                            wgpu::TexelCopyBufferInfo {
                                buffer: &readback,
                                layout: wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(stride),
                                    rows_per_image: Some(height as u32),
                                },
                            },
                            wgpu::Extent3d {
                                width: width as u32,
                                height: height as u32,
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
                        let bytes = readback.slice(..).get_mapped_range();
                        // Newest is at top for vertical, at right for horizontal.
                        let newest = if vertical {
                            bytes[(width / 2 * 4) as usize]
                        } else {
                            bytes[((height / 2) as u32 * stride + (width as u32 - 1) * 4) as usize]
                        };
                        let oldest = if vertical {
                            bytes[((height as u32 - 1) * stride + (width as u32 / 2) * 4) as usize]
                        } else {
                            bytes[((height / 2) as u32 * stride) as usize]
                        };
                        assert!(
                            newest > oldest + 60,
                            "newest row leaked to oldest edge at {width}x{height}, head={head}, vertical={vertical}: {newest}/{oldest}"
                        );
                        // Frequency endpoint must remain valid, including the final pixel.
                        assert_eq!(
                            bytes[((height as u32 - 1) * stride + (width as u32 - 1) * 4 + 3)
                                as usize],
                            255
                        );
                        drop(bytes);
                        readback.unmap();
                    }
                }
            }
            assert!(device.pop_error_scope().await.is_none());
        });
    }
}
