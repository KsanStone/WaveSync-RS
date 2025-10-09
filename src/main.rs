#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod sound;
mod ui;

use crate::ui::plot::{Plot, PlotData};
use crate::ui::visualizer::spectrum::{SpectrumVisualizer, SpectrumVisualizerCallback};
use crate::ui::visualizer::visualizer_trait::Visualizer;
use crate::ui::visualizer::waveform::{WaveformVisualizer, WaveformVisualizerCallback};
use eframe::egui;
use egui_extras::{Size, StripBuilder};
use std::env;

fn main() -> eframe::Result {
    if env::var("RUST_LOG").is_err() {
        unsafe { env::set_var("RUST_LOG", "info") }
    }
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Wavesync",
        options,
        Box::new(|_cc| {
            Ok(Box::<MyApp>::default())
        }),
    )
}

struct MyApp {
    segments: u32,
    audio_service: sound::audio_service::AudioService,
    waveform_visualizer: WaveformVisualizer,
    spectrum_visualizer: SpectrumVisualizer,
}

impl Default for MyApp {
    fn default() -> Self {
        let audio_service = sound::audio_service::AudioService::new();
        audio_service.init();
        Self {
            segments: 42,
            waveform_visualizer: WaveformVisualizer::new(audio_service.clone()),
            spectrum_visualizer: SpectrumVisualizer::new(audio_service.clone()),
            audio_service,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        catppuccin_egui::set_theme(ctx, catppuccin_egui::MOCHA);
        ctx.request_repaint();
        let mut waveform_plot_data = PlotData::default();
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
            StripBuilder::new(ui)
                .sizes(Size::remainder(), 2)
                .vertical(|mut strip| {
                    strip.cell(|ui| {
                        self.waveform_visualizer
                            .update_axis(&mut waveform_plot_data);
                        let plot = Plot::new(&mut waveform_plot_data);
                        plot.show(
                            ui,
                            WaveformVisualizerCallback::new(self.waveform_visualizer.clone()),
                        );
                    });
                    strip.cell(|ui| {
                        let mut plot_data = self.spectrum_visualizer.get_plot_data();
                        let plot = Plot::new(&mut plot_data);
                        plot.show(
                            ui,
                            SpectrumVisualizerCallback::new(self.spectrum_visualizer.clone()),
                        );
                    });
                });
        });
    }
}
