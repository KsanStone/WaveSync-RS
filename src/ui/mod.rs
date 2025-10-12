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