#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod ui;
mod sound;

use crate::ui::visualizer::waveform::{WaveformVisualizer, WaveformVisualizerCallback};
use eframe::{egui, egui_wgpu};

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Wavesync",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);

            Ok(Box::<MyApp>::default())
        }),
    )
}

struct MyApp {
    segments: u32,
    waveform_visualizer: WaveformVisualizer
}

impl Default for MyApp {
    fn default() -> Self {
        Self {
            segments: 42,
            waveform_visualizer: WaveformVisualizer::new(),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(egui::Slider::new(&mut self.segments, 0..=120).text("segments"));
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                egui::Rect::from_min_size(ui.min_rect().min, ui.min_rect().size()),
                WaveformVisualizerCallback::new(self.waveform_visualizer.clone()),
            ));
        });
    }
}
