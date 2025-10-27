#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use crate::app::AppHandler;
use crate::persistance::{APP_KEY, Persistence};
use crate::sound;
use crate::sound::AudioChannel;
use crate::sound::audio_service::CHANNELS;
use crate::ui::gradient::{Gradient, Stop};
use crate::ui::visualizer::VisualizerType;
use crate::ui::visualizer::spectrogram::{SpectrogramSettings, SpectrogramVisualizer};
use crate::ui::visualizer::spectrum::{SpectrumVisualizer, SpectrumVisualizerSettings};
use crate::ui::visualizer::vectorscope::{VectorscopeSettings, VectorscopeVisualizer};
use crate::ui::visualizer::visualizer_widget::{RenderArgs, Visualizer, VisualizerWidget};
use crate::ui::visualizer::waveform::{WaveformSettings, WaveformVisualizer};
use egui;
use egui::Rect;
use egui::{Color32, Sense, Vec2, Widget};
use egui_extras::{Size, StripBuilder};
use serde::{Deserialize, Serialize};
use std::default::Default;
use std::ops::RangeInclusive;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

pub type VisualizerBoundCache = std::collections::HashMap<(VisualizerType, AudioChannel), Rect>;

pub struct WaveSync {
    audio_service: sound::audio_service::AudioService,
    waveform_visualizers: Vec<WaveformVisualizer>,
    spectrum_visualizers: Vec<SpectrumVisualizer>,
    spectrogram_visualizer: SpectrogramVisualizer,
    vectorscope_visualizer: VectorscopeVisualizer,
    settings_shown: bool,
    last_update: Instant,
    visuals: WaveSyncVisuals,
    data: Arc<RwLock<WaveSyncAppData>>,
    visualizer_bounds: VisualizerBoundCache,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct WaveSyncAppData {
    pub waveform_settings: WaveformSettings,
    pub spectrum_settings: SpectrumVisualizerSettings,
    pub spectrogram_settings: SpectrogramSettings,
    pub vectorscope_settings: VectorscopeSettings,
    pub fft_rate: u32,
    pub fft_size: usize,
    pub theme_name: String,
}

pub struct WaveSyncVisuals {
    theme: catppuccin_egui::Theme,
}

impl WaveSyncVisuals {
    pub fn wave_color(&self) -> Color32 {
        self.theme.blue
    }

    pub fn color_start(&self) -> Color32 {
        self.theme.base
    }

    pub fn color_end(&self) -> Color32 {
        self.theme.blue
    }

    pub fn plot_grid(&self) -> Color32 {
        self.theme.surface0
    }

    pub fn plot_grid_highlight(&self) -> Color32 {
        self.theme.surface2
    }

    pub fn spectrogram_gradient(&self) -> Gradient {
        Gradient::new(vec![
            Stop::new(0.0, self.theme.base),
            Stop::new(1.0, self.theme.blue),
        ])
        .unwrap()
    }
}

impl WaveSync {
    pub fn new(data: WaveSyncAppData) -> Self {
        let audio_service = sound::audio_service::AudioService::new();

        audio_service.init();
        audio_service.update_fft_plan(data.fft_size);
        audio_service.update_fft_rate(data.fft_rate);
        let theme_name = data.theme_name.clone();
        let data = Arc::new(RwLock::new(data));

        let mut waveform_visualizers = vec![];
        let mut spectrum_visualizers = vec![];

        for channel in 0..CHANNELS {
            let channel = AudioChannel::try_from(channel as u32).unwrap();
            waveform_visualizers.push(WaveformVisualizer::new(
                audio_service.clone(),
                channel,
                data.clone(),
            ));
            spectrum_visualizers.push(SpectrumVisualizer::new(
                audio_service.clone(),
                channel,
                data.clone(),
            ));
        }

        let spectrogram_visualizer =
            SpectrogramVisualizer::new(audio_service.clone(), AudioChannel::Master, data.clone());
        let vectorscope_visualizer =
            VectorscopeVisualizer::new(audio_service.clone(), data.clone());

        // Note: this IS a circular reference
        // But its fine as both the vis, and the audio service, will never be dropper,
        // so its fine that they will always be alive in memory.
        audio_service.register_fft_listener(Box::new(spectrogram_visualizer.clone()));

        Self {
            waveform_visualizers,
            spectrum_visualizers,
            spectrogram_visualizer,
            vectorscope_visualizer,
            audio_service,
            settings_shown: false,
            last_update: Instant::now(),
            visuals: WaveSyncVisuals {
                theme: theme_from_text(&theme_name),
            },
            data,
            visualizer_bounds: Default::default(),
        }
    }

