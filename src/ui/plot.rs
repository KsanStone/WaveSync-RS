use egui::emath::Pos2;
use egui::epaint::Color32;
use egui::{Align2, FontFamily, FontId, Margin, Painter, Rect, Sense, Ui};
use std::ops::Sub;

const PLOT_DIGITS: usize = 2;
const X_AXIS_WIDTH: i8 = 20;
const Y_AXIS_WIDTH: i8 = 30;
const TICK_SIZE: f32 = 4.0;
const FONT_SIZE: f32 = 9.0;
const LABEL_SPACING: f32 = 2.0;
const MARGIN: i8 = 5;

#[derive(Clone)]
pub struct PlotData {
    pub x_axis: Axis,
    pub x_axis_shown: bool,
    pub y_axis: Axis,
    pub y_axis_shown: bool,
}

impl Default for PlotData {
    fn default() -> Self {
        Self {
            x_axis: Axis {
                min: 0.0,
                max: 100.0,
                logarithmic: false,
                always_show_zero: false,
            },
            x_axis_shown: true,
            y_axis: Axis {
                min: 0.0,
                max: 100.0,
                logarithmic: false,
                always_show_zero: false,
            },
            y_axis_shown: true,
        }
    }
}

impl PlotData {
    pub fn from_axis(x_axis: Axis, y_axis: Axis) -> Self {
        Self {
            x_axis,
            x_axis_shown: true,
            y_axis,
            y_axis_shown: true,
        }
    }

    #[allow(unused)]
    pub fn x_axis_shown(mut self, x_axis_shown: bool) -> Self {
        self.x_axis_shown = x_axis_shown;
        self
    }

    #[allow(unused)]
    pub fn y_axis_shown(mut self, y_axis_shown: bool) -> Self {
        self.y_axis_shown = y_axis_shown;
        self
    }
}

#[derive(Default, Clone)]
pub struct Axis {
    pub min: f32,
    pub max: f32,
    pub logarithmic: bool,
    pub always_show_zero: bool,
}

pub struct AxisTicks {
    pub major: Vec<f32>,
    pub minor: Vec<f32>,
}

