use eframe::egui::{Align2, FontId, Margin, Response, Sense, Ui};
use eframe::emath::Pos2;
use eframe::epaint::Color32;
use egui_wgpu::CallbackTrait;
use std::ops::Sub;

const PLOT_DIGITS: usize = 2;

#[derive(Clone)]
pub struct PlotData {
    pub x_axis: Option<Axis>,
    pub x_axis_grid: bool,
    pub y_axis: Option<Axis>,
    pub y_axis_grid: bool,
}

impl Default for PlotData {
    fn default() -> Self {
        Self {
            x_axis: Some(Axis {
                min: 0.0,
                max: 100.0,
                logarithmic: false,
            }),
            x_axis_grid: true,
            y_axis: Some(Axis {
                min: 0.0,
                max: 100.0,
                logarithmic: false,
            }),
            y_axis_grid: true,
        }
    }
}

#[derive(Default, Clone)]
pub struct Axis {
    pub min: f32,
    pub max: f32,
    pub logarithmic: bool,
}

impl Axis {
    pub fn new(min: f32, max: f32, logarithmic: bool) -> Self {
        Self {
            min,
            max,
            logarithmic,
        }
    }

    /// Returns the positions for ticks in axis-space
    pub fn tick_positions(&self, px_size: f32) -> Vec<f32> {
        const LABEL_SPACING_PX: f32 = 60.0;
        let labels = (px_size / LABEL_SPACING_PX + 1.0).max(2.0).floor();
        let step = (self.max - self.min) / (labels - 1.0);

        let mut pos = self.min;
        let mut label_positons = vec![];
        for _ in 0..labels as usize {
            label_positons.push(pos);
            pos += step;
        }
        label_positons
    }

    /// Maps a value in axis-space to a pixel position
    pub fn px_pos(&self, value: f32, px_min: f32, px_max: f32) -> f32 {
        let range = self.max - self.min;
        let pos = (value - self.min) / range * (px_max - px_min);
        pos + px_min
    }
}

pub struct GpuPlot<'a> {
    plot_data: &'a mut PlotData,
    grid_color: Color32,
    label_color: Color32,
}

impl<'a> GpuPlot<'a> {
    pub fn new(data: &'a mut PlotData) -> Self {
        Self {
            plot_data: data,
            grid_color: Color32::from_gray(50),
            label_color: Color32::from_gray(200),
        }
    }

    pub fn show(self, ui: &mut Ui, paint_callback: impl CallbackTrait + 'static) -> Response {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::empty());

        const AXIS_WIDTH: i8 = 30;

        let painter = ui.painter();
        let content_rect = rect.sub(Margin {
            left: if self.plot_data.y_axis.is_some() {
                AXIS_WIDTH
            } else {
                0
            },
            right: 0,
            top: 0,
            bottom: if self.plot_data.x_axis.is_some() {
                AXIS_WIDTH
            } else {
                0
            },
        });

        if let Some(axis) = self.plot_data.x_axis.as_ref() {
            for pos in axis.tick_positions(content_rect.width()) {
                let px_pos = axis.px_pos(pos, content_rect.min.x, content_rect.max.x);
                painter.line_segment(
                    [
                        Pos2::new(px_pos, content_rect.min.y),
                        Pos2::new(px_pos, content_rect.max.y),
                    ],
                    (1.0, self.grid_color),
                );
                painter.text(
                    Pos2::new(px_pos, content_rect.max.y + 2.0),
                    Align2::CENTER_TOP,
                    format_significant(pos, PLOT_DIGITS),
                    FontId::default(),
                    self.label_color,
                );
            }
        }

        if let Some(axis) = self.plot_data.y_axis.as_ref() {
            for pos in axis.tick_positions(content_rect.height()) {
                let px_pos = axis.px_pos(pos, content_rect.min.y, content_rect.max.y);
                painter.line_segment(
                    [
                        Pos2::new(content_rect.min.x, px_pos),
                        Pos2::new(content_rect.max.x, px_pos),
                    ],
                    (1.0, self.grid_color),
                );
                painter.text(
                    Pos2::new(rect.min.x + 2.0, px_pos),
                    Align2::LEFT_CENTER,
                    format_significant(pos, PLOT_DIGITS),
                    FontId::default(),
                    self.label_color,
                );
            }
        }

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            content_rect,
            paint_callback,
        ));

        response
    }
}

fn format_significant(x: f32, n: usize) -> String {
    if x.abs() < 0.00001 {
        return format!("{:.1}", 0.0);
    }
    let log = x.abs().log10().floor();
    let scale = 10f32.powf(n as f32 - log - 1.0);
    let rounded = (x * scale).round() / scale;
    let s = format!("{}", rounded);
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}
