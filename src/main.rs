use crate::app::App;
use crate::persistance::APP_KEY;
use crate::wavesync::{WaveSync, WaveSyncAppData};
use egui::IconData;
use std::env;
use winit::event_loop::{ControlFlow, EventLoop};

pub mod app;
pub mod egui_tools;
mod persistance;
pub mod sound;
pub mod ui;
pub mod wavesync;

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        pollster::block_on(run());
    }
}

async fn run() {
    if env::var("RUST_LOG").is_err() {
        unsafe { env::set_var("RUST_LOG", "info,wgpu_hal=off") }
    }
    env_logger::init();
    let icon_bytes = include_bytes!("../icon.png");
    let image = image::load_from_memory(icon_bytes)
        .expect("Failed to load icon")
        .into_rgba8();

    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    let icon_data = IconData {
        height,
        width,
        rgba,
    };
    let event_loop = EventLoop::new().unwrap();

    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new("WaveSync", icon_data, |persistence| {
        let data: WaveSyncAppData = persistence.get(APP_KEY).unwrap_or_default();
        Box::new(WaveSync::new(data))
    });

    event_loop.run_app(&mut app).expect("Failed to run app");
}