impl Axis {
    pub fn linear(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            logarithmic: false,
            always_show_zero: false,
        }
    }

    pub fn logarithmic(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            logarithmic: true,
            always_show_zero: false,
        }
    }

    pub fn always_show_zero(mut self, show: bool) -> Self {
        self.always_show_zero = show;
        self
    }

    pub fn log_lower_bound(&self) -> f32 {
        Self::log(self.min)
    }

    pub fn log_upper_bound(&self) -> f32 {
        Self::log(self.max)
    }

    pub fn range(&self) -> f32 {
        self.max - self.min
    }

    /// Returns the positions for ticks in axis-space
    pub fn tick_positions(&self, px_size: f32) -> AxisTicks {
        let mut major_ticks = vec![];
        let mut minor_ticks = vec![];
        const MINOR_TICK_COUNT: usize = 5;

        if !self.logarithmic {
            const LABEL_SPACING_PX: f32 = 60.0;
            let include_zero = self.always_show_zero && self.min < 0.0 && self.max > 0.0;

            let mut labels = (px_size / LABEL_SPACING_PX + 1.0).max(2.0).floor();
            if include_zero && (labels as u32).is_multiple_of(2) {
                labels += 1.0;
            }
            let step = (self.max - self.min) / (labels - 1.0);
            let minor_step = step / (MINOR_TICK_COUNT as f32 + 1.0);

            let mut pos = self.min;
            for i in 0..labels as usize {
                major_ticks.push(pos);
                if i + 1 != labels as usize {
                    let mut tmp = pos;
                    for _ in 0..MINOR_TICK_COUNT {
                        tmp += minor_step;
                        minor_ticks.push(tmp);
                    }
                }
                pos += step;
            }

            // Ensure endpoints
            if major_ticks.is_empty() || *major_ticks.first().unwrap() != self.min {
                major_ticks.insert(0, self.min);
            }
            if major_ticks.is_empty() || *major_ticks.last().unwrap() != self.max {
                major_ticks.push(self.max);
            }
        } else {
            let mut i = 0.0;

            while i <= self.log_upper_bound() {
                for j in 1..=9 {
                    let v = j as f32 * 10.0f32.powf(i);
                    if (v >= self.min) && (v <= self.max) {
                        major_ticks.push(v);
                    }
                }
                i += 1.0
            }

            i = 0.0;
            while i <= self.log_upper_bound() {
                let mut j = 0.0;
                while j <= 9.0 {
                    let value = j * 10.0f32.powf(i);
                    if (value >= self.min) && (value <= self.max) {
                        minor_ticks.push(value);
                    }
                    j += 1.0 / MINOR_TICK_COUNT as f32
                }
                i += 1.0
            }

            if major_ticks.is_empty() || (major_ticks[0] - self.min).abs() > 0.001 {
                major_ticks.insert(0, self.min);
            }
        }
        AxisTicks {
            major: major_ticks,
            minor: minor_ticks,
        }
    }

    /// Maps a value in axis-space to a pixel position
    pub fn val_to_pos(&self, value: f32, pos_min: f32, pos_max: f32) -> f32 {
        if !self.logarithmic {
            let range = self.max - self.min;
            let pos = (value - self.min) / range * (pos_max - pos_min);
            pos + pos_min
        } else {
            let delta = self.log_upper_bound() - self.log_lower_bound();
            let delta_v = Self::log(value) - self.log_lower_bound();
            (delta_v) / delta * (pos_max - pos_min) + pos_min
        }
    }

    /// Utility to convert to gl coordinate space
    pub fn gl_pos(&self, value: f32) -> f32 {
        self.val_to_pos(value, -1.0, 1.0)
    }

    /// [0, 1] ranged pos
    pub fn norm_pos(&self, value: f32) -> f32 {
        self.val_to_pos(value, 0.0, 1.0)
    }

    /// map a [0, 1] position to this axis' value
    pub fn norm_pos_to_val(&self, value: f32) -> f32 {
        if !self.logarithmic {
            self.min + value * (self.max - self.min)
        } else {
            let delta = self.log_upper_bound() - self.log_lower_bound();
            let delta_v = value * delta;
            10.0f32.powf(self.log_lower_bound() + delta_v)
        }
    }

    fn log(val: f32) -> f32 {
        if val <= 0.0 { 0.0 } else { val.log10() }
    }
}

/// Generic plot, to be used as a background for the visualizers.
pub struct Plot<'a> {
    plot_data: &'a mut PlotData,
    grid_color: Color32,
    label_color: Color32,
    zero_line_color: Color32,
}

