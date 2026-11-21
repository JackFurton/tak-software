use chrono::{DateTime, Duration, Utc};

use crate::CotType;

/// A single CoT event: something at a place, valid between `start` and `stale`.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub uid: String,
    pub event_type: String,
    /// How the position was obtained, e.g. `m-g` (machine, GPS) or `h-e` (human, estimated).
    pub how: String,
    pub time: DateTime<Utc>,
    pub start: DateTime<Utc>,
    pub stale: DateTime<Utc>,
    pub access: Option<String>,
    pub qos: Option<String>,
    pub opex: Option<String>,
    pub point: Point,
    pub detail: Detail,
}

impl Event {
    pub const DEFAULT_STALE: Duration = Duration::minutes(2);

    /// A machine-generated GPS event stamped now and going stale after [`Self::DEFAULT_STALE`].
    pub fn new(uid: impl Into<String>, event_type: impl Into<String>, point: Point) -> Self {
        let now = Utc::now();
        Self {
            uid: uid.into(),
            event_type: event_type.into(),
            how: "m-g".into(),
            time: now,
            start: now,
            stale: now + Self::DEFAULT_STALE,
            access: None,
            qos: None,
            opex: None,
            point,
            detail: Detail::default(),
        }
    }

    pub fn cot_type(&self) -> CotType<'_> {
        CotType::new(&self.event_type)
    }

    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        now >= self.stale
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
    /// Height above the WGS84 ellipsoid, meters.
    pub hae: f64,
    /// Circular error, meters.
    pub ce: f64,
    /// Linear (vertical) error, meters.
    pub le: f64,
}

impl Point {
    /// TAK's sentinel for an unknown height or error.
    pub const UNKNOWN: f64 = 9_999_999.0;

    pub fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon, hae: Self::UNKNOWN, ce: Self::UNKNOWN, le: Self::UNKNOWN }
    }

    pub(crate) fn validate(&self) -> crate::Result<()> {
        let check = |name, v: f64, limit: f64| {
            if v.is_finite() && v.abs() <= limit {
                Ok(())
            } else {
                Err(crate::Error::InvalidValue { name, value: v.to_string() })
            }
        };
        check("lat", self.lat, 90.0)?;
        check("lon", self.lon, 180.0)?;
        check("hae", self.hae, f64::MAX)?;
        check("ce", self.ce, f64::MAX)?;
        check("le", self.le, f64::MAX)
    }
}

/// The `<detail>` element. Elements TAK protocol v1 has a typed field for are parsed into the
/// options below; everything else is kept verbatim in `xml` so relays forward it untouched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Detail {
    pub contact: Option<Contact>,
    pub group: Option<Group>,
    pub precision_location: Option<PrecisionLocation>,
    pub status: Option<Status>,
    pub takv: Option<Takv>,
    pub track: Option<Track>,
    /// Remaining child elements as raw XML, in document order.
    pub xml: String,
}

impl Detail {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Contact {
    pub callsign: String,
    /// `host:port:proto` where the sender accepts direct messages, e.g. `*:-1:stcp`.
    pub endpoint: Option<String>,
}

/// Team color and role, serialized as `<__group>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub role: String,
}

impl Group {
    /// ATAK's fixed team colors; clients ignore any other name.
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrecisionLocation {
    pub geopointsrc: Option<String>,
    pub altsrc: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub battery: u32,
}

/// Software that produced the event.
#[derive(Debug, Clone, PartialEq)]
pub struct Takv {
    pub device: Option<String>,
    pub platform: Option<String>,
    pub os: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Meters per second.
    pub speed: f64,
    /// Degrees true.
    pub course: f64,
}
