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
}

impl<'a> Plot<'a> {
    pub fn new(data: &'a mut XYPlotData) -> Self {
        Self {
            plot_data: data,
            grid_color: Color32::from_gray(50),
            label_color: Color32::from_gray(200),
            zero_line_color: Color32::from_gray(255),
            highlight_color: Color32::from_rgb(255, 0, 0),
        }
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
            highlight_color: self.highlight_color
        };
        let mut occupied_label_rects = Vec::new();
        let content_rect = rect.sub(Margin {
            left: if self.plot_data.y_axis_shown {
                Y_AXIS_WIDTH
            } else {
                MARGIN
            },
            right: MARGIN,
            top: MARGIN,
            bottom: if self.plot_data.x_axis_shown {
                X_AXIS_WIDTH
            } else {
                MARGIN
            },
        });

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
