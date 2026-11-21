//! TAK protocol v1 messages. Field tags match the `.proto` files shipped with ATAK-CIV in
//! `commoncommo/core/impl/protobuf/`; times are milliseconds since the Unix epoch.

use chrono::{DateTime, Utc};

use crate::{Error, Event, Point, Result};

#[derive(Clone, PartialEq, prost::Message)]
pub struct TakMessage {
    #[prost(message, optional, tag = "1")]
    pub tak_control: Option<TakControl>,
    #[prost(message, optional, tag = "2")]
    pub cot_event: Option<CotEvent>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TakControl {
    #[prost(uint32, tag = "1")]
    pub min_proto_version: u32,
    #[prost(uint32, tag = "2")]
    pub max_proto_version: u32,
    #[prost(string, tag = "3")]
    pub contact_uid: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CotEvent {
    #[prost(string, tag = "1")]
    pub r#type: String,
    #[prost(string, tag = "2")]
    pub access: String,
    #[prost(string, tag = "3")]
    pub qos: String,
    #[prost(string, tag = "4")]
    pub opex: String,
    #[prost(string, tag = "5")]
    pub uid: String,
    #[prost(uint64, tag = "6")]
    pub send_time: u64,
    #[prost(uint64, tag = "7")]
    pub start_time: u64,
    #[prost(uint64, tag = "8")]
    pub stale_time: u64,
    #[prost(string, tag = "9")]
    pub how: String,
    #[prost(double, tag = "10")]
    pub lat: f64,
    #[prost(double, tag = "11")]
    pub lon: f64,
    #[prost(double, tag = "12")]
    pub hae: f64,
    #[prost(double, tag = "13")]
    pub ce: f64,
    #[prost(double, tag = "14")]
    pub le: f64,
    #[prost(message, optional, tag = "15")]
    pub detail: Option<Detail>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Detail {
    #[prost(string, tag = "1")]
    pub xml_detail: String,
    #[prost(message, optional, tag = "2")]
    pub contact: Option<Contact>,
    #[prost(message, optional, tag = "3")]
    pub group: Option<Group>,
    #[prost(message, optional, tag = "4")]
    pub precision_location: Option<PrecisionLocation>,
    #[prost(message, optional, tag = "5")]
    pub status: Option<Status>,
    #[prost(message, optional, tag = "6")]
    pub takv: Option<Takv>,
    #[prost(message, optional, tag = "7")]
    pub track: Option<Track>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Contact {
    #[prost(string, tag = "1")]
    pub endpoint: String,
    #[prost(string, tag = "2")]
    pub callsign: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Group {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub role: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PrecisionLocation {
    #[prost(string, tag = "1")]
    pub geopointsrc: String,
    #[prost(string, tag = "2")]
    pub altsrc: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Status {
    #[prost(uint32, tag = "1")]
    pub battery: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Takv {
    #[prost(string, tag = "1")]
    pub device: String,
    #[prost(string, tag = "2")]
    pub platform: String,
    #[prost(string, tag = "3")]
    pub os: String,
    #[prost(string, tag = "4")]
    pub version: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Track {
    #[prost(double, tag = "1")]
    pub speed: f64,
    #[prost(double, tag = "2")]
    pub course: f64,
}

impl Event {
    pub fn to_proto(&self) -> CotEvent {
        let d = &self.detail;
        let detail = (!d.is_empty()).then(|| Detail {
            xml_detail: d.xml.clone(),
            contact: d.contact.as_ref().map(|c| Contact {
                endpoint: c.endpoint.clone().unwrap_or_default(),
                callsign: c.callsign.clone(),
            }),
            group: d.group.as_ref().map(|g| Group { name: g.name.clone(), role: g.role.clone() }),
            precision_location: d.precision_location.as_ref().map(|p| PrecisionLocation {
                geopointsrc: p.geopointsrc.clone().unwrap_or_default(),
                altsrc: p.altsrc.clone().unwrap_or_default(),
            }),
            status: d.status.as_ref().map(|s| Status { battery: s.battery }),
            takv: d.takv.as_ref().map(|t| Takv {
                device: t.device.clone().unwrap_or_default(),
                platform: t.platform.clone().unwrap_or_default(),
                os: t.os.clone().unwrap_or_default(),
                version: t.version.clone().unwrap_or_default(),
            }),
            track: d.track.as_ref().map(|t| Track { speed: t.speed, course: t.course }),
        });

        CotEvent {
            r#type: self.event_type.clone(),
            access: self.access.clone().unwrap_or_default(),
            qos: self.qos.clone().unwrap_or_default(),
            opex: self.opex.clone().unwrap_or_default(),
            uid: self.uid.clone(),
            send_time: to_millis(self.time),
            start_time: to_millis(self.start),
            stale_time: to_millis(self.stale),
            how: self.how.clone(),
            lat: self.point.lat,
            lon: self.point.lon,
            hae: self.point.hae,
            ce: self.point.ce,
            le: self.point.le,
            detail,
        }
    }

    pub fn to_tak_message(&self) -> TakMessage {
        TakMessage { tak_control: None, cot_event: Some(self.to_proto()) }
    }

    pub fn from_proto(ev: CotEvent) -> Result<Self> {
        if ev.uid.is_empty() {
            return Err(Error::MissingAttribute("uid"));
        }
        if ev.r#type.is_empty() {
            return Err(Error::MissingAttribute("type"));
        }
        let point = Point { lat: ev.lat, lon: ev.lon, hae: ev.hae, ce: ev.ce, le: ev.le };
        point.validate()?;

        let detail = match ev.detail {
            None => crate::Detail::default(),
            Some(d) => {
                crate::Detail::validate_raw(&d.xml_detail)?;
                crate::Detail {
                    contact: d
                        .contact
                        .map(|c| crate::Contact { callsign: c.callsign, endpoint: non_empty(c.endpoint) }),
                    group: d.group.map(|g| crate::Group { name: g.name, role: g.role }),
                    precision_location: d.precision_location.map(|p| crate::PrecisionLocation {
                        geopointsrc: non_empty(p.geopointsrc),
                        altsrc: non_empty(p.altsrc),
                    }),
                    status: d.status.map(|s| crate::Status { battery: s.battery }),
                    takv: d.takv.map(|t| crate::Takv {
                        device: non_empty(t.device),
                        platform: non_empty(t.platform),
                        os: non_empty(t.os),
                        version: non_empty(t.version),
                    }),
                    track: d
                        .track
                        .filter(|t| t.speed.is_finite() && t.course.is_finite())
                        .map(|t| crate::Track { speed: t.speed, course: t.course }),
                    xml: d.xml_detail,
                }
            }
        };

        Ok(Self {
            uid: ev.uid,
            event_type: ev.r#type,
            how: ev.how,
            time: from_millis("sendTime", ev.send_time)?,
            start: from_millis("startTime", ev.start_time)?,
            stale: from_millis("staleTime", ev.stale_time)?,
            access: non_empty(ev.access),
            qos: non_empty(ev.qos),
            opex: non_empty(ev.opex),
            point,
            detail,
        })
    }
}

fn to_millis(t: DateTime<Utc>) -> u64 {
    t.timestamp_millis().max(0) as u64
}

fn from_millis(name: &'static str, ms: u64) -> Result<DateTime<Utc>> {
    i64::try_from(ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .ok_or(Error::InvalidValue { name, value: ms.to_string() })
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}
