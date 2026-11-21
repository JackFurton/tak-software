use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use cot::{Contact, Event, Group, Point, Status, Takv, Track};
use serde::Deserialize;

use crate::geo::{self, LatLon};
use crate::route::{Mode, Route};

/// ATAK's fixed team colors; any other `__group name` is ignored by clients.
pub const TEAMS: &[&str] = &[
    "White",
    "Yellow",
    "Orange",
    "Magenta",
    "Red",
    "Maroon",
    "Purple",
    "Dark Blue",
    "Blue",
    "Cyan",
    "Teal",
    "Green",
    "Dark Green",
    "Brown",
];

pub const ROLES: &[&str] =
    &["Team Member", "Team Lead", "HQ", "Sniper", "Medic", "Forward Observer", "RTO", "K9"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    #[serde(default = "default_interval")]
    pub interval_secs: f64,
    #[serde(rename = "unit", default)]
    pub units: Vec<UnitSpec>,
}

fn default_interval() -> f64 {
    5.0
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitSpec {
    pub callsign: String,
    pub uid: Option<String>,
    #[serde(rename = "type", default = "default_type")]
    pub cot_type: String,
    #[serde(default = "default_team")]
    pub team: String,
    #[serde(default = "default_role")]
    pub role: String,
    /// Meters per second.
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub mode: Mode,
    /// `[lat, lon]` waypoints.
    pub route: Vec<[f64; 2]>,
}

fn default_type() -> String {
    "a-f-G-U-C".into()
}

fn default_team() -> String {
    "Cyan".into()
}

fn default_role() -> String {
    "Team Member".into()
}

impl Scenario {
    pub fn parse(text: &str) -> Result<Self> {
        let scenario: Scenario = toml::from_str(text)?;
        ensure!(
            scenario.interval_secs.is_finite() && scenario.interval_secs >= 0.1,
            "interval_secs must be at least 0.1"
        );
        ensure!(!scenario.units.is_empty(), "scenario has no [[unit]] entries");
        Ok(scenario)
    }

    pub fn interval(&self) -> Duration {
        Duration::from_secs_f64(self.interval_secs)
    }

    pub fn build_units(&self) -> Result<Vec<Unit>> {
        let units = self
            .units
            .iter()
            .map(|spec| Unit::from_spec(spec).with_context(|| format!("unit {:?}", spec.callsign)))
            .collect::<Result<Vec<_>>>()?;
        let mut uids: Vec<_> = units.iter().map(|u| u.uid.as_str()).collect();
        uids.sort_unstable();
        if let Some(dup) = uids.windows(2).find(|w| w[0] == w[1]) {
            bail!("duplicate uid {:?}; give one of the units an explicit uid", dup[0]);
        }
        Ok(units)
    }
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub uid: String,
    pub callsign: String,
    pub cot_type: String,
    pub team: String,
    pub role: String,
    pub speed: f64,
    pub route: Route,
    /// Meters along the route at t=0, so generated units don't start bunched together.
    pub phase: f64,
}

impl Unit {
    fn from_spec(spec: &UnitSpec) -> Result<Self> {
        ensure!(!spec.callsign.trim().is_empty(), "callsign is empty");
        ensure!(spec.cot_type.starts_with("a-"), "type {:?} is not an atom (a-...)", spec.cot_type);
        ensure!(TEAMS.contains(&spec.team.as_str()), "team {:?} is not one of {TEAMS:?}", spec.team);
        ensure!(ROLES.contains(&spec.role.as_str()), "role {:?} is not one of {ROLES:?}", spec.role);
        ensure!(spec.speed.is_finite() && spec.speed >= 0.0, "speed must be >= 0");
        ensure!(!spec.route.is_empty(), "route is empty");
        for &[lat, lon] in &spec.route {
            ensure!(lat.abs() <= 90.0 && lon.abs() <= 180.0, "waypoint [{lat}, {lon}] is out of range");
        }

        let points = spec.route.iter().map(|&[lat, lon]| LatLon::new(lat, lon)).collect();
        Ok(Self {
            uid: spec.uid.clone().unwrap_or_else(|| default_uid(&spec.callsign)),
            callsign: spec.callsign.clone(),
            cot_type: spec.cot_type.clone(),
            team: spec.team.clone(),
            role: spec.role.clone(),
            speed: spec.speed,
            route: Route::new(points, spec.mode),
            phase: 0.0,
        })
    }

    /// The unit's position report `elapsed` into the run.
    pub fn report(&self, elapsed: Duration, now: DateTime<Utc>, stale_after: Duration) -> Event {
        let (pos, course) = self.route.at(self.phase + self.speed * elapsed.as_secs_f64());
        let mut ev = Event::new(&self.uid, &self.cot_type, Point { ce: 5.0, ..Point::new(pos.lat, pos.lon) });
        ev.time = now;
        ev.start = now;
        ev.stale = now + chrono::Duration::from_std(stale_after).unwrap_or(Event::DEFAULT_STALE);

        let d = &mut ev.detail;
        d.contact = Some(Contact { callsign: self.callsign.clone(), endpoint: Some("*:-1:stcp".into()) });
        d.group = Some(Group { name: self.team.clone(), role: self.role.clone() });
        d.track = Some(Track { speed: self.speed, course });
        // About 1% every 6 minutes, like a phone with the screen off.
        let drained = (elapsed.as_secs() / 360).min(95) as u32;
        d.status = Some(Status { battery: 100 - drained });
        d.takv = Some(Takv {
            device: Some("taksim".into()),
            platform: Some("tak-software".into()),
            os: Some(std::env::consts::OS.into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
        });
        d.xml = format!(r#"<uid Droid="{}"/>"#, xml_escape(&self.callsign));
        ev
    }
}

/// Generates `count` units circling `center` at radii up to `radius` meters, for load testing.
pub fn swarm(count: usize, center: LatLon, radius: f64) -> Vec<Unit> {
    const GOLDEN: f64 = 0.618_033_988_75;
    const TYPES: &[&str] = &["a-f-G-U-C", "a-f-G-U-C-I", "a-f-G-E-V", "a-f-G-U-C-R", "a-f-G-U-U-M"];
    (0..count)
        .map(|i| {
            // Golden-ratio spacing spreads radii and start angles evenly without randomness.
            let spread = (i as f64 * GOLDEN).fract();
            let r = radius * (0.2 + 0.8 * spread);
            let ring = (0..24).map(|k| geo::destination(center, k as f64 * 15.0, r)).collect();
            let route = Route::new(ring, Mode::Loop);
            let callsign = format!("SIM-{:04}", i + 1);
            Unit {
                uid: default_uid(&callsign),
                phase: route.length() * ((i as f64 * GOLDEN * 7.0).fract()),
                speed: 1.4 + 13.6 * ((i as f64 * GOLDEN * 3.0).fract()),
                cot_type: TYPES[i % TYPES.len()].into(),
                team: TEAMS[i % TEAMS.len()].into(),
                role: ROLES[i % ROLES.len()].into(),
                callsign,
                route,
            }
        })
        .collect()
}

fn default_uid(callsign: &str) -> String {
    format!("taksim-{}", callsign.to_lowercase().replace(char::is_whitespace, "-"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENARIO: &str = r#"
        interval_secs = 2
        [[unit]]
        callsign = "ALPHA 1"
        team = "Red"
        speed = 10
        mode = "pingpong"
        route = [[38.0, -77.0], [38.01, -77.0]]
        [[unit]]
        callsign = "HQ"
        type = "a-f-G-I"
        role = "HQ"
        route = [[38.0, -77.0]]
    "#;

    #[test]
    fn parses_and_reports() {
        let scenario = Scenario::parse(SCENARIO).unwrap();
        assert_eq!(scenario.interval(), Duration::from_secs(2));
        let units = scenario.build_units().unwrap();
        assert_eq!(units[0].uid, "taksim-alpha-1");

        let now = Utc::now();
        let ev = units[0].report(Duration::from_secs(10), now, Duration::from_secs(30));
        assert!((ev.point.lat - 38.0).abs() > 0.0008, "moved ~100 m north");
        assert_eq!(ev.detail.contact.as_ref().unwrap().callsign, "ALPHA 1");
        assert!(ev.detail.track.as_ref().unwrap().course < 1.0);
        assert_eq!((ev.stale - now).num_seconds(), 30);
        Event::from_xml(&ev.to_xml()).unwrap();

        let hq = units[1].report(Duration::from_secs(600), now, Duration::from_secs(30));
        assert_eq!((hq.point.lat, hq.point.lon), (38.0, -77.0));
    }

    #[test]
    fn rejects_bad_specs() {
        let bad = |field: &str| {
            let text = format!("[[unit]]\ncallsign = \"X\"\nroute = [[0, 0]]\n{field}\n");
            Scenario::parse(&text).and_then(|s| s.build_units()).unwrap_err().to_string()
        };
        assert!(bad("team = \"Pink\"").contains("unit \"X\""));
        assert!(Scenario::parse("[[unit]]\ncallsign = \"X\"\nroute = [[0, 0]]\nspeeed = 1").is_err());
        let dup =
            "[[unit]]\ncallsign = \"A\"\nroute = [[0, 0]]\n[[unit]]\ncallsign = \"a\"\nroute = [[0, 0]]";
        assert!(Scenario::parse(dup).unwrap().build_units().unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn swarm_is_deterministic_and_unique() {
        let a = swarm(200, LatLon::new(0.0, 0.0), 5_000.0);
        let b = swarm(200, LatLon::new(0.0, 0.0), 5_000.0);
        assert_eq!(
            a.iter().map(|u| u.phase).collect::<Vec<_>>(),
            b.iter().map(|u| u.phase).collect::<Vec<_>>()
        );
        let mut uids: Vec<_> = a.iter().map(|u| &u.uid).collect();
        uids.dedup();
        assert_eq!(uids.len(), 200);
    }
}
