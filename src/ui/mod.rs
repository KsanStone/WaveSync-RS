use egui::Rect;
use egui_wgpu::ScreenDescriptor;
use egui_wgpu::wgpu;
use wgpu::{BindGroup, BindGroupLayout, BlendState, Device, RenderPass};

pub mod gradient;
pub mod plot;
pub mod visualizer;

#[macro_export]
macro_rules! deref_arc {
    ($name:ident) => {
        #[derive(Clone)]
        pub struct $name(pub std::sync::Arc<Inner>);
        impl std::ops::Deref for $name {
            type Target = Inner;
            fn deref(&self) -> &Self::Target {
                &self.0.deref()
            }
        }
    };
}

const fn quad_to_triangles(x_min: f32, y_min: f32, x_max: f32, y_max: f32) -> [[f32; 2]; 6] {
    [
        [x_min, y_min], // triangle 1
        [x_max, y_min],
        [x_max, y_max],
        [x_min, y_min], // triangle 2
        [x_max, y_max],
        [x_min, y_max],
    ]
}

pub const QUAD_VERTICES: [[f32; 2]; 6] = quad_to_triangles(0.0, 0.0, 1.0, 1.0);
pub const FULL_SCREEN_QUAD: [[f32; 2]; 6] = quad_to_triangles(-1.0, -1.0, 1.0, 1.0);

pub const VERTEX_2D_BUFFER_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: size_of::<[f32; 2]>() as wgpu::BufferAddress,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &[wgpu::VertexAttribute {
        offset: 0,
        shader_location: 0,
        format: wgpu::VertexFormat::Float32x2,
    }],
};

#[macro_export]
macro_rules! create_shader {
    ($device:expr, $name:expr, $source:expr) => {
        $device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some($name),
            source: wgpu::ShaderSource::Wgsl(include_str!($source).into()),
        })
    };
}

fn create_pipeline(
    device: &Device,
    shader: &wgpu::ShaderModule,
    pipeline_layout: &wgpu::PipelineLayout,
    topology: wgpu::PrimitiveTopology,
    buffers: &[wgpu::VertexBufferLayout<'_>],
    name: &'static str,
) -> wgpu::RenderPipeline {
    create_pipeline_color(
        device,
        shader,
        pipeline_layout,
        topology,
        buffers,
        name,
        wgpu::TextureFormat::Bgra8Unorm,
        BlendState::ALPHA_BLENDING,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_pipeline_color(
    device: &Device,
    shader: &wgpu::ShaderModule,
    pipeline_layout: &wgpu::PipelineLayout,
    topology: wgpu::PrimitiveTopology,
    buffers: &[wgpu::VertexBufferLayout<'_>],
    name: &'static str,
    target_format: wgpu::TextureFormat,
    blend: BlendState,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(name),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Option::from("vs_main"),
            compilation_options: Default::default(),
            buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Option::from("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn bind_buff(ty: wgpu::BufferBindingType) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn uniform_bindings(
    device: &Device,
    binding: u32,
    buffer: &wgpu::Buffer,
    name: &'static str,
) -> (BindGroupLayout, BindGroup) {
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&format!("{}{}", name, "_bind_group_layout")),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding,
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
        label: Some(&format!("{}{}", name, "_bind_group")),
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (bind_group_layout, bind_group)
}

pub fn create_bind_group_with_layout(
    device: &Device,
    entries: &[(u32, &wgpu::BindingType, wgpu::BindingResource)],
    label: &'static str,
) -> (BindGroupLayout, BindGroup) {
    let layout_entries: Vec<_> = entries
        .iter()
        .map(|(binding, ty, _)| wgpu::BindGroupLayoutEntry {
            binding: *binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: *(*ty),
            count: None,
        })
        .collect();

    let bind_group_entries: Vec<_> = entries
        .iter()
        .map(|(binding, _, resource)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: resource.clone(),
        })
        .collect();

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&format!("{}_layout", label)),
        entries: &layout_entries,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("{}_bind_group", label)),
        layout: &bind_group_layout,
        entries: &bind_group_entries,
    });

    (bind_group_layout, bind_group)
}

