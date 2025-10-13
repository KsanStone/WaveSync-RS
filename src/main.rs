#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod sound;
mod ui;

use crate::ui::visualizer::spectrum::SpectrumVisualizer;
use crate::ui::visualizer::visualizer_widget::VisualizerWidget;
use crate::ui::visualizer::waveform::WaveformVisualizer;
use eframe::egui::{Color32, Widget};
use eframe::{egui, wgpu};
use egui_extras::{Size, StripBuilder};
use log::info;
use std::env;
use std::ops::RangeInclusive;
use std::time::Instant;

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
        Box::new(|cc| {
            let device = &cc.wgpu_render_state.as_ref().unwrap().device;
            if device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
                // create query set & buffer
                let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("timestamp_queries"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                });
                let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("timestamp_resolve_buffer"),
                    size: 16, // enough for 2 timestamps
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                info!("Time Queries available")
            }
            Ok(Box::new(WaveSync::new()))
        }),
    )
}

struct WaveSync {
    audio_service: sound::audio_service::AudioService,
    waveform_visualizer: WaveformVisualizer,
    spectrum_visualizer: SpectrumVisualizer,
    settings_shown: bool,
    last_update: Instant,
    visuals: WaveSyncVisuals
}

struct WaveSyncVisuals {
    theme: catppuccin_egui::Theme,
}

impl WaveSyncVisuals {
    pub fn wave_color(&self) -> Color32 {
        self.theme.blue
    }
}

impl WaveSync {
    fn new() -> Self {
        let audio_service = sound::audio_service::AudioService::new();
        audio_service.init();
        Self {
            waveform_visualizer: WaveformVisualizer::new(audio_service.clone()),
            spectrum_visualizer: SpectrumVisualizer::new(audio_service.clone()),
            audio_service,
            settings_shown: false,
            last_update: Instant::now(),
            visuals: WaveSyncVisuals {
                theme: catppuccin_egui::MOCHA
            },
        }
    }
}

impl eframe::App for WaveSync {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.last_update = Instant::now();
        catppuccin_egui::set_theme(ctx, self.visuals.theme);
        ctx.request_repaint();

        egui::TopBottomPanel::bottom("bottom_bar")
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Settings").clicked() {
                        self.settings_shown = true;
                    }
                    ui.separator();
                    for peak in self.audio_service.get_peak_labels() {
                        ui.label(peak);
                        ui.separator();
                    }
                    ui.label(format!("[{}]", self.audio_service.get_last_frame_size()));

                    let audio_backend = self.audio_service.audio_backend.lock().unwrap();
                    if let Some(dummy_backend) = audio_backend
                        .as_any()
                        .downcast_ref::<sound::dummy_audio_backend::DummyAudioBackend>(
                    ) {
                        let mut sequencer_frequency =
                            dummy_backend.sequencer_frequency.lock().unwrap();
                        egui::Slider::new(
                            &mut *sequencer_frequency,
                            RangeInclusive::from(egui::Rangef::new(20.0, 1000.0)),
                        )
                        .ui(ui);
                    }
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            StripBuilder::new(ui)
                .sizes(Size::remainder(), 2)
                .vertical(|mut strip| {
                    strip.cell(|ui| {
                        ui.add(VisualizerWidget::new(
                            Box::new(self.waveform_visualizer.clone()),
                            ctx,
                            &self.visuals
                        ));
                    });
                    strip.cell(|ui| {
                        ui.add(VisualizerWidget::new(
                            Box::new(self.spectrum_visualizer.clone()),
                            ctx,
                            &self.visuals
                        ));
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
                                for i in 8..17 {
                                    let v = 2usize.pow(i);
                                    ui.selectable_value(&mut selected, v, format!("{:?}", v));
                                }
                            });
                        self.audio_service.set_fft_size(selected);
                    });
                    ui.horizontal(|ui| {
                        ui.label("theme");
                        let selected = &mut self.visuals.theme;
                        egui::ComboBox::from_id_salt("theme")
                            .selected_text(theme_text(*selected))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(selected, catppuccin_egui::MOCHA, "Mocha");
                                ui.selectable_value(selected, catppuccin_egui::FRAPPE, "Frappe");
                                ui.selectable_value(selected, catppuccin_egui::LATTE, "Latte");
                                ui.selectable_value(selected, catppuccin_egui::MACCHIATO, "Machiato");
                            });
                    });
                    if ui.button("Close").clicked() {
                        self.settings_shown = false;
                    }
                });
        }
    }
}

fn theme_text(theme: catppuccin_egui::Theme) -> String {
    match theme {
        catppuccin_egui::MOCHA => "Mocha",
        catppuccin_egui::FRAPPE => "Frappe",
        catppuccin_egui::LATTE => "Latte",
        catppuccin_egui::MACCHIATO => "Machiato",
        _ => "None"
    }
    .to_string()
}