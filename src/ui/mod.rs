use eframe::wgpu;

pub mod plot;
pub mod visualizer;

/// Creates a custom wrapper type for WGPU resources
/// this allows us to have multiple objects of the same type in the
/// default object registry, neet!
#[macro_export]
macro_rules! define_resource {
    ($name:ident, $inner:ty) => {
        struct $name(pub $inner);
        impl std::ops::Deref for $name {
            type Target = $inner;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        impl std::ops::DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}

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

fn quad_to_triangles(x_min: f32, y_min: f32, x_max: f32, y_max: f32) -> [[f32; 2]; 6] {
    [
        [x_min, y_min], // triangle 1
        [x_max, y_min],
        [x_max, y_max],
        [x_min, y_min], // triangle 2
        [x_max, y_max],
        [x_min, y_max],
    ]
}

fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    pipeline_layout: &wgpu::PipelineLayout,
    topology: wgpu::PrimitiveTopology,
    name: &'static str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(name),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Option::from("vs_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                }],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Option::from("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Bgra8Unorm,
                blend: Some(wgpu::BlendState::REPLACE),
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
