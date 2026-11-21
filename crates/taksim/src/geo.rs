//! Spherical-earth geodesy. Error against WGS84 is under 0.5%, far inside GPS noise at the
//! distances a simulated unit covers.

const EARTH_RADIUS_M: f64 = 6_371_008.8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    pub fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon }
    }

    fn radians(self) -> (f64, f64) {
        (self.lat.to_radians(), self.lon.to_radians())
    }

    fn from_radians(lat: f64, lon: f64) -> Self {
        Self { lat: lat.to_degrees(), lon: normalize_lon(lon.to_degrees()) }
    }
}

pub fn distance(a: LatLon, b: LatLon) -> f64 {
    let (la1, lo1) = a.radians();
    let (la2, lo2) = b.radians();
    let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// Initial great-circle bearing from `a` to `b`, degrees true in `[0, 360)`.
pub fn bearing(a: LatLon, b: LatLon) -> f64 {
    let (la1, lo1) = a.radians();
    let (la2, lo2) = b.radians();
    let y = (lo2 - lo1).sin() * la2.cos();
    let x = la1.cos() * la2.sin() - la1.sin() * la2.cos() * (lo2 - lo1).cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

pub fn destination(from: LatLon, bearing_deg: f64, meters: f64) -> LatLon {
    let (la, lo) = from.radians();
    let d = meters / EARTH_RADIUS_M;
    let b = bearing_deg.to_radians();
    let la2 = (la.sin() * d.cos() + la.cos() * d.sin() * b.cos()).asin();
    let lo2 = lo + (b.sin() * d.sin() * la.cos()).atan2(d.cos() - la.sin() * la2.sin());
    LatLon::from_radians(la2, lo2)
}

/// Point `fraction` of the way along the great circle from `a` to `b`.
pub fn interpolate(a: LatLon, b: LatLon, fraction: f64) -> LatLon {
    let d = distance(a, b);
    if d < 1e-3 {
        return a;
    }
    destination(a, bearing(a, b), d * fraction)
}

fn normalize_lon(lon: f64) -> f64 {
    (lon + 540.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_distance_and_bearing() {
        // White House to the Capitol: ~2.5 km, roughly east-southeast.
        let wh = LatLon::new(38.8977, -77.0365);
        let cap = LatLon::new(38.8899, -77.0091);
        assert!((distance(wh, cap) - 2_520.0).abs() < 30.0, "{}", distance(wh, cap));
        assert!((bearing(wh, cap) - 110.0).abs() < 2.0, "{}", bearing(wh, cap));
    }

    #[test]
    fn destination_inverts_distance() {
        let a = LatLon::new(51.5, -0.12);
        let b = destination(a, 42.0, 10_000.0);
        assert!((distance(a, b) - 10_000.0).abs() < 1e-6);
        assert!((bearing(a, b) - 42.0).abs() < 1e-6);
    }

    #[test]
    fn wraps_antimeridian() {
        let b = destination(LatLon::new(0.0, 179.99), 90.0, 5_000.0);
        assert!(b.lon < -179.9, "{}", b.lon);
    }

    #[test]
    fn interpolate_midpoint() {
        let a = LatLon::new(0.0, 0.0);
        let b = LatLon::new(0.0, 1.0);
        let m = interpolate(a, b, 0.5);
        assert!(m.lat.abs() < 1e-9 && (m.lon - 0.5).abs() < 1e-9);
    }
}
