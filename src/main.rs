#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use crate::app::App;
use crate::persistance::APP_KEY;
use crate::wavesync::{WaveSync, WaveSyncAppData};
use egui::IconData;
use std::env;
use winit::event_loop::EventLoop;

pub mod app;
mod cpu_profiler;
pub mod egui_tools;
pub mod media;
mod performance;
mod persistance;
pub mod sound;
pub mod themes;
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
    cpu_profiler::start();
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

    let mut app = App::new("WaveSync", icon_data, |persistence| {
        let data: WaveSyncAppData = persistence.get(APP_KEY).unwrap_or_default();
        Box::new(WaveSync::new(data))
    });

    event_loop.run_app(&mut app).expect("Failed to run app");
}
