use egui::{Color32, Margin, Pos2, Rect, Sense, Ui};
use std::ops::Sub;

#[derive(Clone)]
pub struct PolarPlotData {
    // This is to be used to display graphs where the data is bounded, so we dont need an infinite plot.
    pub viewport: Rect,
    pub center_offset: Pos2,
    pub radius: f32,
    pub radial_lines: PolarRadials,
    pub concentric_circles: PolarConcentricCircles,
}

#[derive(Clone)]
pub struct PolarRadials {
    /// The angles (radians) of the radial lines to be drawn on the polar plot.
    pub line_angles: Vec<f32>,
}

impl PolarConcentricCircles {
    pub fn new(preffered_spacing: f32) -> Self {
        Self { preffered_spacing }
    }
}

#[derive(Clone)]
pub struct PolarConcentricCircles {
    /// preffered distance to keep between the circles.
    /// their count will be determined by the radius of the plot and this value.
    pub preffered_spacing: f32,
}

impl Default for PolarPlotData {
    fn default() -> Self {
        Self::new()
    }
}

impl PolarPlotData {
    pub fn new() -> Self {
        Self {
            center_offset: Pos2::new(0.0, 0.0),
            radius: 1.0,
            radial_lines: PolarRadials {
                line_angles: vec![],
            },
            concentric_circles: PolarConcentricCircles::new(0.1),
            viewport: Rect::from_min_max(Pos2::new(-1.0, -1.0), Pos2::new(1.0, 1.0)),
        }
    }

    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn with_center_offset(mut self, offset: Pos2) -> Self {
        self.center_offset = offset;
        self
    }

    pub fn with_radial_lines(mut self, angles: Vec<f32>) -> Self {
        self.radial_lines.line_angles = angles;
        self
    }

    pub fn with_concentric_circles(mut self, spacing: f32) -> Self {
        self.concentric_circles.preffered_spacing = spacing;
        self
    }

    pub fn with_viewport(mut self, viewport: Rect) -> Self {
        self.viewport = viewport;
        self
    }

    pub fn gl_radius(&self, r: f32) -> f32 {
        r / self.viewport.width() * 2.0
    }

    pub fn gl_center_pos(&self) -> Pos2 {
        let gl_x = (self.center_offset.x - self.viewport.left()) / self.viewport.width() * 2.0 - 1.0;
        let gl_y = (self.center_offset.y - self.viewport.top()) / self.viewport.height() * 2.0 - 1.0;
        Pos2::new(gl_x, gl_y)
    }

    // Compute gl-space coordinate of a point in this plot's coordinate system.
    pub fn gl_pos(&self, theta: f32, r: f32) -> Pos2 {
        let x = r * theta.cos() + self.center_offset.x;
        let y = r * theta.sin() + self.center_offset.y;
        // now map to gl -1..1 space
        let gl_x = (x - self.viewport.left()) / self.viewport.width() * 2.0 - 1.0;
        let gl_y = (y - self.viewport.top()) / self.viewport.height() * 2.0 - 1.0;
        Pos2::new(gl_x, gl_y)
    }


    /// Compute the center position of the plot in the egui Ui coordinate space.
    pub fn center_pos_in_ui(&self, ui_rect: Rect) -> Pos2 {
        Pos2::new(
            (self.center_offset.x - self.viewport.left()) / self.viewport.width() * ui_rect.width() + ui_rect.min.x,
            (self.center_offset.y + self.viewport.bottom()) / self.viewport.height() * ui_rect.height() + ui_rect.min.y,
        )
    }

    pub fn ellipse_size_in_ui(&self, ui_rect: Rect, radius: f32) -> (f32, f32) {
        let width = radius / self.viewport.width() * ui_rect.width();
        let height = radius / self.viewport.height() * ui_rect.height();
        (width, height)
    }
}

pub struct PolarPlot<'a> {
    plot_data: &'a PolarPlotData,
    grid_color: Color32,
    label_color: Color32,
    zero_line_color: Color32,
}

impl<'a> PolarPlot<'a> {
    pub fn new(data: &'a PolarPlotData) -> Self {
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

        let plot_paint_rect = rect.sub(Margin::same(1)); // we need to leave some space for the stroke of the ellipse.

        let painter = ui.painter().with_clip_rect(rect);
        // we need to clip the painter so that the ellipse does not go outside the plot area.

        let center = self.plot_data.center_pos_in_ui(plot_paint_rect);
        let outer_rim_size = self
            .plot_data
            .ellipse_size_in_ui(plot_paint_rect, self.plot_data.radius);

        self.stroke_ellipse(&painter, center, outer_rim_size, self.zero_line_color);

        // draw concentric circles
        let num_circles = (self.plot_data.radius / self.plot_data.concentric_circles.preffered_spacing).floor() as usize;
        let cicrle_radii: Vec<f32> = (1..=num_circles)
            .map(|i| i as f32 * self.plot_data.concentric_circles.preffered_spacing)
            .collect();
        for r in cicrle_radii {
            let size = self.plot_data.ellipse_size_in_ui(plot_paint_rect, r);
            self.stroke_ellipse(&painter, center, size, self.grid_color);
        }


        // draw the radial lines
        for angle in &self.plot_data.radial_lines.line_angles {
            // flip the angle across the x-axis to have angles behave as the user would expect.
            let angle = -angle;
            let end_x = center.x + outer_rim_size.0 * angle.cos();
            let end_y = center.y + outer_rim_size.1 * angle.sin();
            painter.line_segment(
                [center, Pos2::new(end_x, end_y)],
                (1.0, self.zero_line_color),
            );
        }

        rect
    }

    fn stroke_ellipse(
        &self,
        painter: &egui::Painter,
        center: Pos2,
        size: (f32, f32),
        color: Color32,
    ) {
        // we need to draw the ellipse as a polygon because egui does not have a built-in ellipse drawing function.
        let num_segments = 128;
        let mut points = Vec::with_capacity(num_segments);
        for i in 0..num_segments {
            let theta = 2.0 * std::f32::consts::PI * (i as f32) / (num_segments as f32);
            let x = size.0 * theta.cos();
            let y = size.1 * theta.sin();
            points.push(Pos2::new(center.x + x, center.y + y));
        }
        painter.add(egui::Shape::closed_line(points, (1.0, color)));
    }
}