    fn mono_layout(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        StripBuilder::new(ui)
            .sizes(Size::remainder(), 2)
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    StripBuilder::new(ui)
                        .sizes(Size::remainder(), 2)
                        .vertical(|mut strip| {
                            strip.cell(|ui| {
                                ui.add(VisualizerWidget::new(
                                    Box::new(self.spectrum_visualizers[0].clone()),
                                    ctx,
                                    &self.visuals,
                                ));
                            });

                            strip.cell(|ui| {
                                ui.add(VisualizerWidget::new(
                                    Box::new(self.spectrogram_visualizer.clone()),
                                    ctx,
                                    &self.visuals,
                                ));
                            });
                        });
                });
                strip.cell(|ui| {
                    StripBuilder::new(ui)
                        .sizes(Size::remainder(), 2)
                        .vertical(|mut strip| {
                            strip.cell(|ui| {
                                let mut rect = Rect::ZERO;
                                ui.add(
                                    VisualizerWidget::new(
                                        Box::new(self.vectorscope_visualizer.clone()),
                                        ctx,
                                        &self.visuals,
                                    )
                                    .view_rect(&mut rect),
                                );
                                self.visualizer_bounds.insert(
                                    (VisualizerType::Vectorscope, AudioChannel::Master),
                                    rect,
                                );
                            });

                            strip.cell(|ui| {
                                ui.add(VisualizerWidget::new(
                                    Box::new(self.waveform_visualizers[0].clone()),
                                    ctx,
                                    &self.visuals,
                                ));
                            });
                        });
                });
            });
    }

    fn stereo_layout(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        StripBuilder::new(ui)
            .sizes(Size::remainder(), 3)
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    StripBuilder::new(ui)
                        .sizes(Size::remainder(), 2)
                        .vertical(|mut strip| {
                            strip.cell(|ui| {
                                let mut rect = Rect::ZERO;
                                ui.add(
                                    VisualizerWidget::new(
                                        Box::new(self.vectorscope_visualizer.clone()),
                                        ctx,
                                        &self.visuals,
                                    )
                                    .view_rect(&mut rect),
                                );
                                self.visualizer_bounds.insert(
                                    (VisualizerType::Vectorscope, AudioChannel::Master),
                                    rect,
                                );
                            });

                            strip.cell(|ui| {
                                ui.add(VisualizerWidget::new(
                                    Box::new(self.spectrogram_visualizer.clone()),
                                    ctx,
                                    &self.visuals,
                                ));
                            });
                        });
                });

                for i in 1..3 {
                    strip.cell(|ui| {
                        StripBuilder::new(ui)
                            .sizes(Size::remainder(), 2)
                            .vertical(|mut strip| {
                                strip.cell(|ui| {
                                    ui.add(VisualizerWidget::new(
                                        Box::new(self.waveform_visualizers[i].clone()),
                                        ctx,
                                        &self.visuals,
                                    ));
                                });

                                strip.cell(|ui| {
                                    ui.add(VisualizerWidget::new(
                                        Box::new(self.spectrum_visualizers[i].clone()),
                                        ctx,
                                        &self.visuals,
                                    ));
                                });
                            });
                    });
                }
            });
    }
}

