use crate::sound::audio_service::CHANNELS;
use crate::ui::plot::{Plot, PlotData, PolarPlot};
use crate::wavesync::WaveSyncVisuals;
use egui;
use egui::FontId;
use egui::emath::Align2;
use egui::epaint;
use egui::{Pos2, Rect, Response, Ui, Widget};
use egui_wgpu::ScreenDescriptor;
use egui_wgpu::wgpu;
use wgpu::{CommandEncoder, Device, Queue, TextureView};
use winit::window::Window;

pub struct RenderArgs<'a> {
    pub encoder: &'a mut CommandEncoder,
    pub window: &'a Window,
    pub queue: &'a Queue,
    pub device: &'a Device,
    pub window_surface_view: &'a TextureView,
    pub screen_descriptor: &'a ScreenDescriptor,
}

pub trait Visualizer: Send + Sync + 'static {
    fn get_plot_data(&self) -> PlotData;

    fn accept_fft(&self, _fft_data: &[Vec<f32>; CHANNELS], _fft_size: usize) {}

    /// If the visualizer is not able to function,
    /// for reasons such as there being not enough audio channels,
    /// it should return a message.
    ///
    /// If the message is returned, any drawing is skipped for the frame.
    fn error_message(&self, canvas_size: egui::Vec2) -> Option<String> {
        self.size_error(canvas_size)
    }

    /// Minimum usable canvas in logical pixels; axis margins are excluded.
    fn min_canvas_size(&self) -> egui::Vec2;

    /// Bound allocations and geometry work, including on high-DPI displays.
    fn size_error(&self, canvas_size: egui::Vec2) -> Option<String> {
        canvas_size_error(canvas_size, self.min_canvas_size(), 8192.0)
    }

    fn get_draw_callback(
        &self,
        rect: Rect,
        visuals: &WaveSyncVisuals,
    ) -> Option<epaint::PaintCallback>;

    fn get_post_egui_render(
        &self,
        _rect: Rect,
        _visuals: &WaveSyncVisuals,
    ) -> Option<Box<dyn PostEquiRender>> {
        None
    }

    fn draw_settings(&self, _ctx: &egui::Context) {}

    fn open_settings(&self) {}
}

/// Render callback for after the egui render pass, for additional render passes.
pub trait PostEquiRender {
    fn post_egui_render(&self, _args: &mut RenderArgs) {}
}

pub struct VisualizerWidget<'a> {
    visualizer: Box<dyn Visualizer>,
    ctx: &'a egui::Context,
    wavesync_visuals: &'a WaveSyncVisuals,
    cached_rect: Option<&'a mut Rect>,
    aspect_ratio: Option<f32>,
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
            cached_rect: None,
            aspect_ratio: None,
        }
    }

    pub fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.aspect_ratio = Some(ratio);
        self
    }

    pub fn view_rect(self, rect: &'a mut Rect) -> Self {
        Self {
            visualizer: self.visualizer,
            ctx: self.ctx,
            wavesync_visuals: self.wavesync_visuals,
            cached_rect: Some(rect),
            aspect_ratio: self.aspect_ratio,
        }
    }
}

impl<'a> Widget for VisualizerWidget<'a> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        if let Some(rect) = self.cached_rect.as_deref_mut() {
            *rect = Rect::ZERO;
        }
        let available = ui.available_size_before_wrap();
        // Avoid negative plot rectangles and axis tick calculations before the
        // exact canvas (after axis margins and aspect fitting) is available.
        if canvas_size_error(
            available,
            egui::vec2(40.0, 40.0),
            8192.0 / ui.pixels_per_point(),
        )
        .is_some()
        {
            let (rect, response) =
                ui.allocate_exact_size(available.max(egui::Vec2::ZERO), egui::Sense::empty());
            ui.painter().with_clip_rect(rect).text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Visualizer size out of range",
                FontId::default(),
                ui.visuals().text_color(),
            );
            self.visualizer.draw_settings(self.ctx);
            return response;
        }
        let plot_data = self.visualizer.get_plot_data();
        let content_rect = match plot_data {
            PlotData::XY(mut xy_data) => {
                let plot = Plot::new(&mut xy_data)
                    .aspect_ratio(self.aspect_ratio)
                    .set_grid_color(self.wavesync_visuals.plot_grid())
                    .set_label_color(ui.visuals().text_color())
                    .set_zero_line_color(self.wavesync_visuals.plot_grid_highlight())
                    .set_highlight_color(self.wavesync_visuals.color_end());
                plot.show(ui)
            }
            PlotData::Polar(radial_plot) => {
                let plot = PolarPlot::new(&radial_plot)
                    .aspect_ratio(self.aspect_ratio)
                    .set_grid_color(self.wavesync_visuals.plot_grid())
                    .set_label_color(ui.visuals().text_color())
                    .set_zero_line_color(self.wavesync_visuals.plot_grid_highlight())
                    .set_highlight_color(self.wavesync_visuals.color_end());
                plot.show(ui)
            }
        };

        let error = self.visualizer.error_message(content_rect.size());
        let valid = error.is_none();
        if let Some(err_message) = error {
            ui.painter()
                .with_clip_rect(content_rect.intersect(ui.clip_rect()))
                .text(
                    content_rect.center(),
                    Align2::CENTER_CENTER,
                    err_message,
                    FontId::default(),
                    ui.visuals().text_color(),
                );
        } else if let Some(callback) = self
            .visualizer
            .get_draw_callback(content_rect, self.wavesync_visuals)
        {
            ui.painter().add(callback);
        }

        if let Some(cached_rect) = self.cached_rect {
            if valid {
                cached_rect.clone_from(&content_rect);
            }
        }

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
            self.settings_open.store(
                !self.settings_open.load(Ordering::Acquire),
                Ordering::Release,
            );
        }
    };
}

fn canvas_size_error(size: egui::Vec2, min: egui::Vec2, max: f32) -> Option<String> {
    if !size.is_finite() || size.x < min.x || size.y < min.y {
        Some(format!("Canvas too small (min {} × {})", min.x, min.y))
    } else if size.x > max || size.y > max {
        Some(format!("Canvas too large (max {max} × {max})"))
    } else {
        None
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;
    #[test]
    fn rejects_degenerate_and_oversized_canvases() {
        let min = egui::vec2(64.0, 24.0);
        for size in [
            egui::Vec2::ZERO,
            egui::vec2(-1.0, 100.0),
            egui::vec2(f32::NAN, 100.0),
            egui::vec2(f32::INFINITY, 100.0),
            egui::vec2(8193.0, 100.0),
            egui::vec2(64.0, 23.0),
        ] {
            assert!(canvas_size_error(size, min, 8192.0).is_some());
        }
        assert!(canvas_size_error(min, min, 8192.0).is_none());
        assert!(canvas_size_error(egui::vec2(8192.0, 8192.0), min, 8192.0).is_none());
    }
}
