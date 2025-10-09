#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod ui;
mod sound;

use crate::ui::plot::{GpuPlot, PlotData};
use crate::ui::visualizer::waveform::{WaveformVisualizer, WaveformVisualizerCallback};
use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
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
    audio_service: sound::audio_service::AudioService,
    waveform_visualizer: WaveformVisualizer
}

impl Default for MyApp {
    fn default() -> Self {
        let audio_service = sound::audio_service::AudioService::new();
        audio_service.init();
        Self {
            segments: 42,
            waveform_visualizer: WaveformVisualizer::new(audio_service.clone()),
            audio_service,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        let mut plot_data = PlotData::default();
        egui::TopBottomPanel::bottom("bottom_bar")
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let mut selected = "a";
                    egui::ComboBox::from_id_salt("asd")
                        .selected_text(format!("{:?}", selected))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut selected, "a", "First");
                            ui.selectable_value(&mut selected, "b", "Second");
                            ui.selectable_value(&mut selected, "c", "Third");
                        });
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            self.waveform_visualizer.update_axis(&mut plot_data);
            let plot = GpuPlot::new(&mut plot_data);
            plot.show(ui, WaveformVisualizerCallback::new(self.waveform_visualizer.clone()), );
        });
    }
}