fn create_texture(
    label: &'static str,
    device: &Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    filterable: bool,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindingType) {
    let dimension = if height == 1 {
        wgpu::TextureDimension::D1
    } else {
        wgpu::TextureDimension::D2
    };
    let view_dimension = if height == 1 {
        wgpu::TextureViewDimension::D1
    } else {
        wgpu::TextureViewDimension::D2
    };
    let tex_desc = wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    };
    let tex = device.create_texture(&tex_desc);
    let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_ty = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable },
        view_dimension,
        multisampled: false,
    };
    (tex, tex_view, bind_ty)
}

pub fn write_1d_texture(queue: &wgpu::Queue, texture: &wgpu::Texture, data: &[[f32; 4]]) {
    let tex_bytes = bytemuck::cast_slice(data);

    let buf_layout = wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some((size_of::<f32>() * data.len()) as u32 * 4),
        rows_per_image: None,
    };

    // Define the copy size
    let copy_size = wgpu::Extent3d {
        width: data.len() as u32,
        height: 1,
        depth_or_array_layers: 1,
    };

    // Perform the texture write operation
    queue.write_texture(texture.as_image_copy(), tex_bytes, buf_layout, copy_size);
}

pub fn write_2d_texture_row(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    row: &[f32],
    sample_buffer_position: u32,
) {
    // Convert row to bytes
    let row_bytes = bytemuck::cast_slice(row);

    // Specify the destination in the texture
    let texture_copy = wgpu::TexelCopyTextureInfo {
        texture,
        mip_level: 0,
        origin: wgpu::Origin3d {
            x: 0,
            y: sample_buffer_position,
            z: 0,
        },
        aspect: wgpu::TextureAspect::All,
    };

    // Specify the layout of the data in memory
    let data_layout = wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(size_of_val(row) as u32),
        rows_per_image: Some(1),
    };

    // Size to write
    let copy_size = wgpu::Extent3d {
        width: row.len() as u32,
        height: 1,
        depth_or_array_layers: 1,
    };

    queue.write_texture(texture_copy, row_bytes, data_layout, copy_size);
}

pub fn viewport(rect: Rect, screen_descriptor: &ScreenDescriptor, pass: &mut RenderPass) {
    pass.set_viewport(
        rect.min.x * screen_descriptor.pixels_per_point,
        rect.min.y * screen_descriptor.pixels_per_point,
        rect.width() * screen_descriptor.pixels_per_point,
        rect.height() * screen_descriptor.pixels_per_point,
        0.0,
        1.0,
    );
}

pub fn catmull_rom_spline(
    points: &[[f32; 2]],
    px_per_point: f32,
    px_width: f32,
    min_width: f32,
) -> Vec<[f32; 2]> {
    let n = points.len();
    if n < 2 {
        return points.to_vec();
    }

    // Pad the ends to keep first/last visible
    let mut padded = Vec::with_capacity(n + 2);
    padded.push(points[0]);
    padded.extend_from_slice(points);
    padded.push(points[n - 1]);

    let mut result = Vec::new();
    result.push(points[0]); // explicitly keep first point

    for i in 0..padded.len() - 3 {
        let p0 = padded[i];
        let p1 = padded[i + 1];
        let p2 = padded[i + 2];
        let p3 = padded[i + 3];

        let seg_width = (p2[0] - p1[0]).abs();
        if seg_width < min_width {
            result.extend_from_slice(&padded[i + 2..padded.len() - 1]);
            break;
        }

        let samples_per_segment = ((seg_width / px_width) * px_per_point).max(1.0).round() as usize;

        for j in 1..=samples_per_segment {
            let t = j as f32 / samples_per_segment as f32;
            let t2 = t * t;
            let t3 = t2 * t;

            let x = 0.5
                * ((2.0 * p1[0])
                    + (-p0[0] + p2[0]) * t
                    + (2.0 * p0[0] - 5.0 * p1[0] + 4.0 * p2[0] - p3[0]) * t2
                    + (-p0[0] + 3.0 * p1[0] - 3.0 * p2[0] + p3[0]) * t3);

            let y = 0.5
                * ((2.0 * p1[1])
                    + (-p0[1] + p2[1]) * t
                    + (2.0 * p0[1] - 5.0 * p1[1] + 4.0 * p2[1] - p3[1]) * t2
                    + (-p0[1] + 3.0 * p1[1] - 3.0 * p2[1] + p3[1]) * t3);

            result.push([x, y]);
        }
    }

    result.push(points[n - 1]);
    result
}
