use egui::epaint::Color32;
use egui::{Margin, Rect, Sense, Ui};
use std::ops::Sub;

mod axis;
mod polar;

pub use crate::ui::plot::axis::{Axis, AxisTicks};
pub use crate::ui::plot::polar::*;
use axis::{AxisOrientation, AxisPaintStyle, paint_axis};

const X_AXIS_WIDTH: i8 = 20;
const Y_AXIS_WIDTH: i8 = 30;
const MARGIN: i8 = 5;

#[derive(Clone)]
pub enum PlotData {
    XY(XYPlotData),
    Polar(PolarPlotData),
}

impl PlotData {
    pub fn expect_xy(&self) -> XYPlotData {
        match self {
            PlotData::XY(data) => data.clone(),
            _ => panic!("Expected XYPlotData, but got PolarPlotData"),
        }
    }

    pub fn expect_polar(&self) -> PolarPlotData {
        match self {
            PlotData::Polar(data) => data.clone(),
            _ => panic!("Expected PolarPlotData, but got XYPlotData"),
        }
    }
}

#[derive(Clone)]
pub struct XYPlotData {
    pub x_axis: Axis,
    pub x_axis_shown: bool,
    pub x_axis_grid_lines_shown: bool,

    pub y_axis: Axis,
    pub y_axis_shown: bool,
    pub y_axis_grid_lines_shown: bool,
}

impl Default for XYPlotData {
    fn default() -> Self {
        Self {
            x_axis: Axis {
                min: 0.0,
                max: 100.0,
                logarithmic: false,
                always_show_zero: false,
                highlight_values: Vec::new(),
            },
            x_axis_shown: true,
            y_axis: Axis {
                min: 0.0,
                max: 100.0,
                logarithmic: false,
                always_show_zero: false,
                highlight_values: Vec::new(),
            },
            y_axis_shown: true,
            x_axis_grid_lines_shown: true,
            y_axis_grid_lines_shown: true,
        }
    }
}

impl XYPlotData {
    pub fn canvas_margin(&self) -> Margin {
        Margin {
            left: if self.y_axis_shown {
                Y_AXIS_WIDTH
            } else {
                MARGIN
            },
            right: MARGIN,
            top: MARGIN,
            bottom: if self.x_axis_shown {
                X_AXIS_WIDTH
            } else {
                MARGIN
            },
        }
    }

    pub fn from_axis(x_axis: Axis, y_axis: Axis) -> Self {
        Self {
            x_axis,
            x_axis_shown: true,
            x_axis_grid_lines_shown: true,
            y_axis,
            y_axis_shown: true,
            y_axis_grid_lines_shown: true,
        }
    }

    #[allow(unused)]
    pub fn x_axis_shown(mut self, x_axis_shown: bool) -> Self {
        self.x_axis_shown = x_axis_shown;
        self
    }

    pub fn y_axis_shown(mut self, y_axis_shown: bool) -> Self {
        self.y_axis_shown = y_axis_shown;
        self
    }

    pub fn x_axis_grid_lines_shown(mut self, x_axis_grid_lines_shown: bool) -> Self {
        self.x_axis_grid_lines_shown = x_axis_grid_lines_shown;
        self
    }

    pub fn y_axis_grid_lines_shown(mut self, y_axis_grid_lines_shown: bool) -> Self {
        self.y_axis_grid_lines_shown = y_axis_grid_lines_shown;
        self
    }
}

/// Generic plot, to be used as a background for the visualizers.
pub struct Plot<'a> {
    plot_data: &'a mut XYPlotData,
    grid_color: Color32,
    label_color: Color32,
    zero_line_color: Color32,
    highlight_color: Color32,
    aspect_ratio: Option<f32>,
}

impl<'a> Plot<'a> {
    pub fn new(data: &'a mut XYPlotData) -> Self {
        Self {
            plot_data: data,
            grid_color: Color32::from_gray(50),
            label_color: Color32::from_gray(200),
            zero_line_color: Color32::from_gray(255),
            highlight_color: Color32::from_rgb(255, 0, 0),
            aspect_ratio: None,
        }
    }

