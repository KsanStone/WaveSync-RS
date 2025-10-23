use crate::wavesync::WaveSyncVisuals;
use crate::ui::plot::{Plot, PlotData};
use eframe::egui::{Pos2, Rect, Response, Ui, Widget};
use eframe::{egui, epaint};
use crate::sound::audio_service::CHANNELS;

pub trait Visualizer: Send + Sync + 'static {
    fn get_plot_data(&self) -> PlotData;

    fn accept_fft(&self, _fft_data: &[Vec<f32>; CHANNELS], _fft_size: usize) {}

    fn get_draw_callback(&self, rect: Rect, visuals: &WaveSyncVisuals) -> epaint::PaintCallback;

    fn draw_settings(&self, ctx: &egui::Context) {}

    fn open_settings(&self) {}
}

pub struct VisualizerWidget<'a> {
    visualizer: Box<dyn Visualizer>,
    ctx: &'a egui::Context,
    wavesync_visuals: &'a WaveSyncVisuals,
}

impl<'a> VisualizerWidget<'a> {
    pub fn new(
        visualizer: Box<dyn Visualizer + 'static>,
        ctx: &'a egui::Context,
        wavesync_visuals: &'a WaveSyncVisuals,
    ) -> Self {
        Self {
            visualizer,
            ctx,
            wavesync_visuals,
        }
    }
}

impl<'a> Widget for VisualizerWidget<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let mut plot_data = self.visualizer.get_plot_data();
        let plot = Plot::new(&mut plot_data)
            .set_grid_color(ui.visuals().faint_bg_color)
            .set_label_color(ui.visuals().text_color());
        let content_rect = plot.show(ui);
        ui.painter().add(
            self.visualizer
                .get_draw_callback(content_rect, self.wavesync_visuals),
        );

        let settings_rect = Rect::from_min_max(
            Pos2::new(content_rect.max.x - 22.0, content_rect.min.y + 2.0),
            Pos2::new(content_rect.max.x - 2.0, content_rect.min.y + 22.0),
        );

        ui.put(settings_rect, |ui: &mut Ui| {
            if ui.button(egui_phosphor::regular::GEAR).clicked() {
                self.visualizer.open_settings();
            }
            self.visualizer.draw_settings(self.ctx);
            ui.response()
        });

        ui.response()
    }
}

#[macro_export]
macro_rules! impl_settings {
    ($name:expr, $ui:ident, $this:ident, $draw_callback:expr) => {
        fn draw_settings(&self, ctx: &egui::Context) {
            if self.settings_open.load(Ordering::Acquire) {
                egui::Window::new($name)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .show(ctx, |ui| {
                        (|$ui: &mut Ui, $this: &Self| $draw_callback)(ui, self);

                        if ui.button("Close").clicked() {
                            self.settings_open
                                .store(false, std::sync::atomic::Ordering::Release);
                        }
                    });
            }
        }

        fn open_settings(&self) {
            self.settings_open
                .store(true, std::sync::atomic::Ordering::Release);
        }
    };
}
