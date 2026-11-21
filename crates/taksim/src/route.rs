use serde::Deserialize;

use crate::geo::{self, LatLon};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Return to the first waypoint and go round again.
    #[default]
    Loop,
    /// Walk to the last waypoint, then retrace back to the first.
    Pingpong,
}

#[derive(Debug, Clone)]
pub struct Route {
    points: Vec<LatLon>,
    // cumulative[i] is the distance from points[0] to points[i] along the route.
    cumulative: Vec<f64>,
    mode: Mode,
}

impl Route {
    /// Panics on an empty route; scenarios are validated before routes are built.
    pub fn new(mut points: Vec<LatLon>, mode: Mode) -> Self {
        assert!(!points.is_empty(), "route needs at least one point");
        if mode == Mode::Loop && points.len() > 1 && points.first() != points.last() {
            points.push(points[0]);
        }
        let mut cumulative = vec![0.0];
        for pair in points.windows(2) {
            cumulative.push(cumulative.last().unwrap() + geo::distance(pair[0], pair[1]));
        }
        Self { points, cumulative, mode }
    }

    pub fn length(&self) -> f64 {
        *self.cumulative.last().unwrap()
    }

    /// Position and course (degrees true) after travelling `travelled` meters from the start.
    pub fn at(&self, travelled: f64) -> (LatLon, f64) {
        let length = self.length();
        if length < 1e-3 {
            return (self.points[0], 0.0);
        }
        let (along, reversed) = match self.mode {
            Mode::Loop => (travelled.rem_euclid(length), false),
            Mode::Pingpong => {
                let d = travelled.rem_euclid(2.0 * length);
                if d > length { (2.0 * length - d, true) } else { (d, false) }
            }
        };

        let seg = self.cumulative.partition_point(|&c| c <= along).clamp(1, self.points.len() - 1) - 1;
        let (a, b) = (self.points[seg], self.points[seg + 1]);
        let seg_len = self.cumulative[seg + 1] - self.cumulative[seg];
        let fraction = if seg_len > 0.0 { (along - self.cumulative[seg]) / seg_len } else { 0.0 };
        let pos = geo::interpolate(a, b, fraction);
        let course = if reversed { geo::bearing(b, a) } else { geo::bearing(a, b) };
        (pos, course)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<LatLon> {
        let o = LatLon::new(10.0, 10.0);
        let e = geo::destination(o, 90.0, 1_000.0);
        let ne = geo::destination(e, 0.0, 1_000.0);
        let n = geo::destination(o, 0.0, 1_000.0);
        vec![o, e, ne, n]
    }

    #[test]
    fn loop_closes_and_wraps() {
        let r = Route::new(square(), Mode::Loop);
        assert!((r.length() - 4_000.0).abs() < 1.0);
        let (p, course) = r.at(500.0);
        assert!((geo::distance(square()[0], p) - 500.0).abs() < 1.0);
        assert!((course - 90.0).abs() < 0.5);
        let (p, _) = r.at(r.length() + 500.0);
        assert!((geo::distance(square()[0], p) - 500.0).abs() < 1.0);
        let (_, course) = r.at(3_500.0);
        assert!((course - 180.0).abs() < 0.5, "{course}");
    }

    #[test]
    fn pingpong_reverses() {
        let pts = square();
        let r = Route::new(pts.clone(), Mode::Pingpong);
        assert!((r.length() - 3_000.0).abs() < 1.0);
        // 500 m back from the far end, heading east again along the last leg.
        let (p, course) = r.at(3_500.0);
        assert!((geo::distance(pts[3], p) - 500.0).abs() < 1.0);
        assert!((course - 90.0).abs() < 0.5, "{course}");
    }

    #[test]
    fn single_point_is_stationary() {
        let r = Route::new(vec![LatLon::new(1.0, 2.0)], Mode::Loop);
        assert_eq!(r.at(1_000.0), (LatLon::new(1.0, 2.0), 0.0));
    }
}