    pub fn aspect_ratio(mut self, ratio: Option<f32>) -> Self {
        self.aspect_ratio = ratio;
        self
    }

    pub fn set_grid_color(mut self, color: Color32) -> Self {
        self.grid_color = color;
        self
    }

    pub fn set_label_color(mut self, color: Color32) -> Self {
        self.label_color = color;
        self
    }

    pub fn set_zero_line_color(mut self, color: Color32) -> Self {
        self.zero_line_color = color;
        self
    }

    pub fn set_highlight_color(mut self, color: Color32) -> Self {
        self.highlight_color = color;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Rect {
        let (rect, _) = ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::empty());

        let painter = ui.painter();
        let style = AxisPaintStyle {
            grid_color: self.grid_color,
            label_color: self.label_color,
            zero_line_color: self.zero_line_color,
            highlight_color: self.highlight_color,
        };
        let mut occupied_label_rects = Vec::new();
        let content_rect = rect.sub(self.plot_data.canvas_margin());

        let content_rect = fit_plot_aspect(content_rect, self.aspect_ratio);

        for (axis, shown, grid_shown, orientation) in [
            (
                &self.plot_data.x_axis,
                self.plot_data.x_axis_shown,
                self.plot_data.x_axis_grid_lines_shown,
                AxisOrientation::Horizontal,
            ),
            (
                &self.plot_data.y_axis,
                self.plot_data.y_axis_shown,
                self.plot_data.y_axis_grid_lines_shown,
                AxisOrientation::Vertical,
            ),
        ] {
            if shown {
                paint_axis(
                    painter,
                    axis,
                    content_rect,
                    orientation,
                    grid_shown,
                    &style,
                    &mut occupied_label_rects,
                );
            }
        }

        content_rect
    }
}

/// Fit the actual plotting area, leaving axis labels outside the constrained rectangle.
fn fit_plot_aspect(rect: Rect, ratio: Option<f32>) -> Rect {
    let Some(ratio) = ratio.filter(|r| r.is_finite() && *r > 0.0) else {
        return rect;
    };
    let width = rect.width().max(0.0).min(rect.height().max(0.0) * ratio);
    Rect::from_center_size(rect.center(), egui::vec2(width, width / ratio))
}

#[cfg(test)]
mod aspect_tests {
    use super::*;

    #[test]
    fn square_canvas_accounts_for_unequal_axis_margins() {
        let ctx = egui::Context::default();
        let mut data = XYPlotData::default();
        let margin = data.canvas_margin();
        let outer_size = egui::vec2(300.0, 300.0) + margin.sum();
        assert_ne!(outer_size.x, outer_size.y);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 1000.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_size(ui.cursor().min, outer_size)),
                    );
                    let canvas = Plot::new(&mut data)
                        .aspect_ratio(Some(1.0))
                        .show(&mut child);
                    assert!((canvas.width() - 300.0).abs() < 0.001);
                    assert!((canvas.height() - 300.0).abs() < 0.001);
                });
            },
        );
    }

    #[test]
    fn constrained_plots_fit_wide_tall_and_small_cells() {
        for size in [
            egui::vec2(600.0, 220.0),
            egui::vec2(180.0, 500.0),
            egui::vec2(24.0, 12.0),
        ] {
            let bounds = Rect::from_min_size(egui::pos2(35.0, 25.0), size);
            for ratio in [1.0, 2.0] {
                let plot = fit_plot_aspect(bounds, Some(ratio));
                assert!((plot.width() / plot.height() - ratio).abs() < 0.0001);
                assert!(bounds.contains_rect(plot));
                assert_eq!(plot.center(), bounds.center());
            }
            assert_eq!(fit_plot_aspect(bounds, None), bounds);
        }
    }
}