impl AppHandler for WaveSync {
    fn update(&mut self, ctx: &egui::Context) {
        self.last_update = Instant::now();
        catppuccin_egui::set_theme(ctx, self.visuals.theme);
        ctx.request_repaint();

        egui::TopBottomPanel::bottom("bottom_bar")
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(egui_phosphor::regular::GEAR).clicked() {
                        self.settings_shown = true;
                    }
                    ui.separator();

                    {
                        let sources = self.audio_service.available_sources.lock().unwrap();
                        let mut current_value = self.audio_service.get_source().id;
                        egui::ComboBox::from_id_salt("source_select")
                            .selected_text(self.audio_service.get_source().name)
                            .show_ui(ui, |ui| {
                                for source in &*sources {
                                    ui.selectable_value(
                                        &mut current_value,
                                        source.id.clone(),
                                        source.name.clone(),
                                    );
                                }
                            });
                        drop(sources);
                        self.audio_service.update_source(current_value);
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
                        let mut sequencer_frequency = dummy_backend
                            .pattern_data
                            .sequencer_frequency
                            .lock()
                            .unwrap();
                        egui::Slider::new(
                            &mut *sequencer_frequency,
                            RangeInclusive::from(egui::Rangef::new(20.0, 1000.0)),
                        )
                        .ui(ui);
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.audio_service.get_active_audio_channels() == 1 {
                self.mono_layout(ui, ctx);
            } else {
                self.stereo_layout(ui, ctx);
            }
        });
        if self.settings_shown {
            egui::Window::new("Settings")
                .collapsible(false)
                .resizable(false)
                .min_width(300.0)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
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
                                ui.selectable_value(
                                    selected,
                                    catppuccin_egui::MACCHIATO,
                                    "Machiato",
                                );
                                ui.selectable_value(selected, catppuccin_egui::LATTE, "Latte");
                            });
                    });
                    let mut val = self.audio_service.fft_rate.load(std::sync::atomic::Ordering::Acquire);
                    ui.horizontal(|ui| {
                        ui.label("Fft rate");
                        ui.add(egui::Slider::new(&mut val, 1..=200));
                        self.audio_service.fft_rate.store(val, Ordering::Release);
                    });
                    let min_accurate_freq = self.audio_service.get_source().calculate_frequency_resolution(self.audio_service.get_fft_size());
                        let duration_between_fft = Duration::from_secs_f64(1.0 / val as f64);
                        ui.label(format!("Fft interval: {duration_between_fft:?} Min accurate freq: {min_accurate_freq:.1}Hz"));
                    if ui.button("Close").clicked() {
                        self.settings_shown = false;
                    }
                    ui.allocate_at_least(Vec2::new(ui.available_width(), 0.0), Sense::empty());
                });
        }
    }

    fn save(&mut self, persistence: &mut Persistence) {
        let mut data = self.data.write().unwrap();

        data.fft_size = self.audio_service.get_fft_size();
        data.fft_rate = self.audio_service.fft_rate.load(Ordering::Acquire);
        data.theme_name = theme_text(self.visuals.theme);

        persistence.set(APP_KEY, &*data);
    }

    fn post_egui(&mut self, mut args: RenderArgs) {
        let rect = self
            .visualizer_bounds
            .get(&(VisualizerType::Vectorscope, AudioChannel::Master))
            .cloned();
        if let Some(rect) = rect {
            let vectorscope_callback = &self
                .vectorscope_visualizer
                .get_post_egui_render(rect, &self.visuals);
            if let Some(callback) = vectorscope_callback {
                callback.post_egui_render(&mut args);
            }
        }
    }
}

/// Format a number such that its length doesn't vary and jitter the ui
pub fn stable_num(length: usize, decimals: usize, val: f32) -> String {
    let val = format!("{:.1$}", val, decimals);
    let missing_length = length.saturating_sub(val.len());
    let pad = "-".repeat(missing_length);

    format!("{}{}", pad, val)
}

fn theme_text(theme: catppuccin_egui::Theme) -> String {
    match theme {
        catppuccin_egui::MOCHA => "Mocha",
        catppuccin_egui::FRAPPE => "Frappe",
        catppuccin_egui::LATTE => "Latte",
        catppuccin_egui::MACCHIATO => "Machiato",
        _ => "None",
    }
    .to_string()
}

fn theme_from_text(text: &str) -> catppuccin_egui::Theme {
    match text {
        "Mocha" => catppuccin_egui::MOCHA,
        "Frappe" => catppuccin_egui::FRAPPE,
        "Latte" => catppuccin_egui::LATTE,
        "Machiato" => catppuccin_egui::MACCHIATO,
        _ => catppuccin_egui::MOCHA,
    }
}
