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
        Box::new(|_cc| Ok(Box::<MyApp>::default())),
    )
}

struct MyApp {
    segments: u32,
    audio_service: sound::audio_service::AudioService,
    waveform_visualizer: WaveformVisualizer,
    spectrum_visualizer: SpectrumVisualizer,
    settings_shown: bool,
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
            settings_shown: false,
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
                    if ui.button("Settings").clicked() {
                        self.settings_shown = true;
                    }
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
        if self.settings_shown {
            egui::Window::new("Settings")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Fft size");
                        let mut selected = self.audio_service.get_fft_size();
                        egui::ComboBox::from_id_salt("fft_size")
                            .selected_text(format!("{:?}", selected))
                            .show_ui(ui, |ui| {
                                for i in 8 .. 17 {
                                    let v = 2usize.pow(i);
                                    ui.selectable_value(&mut selected, v, format!("{:?}", v));
                                }
                            });
                        self.audio_service.set_fft_size(selected);
                    });
                    if ui.button("Close").clicked() {
                        self.settings_shown = false;
                    }
                });
        }
    }
}