impl<'a> Plot<'a> {
    pub fn new(data: &'a mut PlotData) -> Self {
        Self {
            plot_data: data,
            grid_color: Color32::from_gray(50),
            label_color: Color32::from_gray(200),
            zero_line_color: Color32::from_gray(255),
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

    pub fn show(self, ui: &mut Ui) -> Rect {
        let (rect, _) = ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::empty());

        let painter = ui.painter();
        let font = FontId::new(FONT_SIZE, FontFamily::Monospace);
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

        if self.plot_data.x_axis_shown {
            let ticks = self.plot_data.x_axis.tick_positions(content_rect.width());
            for &pos in &ticks.major {
                let px_pos =
                    self.plot_data
                        .x_axis
                        .val_to_pos(pos, content_rect.min.x, content_rect.max.x);
                painter.line_segment(
                    [
                        Pos2::new(px_pos, content_rect.min.y),
                        Pos2::new(px_pos, content_rect.max.y + TICK_SIZE * 2.0),
                    ],
                    (1.0, self.grid_color),
                );
            }
            for pos in prioritized_label_positions(&ticks.major, self.plot_data.x_axis.logarithmic)
            {
                let px_pos =
                    self.plot_data
                        .x_axis
                        .val_to_pos(pos, content_rect.min.x, content_rect.max.x);
                paint_label_if_available(
                    painter,
                    Pos2::new(px_pos, content_rect.max.y + 2.0),
                    Align2::CENTER_TOP,
                    label_format(pos, PLOT_DIGITS),
                    font.clone(),
                    self.label_color,
                    &mut occupied_label_rects,
                );
            }
            for &pos in &ticks.minor {
                let px_pos =
                    self.plot_data
                        .x_axis
                        .val_to_pos(pos, content_rect.min.x, content_rect.max.x);
                painter.line_segment(
                    [
                        Pos2::new(px_pos, content_rect.max.y),
                        Pos2::new(px_pos, content_rect.max.y + TICK_SIZE),
                    ],
                    (1.0, self.grid_color),
                );
            }

            if self.plot_data.x_axis.always_show_zero
                && self.plot_data.x_axis.min < 0.0
                && self.plot_data.x_axis.max > 0.0
            {
                let px_pos =
                    self.plot_data
                        .x_axis
                        .val_to_pos(0.0, content_rect.min.x, content_rect.max.x);
                painter.line_segment(
                    [
                        Pos2::new(px_pos, content_rect.min.y),
                        Pos2::new(px_pos, content_rect.max.y),
                    ],
                    (1.0, self.zero_line_color),
                );
            }
        }

        if self.plot_data.y_axis_shown {
            let ticks = self.plot_data.y_axis.tick_positions(content_rect.height());
            for &pos in &ticks.major {
                let px_pos =
                    self.plot_data
                        .y_axis
                        .val_to_pos(pos, content_rect.max.y, content_rect.min.y);
                painter.line_segment(
                    [
                        Pos2::new(content_rect.min.x - TICK_SIZE * 2.0, px_pos),
                        Pos2::new(content_rect.max.x, px_pos),
                    ],
                    (1.0, self.grid_color),
                );
            }
            for pos in prioritized_label_positions(&ticks.major, self.plot_data.y_axis.logarithmic)
            {
                let px_pos =
                    self.plot_data
                        .y_axis
                        .val_to_pos(pos, content_rect.max.y, content_rect.min.y);
                paint_label_if_available(
                    painter,
                    Pos2::new(content_rect.min.x - 2.0, px_pos),
                    Align2::RIGHT_CENTER,
                    label_format(pos, PLOT_DIGITS),
                    font.clone(),
                    self.label_color,
                    &mut occupied_label_rects,
                );
            }
            for &pos in &ticks.minor {
                let px_pos =
                    self.plot_data
                        .y_axis
                        .val_to_pos(pos, content_rect.max.y, content_rect.min.y);
                painter.line_segment(
                    [
                        Pos2::new(content_rect.min.x, px_pos),
                        Pos2::new(content_rect.min.x - TICK_SIZE, px_pos),
                    ],
                    (1.0, self.grid_color),
                );
            }

            if self.plot_data.y_axis.always_show_zero
                && self.plot_data.y_axis.min < 0.0
                && self.plot_data.y_axis.max > 0.0
            {
                let px_pos =
                    self.plot_data
                        .y_axis
                        .val_to_pos(0.0, content_rect.max.y, content_rect.min.y);
                painter.line_segment(
                    [
                        Pos2::new(content_rect.min.x, px_pos),
                        Pos2::new(content_rect.max.x, px_pos),
                    ],
                    (1.0, self.zero_line_color),
                );
            }
        }

        content_rect
    }
}

fn prioritized_label_positions(ticks: &[f32], logarithmic: bool) -> Vec<f32> {
    let mut positions = ticks.to_vec();
    if logarithmic {
        positions.sort_by_key(|value| !is_decade(*value));
    }
    positions
}

fn is_decade(value: f32) -> bool {
    value > 0.0 && (value.log10() - value.log10().round()).abs() < 0.0001
}

fn paint_label_if_available(
    painter: &Painter,
    position: Pos2,
    anchor: Align2,
    text: String,
    font: FontId,
    color: Color32,
    occupied_rects: &mut Vec<Rect>,
) {
    let galley = painter.layout_no_wrap(text, font, color);
    let rect = anchor.anchor_size(position, galley.size());

    if label_rect_is_available(rect, occupied_rects) {
        painter.galley(rect.min, galley, color);
        occupied_rects.push(rect);
    }
}

fn label_rect_is_available(rect: Rect, occupied_rects: &[Rect]) -> bool {
    occupied_rects
        .iter()
        .all(|occupied| !occupied.expand(LABEL_SPACING).intersects(rect))
}

/// Formats small values with at most n significant digits,
/// abbreviates large values to for ex. 20k, 1M
/// and rounds vals, to make the fit and look nice on sound-related charts.
fn label_format(x: f32, n: usize) -> String {
    if x.abs() < 0.001 {
        return "0".to_string();
    }

    // Handle large numbers with suffixes
    let (val, suffix) = if x.abs() >= 1_000_000_000.0 {
        (x / 1_000_000_000.0, "B")
    } else if x.abs() >= 1_000_000.0 {
        (x / 1_000_000.0, "M")
    } else if x.abs() >= 1_000.0 {
        (x / 1_000.0, "k")
    } else {
        (x, "")
    };

    // Scale and round to n significant digits
    let log = val.abs().log10().floor();
    let scale = 10f32.powf(n as f32 - log - 1.0);
    let mut rounded = (val * scale).round() / scale;

    // If value is large (>100), round to nearest integer
    if rounded.abs() >= 100.0 {
        rounded = rounded.round();
    }

    let s = format!("{}", rounded);
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };

    format!("{}{}", s, suffix)
}

#[cfg(test)]
mod tests {
    use super::{is_decade, label_rect_is_available, prioritized_label_positions};
    use egui::{Pos2, Rect};

    #[test]
    fn logarithmic_labels_prioritize_decades() {
        let ticks = [700.0, 900.0, 1_000.0, 2_000.0, 10_000.0];

        assert_eq!(
            prioritized_label_positions(&ticks, true),
            [1_000.0, 10_000.0, 700.0, 900.0, 2_000.0]
        );
    }

    #[test]
    fn linear_labels_keep_axis_order() {
        let ticks = [700.0, 900.0, 1_000.0, 2_000.0];

        assert_eq!(prioritized_label_positions(&ticks, false), ticks.to_vec());
    }

    #[test]
    fn detects_decade_values() {
        assert!(is_decade(1.0));
        assert!(is_decade(1_000.0));
        assert!(is_decade(100_000.0));
        assert!(!is_decade(700.0));
        assert!(!is_decade(2_000.0));
    }

    #[test]
    fn accepts_label_with_enough_space() {
        let occupied = [Rect::from_min_max(
            Pos2::new(0.0, 0.0),
            Pos2::new(20.0, 10.0),
        )];
        let candidate = Rect::from_min_max(Pos2::new(23.0, 0.0), Pos2::new(43.0, 10.0));

        assert!(label_rect_is_available(candidate, &occupied));
    }

    #[test]
    fn rejects_overlapping_label() {
        let occupied = [Rect::from_min_max(
            Pos2::new(0.0, 0.0),
            Pos2::new(20.0, 10.0),
        )];
        let candidate = Rect::from_min_max(Pos2::new(15.0, 0.0), Pos2::new(35.0, 10.0));

        assert!(!label_rect_is_available(candidate, &occupied));
    }

    #[test]
    fn rejects_label_inside_spacing_margin() {
        let occupied = [Rect::from_min_max(
            Pos2::new(0.0, 0.0),
            Pos2::new(20.0, 10.0),
        )];
        let candidate = Rect::from_min_max(Pos2::new(21.0, 0.0), Pos2::new(41.0, 10.0));

        assert!(!label_rect_is_available(candidate, &occupied));
    }
}
