use eframe::egui::Color32;

#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub offset: f32,
    pub color: [f32; 4],
}

impl Stop {
    pub fn new(offset: f32, color: Color32) -> Self {
        Self {
            offset,
            color: color.to_normalized_gamma_f32(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub stops: Vec<Stop>,
}

impl Gradient {
    pub fn new(stops: Vec<Stop>) -> Result<Self, String> {
        if stops.len() < 2 {
            return Err("Must have at least 2 stops".into());
        }

        let mut last = stops[0].offset;
        if !(0.0..=1.0).contains(&last) {
            return Err("Stop offset out of range".into());
        }

        for stop in stops.iter().skip(1) {
            if stop.offset <= last {
                return Err("Stop offset out of order".into());
            }
            last = stop.offset;
            if !(0.0..=1.0).contains(&last) {
                return Err("Stop offset out of range".into());
            }
        }

        Ok(Self { stops })
    }

    pub fn get(&self, pos: f32) -> [f32; 4] {
        let value = pos.clamp(0.0, 1.0);

        let mut start = &self.stops[0];
        let mut end = &self.stops[self.stops.len() - 1];

        for i in 1..self.stops.len() {
            if self.stops[i].offset >= value {
                start = &self.stops[i - 1];
                end = &self.stops[i];
                break;
            }
        }

        let ratio = (value - start.offset) / (end.offset - start.offset);
        Self::interpolate_color(&start.color, &end.color, ratio)
    }

    pub fn pre_compute_lookup(&self, size: usize) -> Vec<[f32; 4]> {
        let mut lookup = Vec::with_capacity(size);
        for i in 0..size {
            lookup.push(self.get(i as f32 / (size - 1) as f32));
        }
        lookup
    }

    fn interpolate_color(a: &[f32; 4], b: &[f32; 4], t: f32) -> [f32; 4] {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
            a[3] + (b[3] - a[3]) * t,
        ]
    }
}
