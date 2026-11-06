use egui::Color32;

#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub offset: f32,
    pub color: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_interpolates_multiple_stops_and_preserves_endpoints() {
        let gradient = Gradient::new(vec![
            Stop::new(0.0, Color32::RED),
            Stop::new(0.5, Color32::GREEN),
            Stop::new(1.0, Color32::BLUE),
        ])
        .unwrap();
        let table = gradient.pre_compute_lookup(5);
        assert_eq!(table[0], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(table[1], [0.5, 0.5, 0.0, 1.0]);
        assert_eq!(table[2], [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(table[4], [0.0, 0.0, 1.0, 1.0]);
        assert!(gradient.pre_compute_lookup(0).is_empty());
        assert_eq!(gradient.pre_compute_lookup(1), vec![table[0]]);
        let partial = Gradient::new(vec![
            Stop::new(0.25, Color32::RED),
            Stop::new(0.75, Color32::BLUE),
        ])
        .unwrap();
        assert_eq!(partial.get(0.0), partial.stops[0].color);
        assert_eq!(partial.get(1.0), partial.stops[1].color);
    }
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

        if value <= self.stops[0].offset {
            return self.stops[0].color;
        }
        if value >= self.stops[self.stops.len() - 1].offset {
            return self.stops[self.stops.len() - 1].color;
        }

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
        if size == 0 {
            return Vec::new();
        }
        if size == 1 {
            return vec![self.get(0.0)];
        }
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
