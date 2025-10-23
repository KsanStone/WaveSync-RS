use crate::sound::audio_service::CHANNELS;
use crate::ui::plot::{Plot, PlotData};
use crate::wavesync::WaveSyncVisuals;
use eframe::egui::{Pos2, Rect, Response, Ui, Widget};
use eframe::wgpu::{CommandEncoder, Device, Queue, TextureView};
use eframe::{egui, epaint};
use egui_wgpu::{CallbackResources, ScreenDescriptor};
use winit::window::Window;

pub struct RenderArgs<'a> {
    pub encoder: &'a mut CommandEncoder,
    pub window: &'a Window,
    pub queue: &'a Queue,
    pub device: &'a Device,
    pub resources: &'a CallbackResources,
    pub window_surface_view: &'a TextureView,
    pub screen_descriptor: &'a ScreenDescriptor,
}

pub trait Visualizer: Send + Sync + 'static {
    fn get_plot_data(&self) -> PlotData;

    fn accept_fft(&self, _fft_data: &[Vec<f32>; CHANNELS], _fft_size: usize) {}

    fn get_draw_callback(
        &self,
        rect: Rect,
        visuals: &WaveSyncVisuals,
    ) -> Option<epaint::PaintCallback>;

    fn get_post_egui_render(
        &self,
        rect: Rect,
        visuals: &WaveSyncVisuals,
    ) -> Option<Box<dyn PostEquiRender>> {
        None
    }

    fn draw_settings(&self, ctx: &egui::Context) {}

    fn open_settings(&self) {}
}

/// Render callback for after the egui render pass, for additional render passes.
pub trait PostEquiRender {
    fn post_egui_render(&self, args: &mut RenderArgs) {}
}

pub struct VisualizerWidget<'a> {
    visualizer: Box<dyn Visualizer>,
    ctx: &'a egui::Context,
    wavesync_visuals: &'a WaveSyncVisuals,
    cached_rect: &'a mut Rect,
}

impl<'a> VisualizerWidget<'a> {
    pub fn new(
        visualizer: Box<dyn Visualizer + 'static>,
        ctx: &'a egui::Context,
        wavesync_visuals: &'a WaveSyncVisuals,
        rect: &'a mut Rect,
    ) -> Self {
        Self {
            visualizer,
            ctx,
            wavesync_visuals,
            cached_rect: rect,
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
        if let Some(callback) = self
            .visualizer
            .get_draw_callback(content_rect, self.wavesync_visuals)
        {
            ui.painter().add(callback);
        }

        self.cached_rect.clone_from(&content_rect);

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
