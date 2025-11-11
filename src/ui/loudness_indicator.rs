use crate::sound::smoothing::FloatArraySmoother;
use crate::sound::smoothing::multiplicative_smoother::MultiplicativeSmoother;
use crate::ui::plot::Axis;
use crate::wavesync::WaveSyncVisuals;
use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, Widget};

pub const LOUDNESS_FONT_SIZE: f32 = 8.0;

pub struct LoudnessIndicator {
    smoother: MultiplicativeSmoother,
    axis: Axis,
    tick_cache: Vec<f32>,
}

impl Default for LoudnessIndicator {
    fn default() -> Self {
        Self::new()
    }
}

impl LoudnessIndicator {
    pub fn new() -> Self {
        let axis = Axis::linear(-100.0, 0.0);
        let mut this = Self {
            smoother: MultiplicativeSmoother::new(),
            tick_cache: axis.tick_positions(200.0).major,
            axis,
        };
        this.smoother.min = -100.0;
        this.smoother.max = 0.0;
        this
    }

    pub fn ui<'a>(
        &'a mut self,
        vals: &[f32],
        dt: f32,
        visuals: &'a WaveSyncVisuals,
        height: f32,
    ) -> LoudnessWidget<'a> {
        let vals = self.smoother.smooth_data(dt, vals).to_vec();
        LoudnessWidget {
            vals,
            visuals,
            axis: &self.axis,
            tics: &self.tick_cache,
            height,
        }
    }
}

pub struct LoudnessWidget<'a> {
    vals: Vec<f32>,
    visuals: &'a WaveSyncVisuals,
    axis: &'a Axis,
    tics: &'a [f32],
    height: f32,
}

impl Widget for LoudnessWidget<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (response, painter) =
            ui.allocate_painter(Vec2::new(150.0, self.height), Sense::empty());
        let height = response.rect.height() / self.vals.len() as f32;

        for (i, val) in self.vals.into_iter().enumerate() {
            let p = self.axis.val_to_pos(val, 0.0, response.rect.width());
            let lerp_factor = self.axis.norm_pos(val);
            let color = self
                .visuals
                .color_start()
                .lerp_to_gamma(self.visuals.color_end(), lerp_factor);
            let rect = Rect::from_min_size(
                Pos2::new(response.rect.min.x, response.rect.min.y + height * i as f32),
                Vec2::new(p, height),
            );
            painter.rect_filled(rect, CornerRadius::same(3), color);
        }

        let tick_size = 4.0;

        let color = self.visuals.plot_grid_highlight();
        let stroke = Stroke::new(1.0, color);
        let font = FontId::monospace(LOUDNESS_FONT_SIZE);

        for (i, tick) in self.tics.iter().enumerate() {
            let p = self
                .axis
                .val_to_pos(*tick, 0.5, response.rect.width() - 0.5);
            painter.vline(
                response.rect.min.x + p,
                response.rect.min.y..=response.rect.min.y + tick_size,
                stroke,
            );
            painter.vline(
                response.rect.min.x + p,
                response.rect.max.y - tick_size..=response.rect.max.y,
                stroke,
            );

            let align = if i == 0 {
                Align2::LEFT_CENTER
            } else if i == self.tics.len() - 1 {
                Align2::RIGHT_CENTER
            } else {
                Align2::CENTER_CENTER
            };

            painter.text(
                Pos2::new(response.rect.min.x + p, response.rect.center().y),
                align,
                tick.round().to_string(),
                font.clone(),
                color,
            );
        }

        ui.response()
    }
}
