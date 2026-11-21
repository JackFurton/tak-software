//! JSON spoken with the web UI. Server to browser: one `{"kind":"event",...}` message per CoT
//! event. Browser to server: [`ClientOp`]s, which the server turns into CoT on the client's behalf.

use chrono::{DateTime, Duration, Utc};
use cot::chat::GeoChat;
use cot::{Contact, Event, Group, Point, Takv, Track, xml_escape};
use roxmltree::Document;
use serde::{Deserialize, Serialize};

use crate::hub::{DELETE_TYPE, Peer, Source, link_uids};

const MAX_TEXT: usize = 2_000;
const MAX_NAME: usize = 64;
const MARKER_LIFETIME: Duration = Duration::days(1);

#[derive(Serialize)]
struct WireEvent<'a> {
    uid: &'a str,
    #[serde(rename = "type")]
    event_type: &'a str,
    how: &'a str,
    time: i64,
    start: i64,
    stale: i64,
    lat: f64,
    lon: f64,
    hae: f64,
    ce: f64,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    callsign: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    battery: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    course: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sidc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remarks: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat: Option<WireChat>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    deletes: Vec<String>,
}

#[derive(Serialize)]
struct WireChat {
    id: String,
    sender_uid: String,
    sender_callsign: String,
    room: String,
    room_id: String,
    text: String,
    time: i64,
}

pub fn event_json(ev: &Event, source: Source) -> String {
    let d = &ev.detail;
    let kind = ev.cot_type();
    let wire = WireEvent {
        uid: &ev.uid,
        event_type: &ev.event_type,
        how: &ev.how,
        time: ev.time.timestamp_millis(),
        start: ev.start.timestamp_millis(),
        stale: ev.stale.timestamp_millis(),
        lat: ev.point.lat,
        lon: ev.point.lon,
        hae: ev.point.hae,
        ce: ev.point.ce,
        source: match source {
            Source::Mesh => "mesh",
            Source::Tcp(_) => "tcp",
            Source::Web(_) => "web",
        },
        callsign: d.contact.as_ref().map(|c| c.callsign.as_str()),
        team: d.group.as_ref().map(|g| g.name.as_str()),
        role: d.group.as_ref().map(|g| g.role.as_str()),
        battery: d.status.as_ref().map(|s| s.battery),
        speed: d.track.as_ref().map(|t| t.speed),
        course: d.track.as_ref().map(|t| t.course),
        sidc: kind.to_sidc(),
        label: kind.label(),
        remarks: if kind.is_atom() { remarks(ev) } else { None },
        chat: GeoChat::from_event(ev).map(|c| WireChat {
            id: c.message_id,
            sender_uid: c.sender_uid,
            sender_callsign: c.sender_callsign,
            room: c.room,
            room_id: c.room_id,
            text: c.text,
            time: c.time.timestamp_millis(),
        }),
        deletes: if ev.event_type == DELETE_TYPE { link_uids(ev) } else { Vec::new() },
    };
    serde_json::json!({ "kind": "event", "event": wire }).to_string()
}

fn remarks(ev: &Event) -> Option<String> {
    let wrapped = format!("<detail>{}</detail>", ev.detail.xml);
    let doc = Document::parse(&wrapped).ok()?;
    let text = doc.root_element().children().find(|n| n.has_tag_name("remarks"))?.text()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientOp {
    /// Sets who this browser is. Must come before anything else.
    Hello {
        uid: String,
        callsign: String,
        team: String,
        role: String,
    },
    /// The browser's own position, sent periodically like a phone's self SA.
    Sa {
        lat: f64,
        lon: f64,
        #[serde(default)]
        speed: Option<f64>,
        #[serde(default)]
        course: Option<f64>,
        /// True when the position came from the device's location sensor rather than a map click.
        #[serde(default)]
        gps: bool,
    },
    Chat {
        id: String,
        text: String,
        /// Recipient for a direct message; absent means All Chat Rooms.
        #[serde(default)]
        to: Option<ChatTo>,
    },
    Marker {
        uid: String,
        #[serde(rename = "type")]
        marker_type: String,
        name: String,
        lat: f64,
        lon: f64,
        #[serde(default)]
        remarks: String,
    },
    Delete {
        uid: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatTo {
    pub uid: String,
    pub callsign: String,
}

/// One browser connection's identity, turning its ops into CoT events.
#[derive(Default)]
pub struct WebSession {
    me: Option<Identity>,
    last_point: Option<Point>,
    last_sa: Option<Event>,
}

struct Identity {
    uid: String,
    callsign: String,
    team: String,
    role: String,
}

impl WebSession {
    pub fn uid(&self) -> Option<&str> {
        self.me.as_ref().map(|m| m.uid.as_str())
    }

    pub fn peer(&self) -> Option<Peer> {
        self.me.as_ref().map(|m| Peer { uid: m.uid.clone(), callsign: m.callsign.clone() })
    }

    /// The last self report, re-sent already stale so maps drop this browser's icon now.
    pub fn farewell(&self, now: DateTime<Utc>) -> Option<Event> {
        let mut ev = self.last_sa.clone()?;
        ev.time = now;
        ev.start = now;
        ev.stale = now;
        Some(ev)
    }

    pub fn handle(&mut self, op: ClientOp, now: DateTime<Utc>) -> Result<Option<Event>, String> {
        if let ClientOp::Hello { uid, callsign, team, role } = op {
            check_id(&uid)?;
            check_name(&callsign)?;
            if !Group::TEAMS.contains(&team.as_str()) {
                return Err(format!("unknown team {team:?}"));
            }
            if !Group::ROLES.contains(&role.as_str()) {
                return Err(format!("unknown role {role:?}"));
            }
            self.me = Some(Identity { uid, callsign: callsign.trim().into(), team, role });
            return Ok(None);
        }
        let me = self.me.as_ref().ok_or("send hello first")?;

        let ev = match op {
            ClientOp::Hello { .. } => unreachable!(),
            ClientOp::Sa { lat, lon, speed, course, gps } => {
                let point = Point { ce: if gps { 10.0 } else { Point::UNKNOWN }, ..Point::new(lat, lon) };
                check_point(&point)?;
                self.last_point = Some(point);
                let mut ev = stamped(Event::new(&me.uid, "a-f-G-U-C", point), now, Duration::minutes(2));
                ev.how = if gps { "m-g" } else { "h-e" }.into();
                let d = &mut ev.detail;
                d.contact =
                    Some(Contact { callsign: me.callsign.clone(), endpoint: Some("*:-1:stcp".into()) });
                d.group = Some(Group { name: me.team.clone(), role: me.role.clone() });
                if let (Some(speed), Some(course)) = (speed, course)
                    && speed.is_finite()
                    && course.is_finite()
                {
                    d.track = Some(Track { speed: speed.max(0.0), course: course.rem_euclid(360.0) });
                }
                d.takv = Some(Takv {
                    device: Some("browser".into()),
                    platform: Some("tak-software web".into()),
                    os: None,
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                });
                d.xml = format!(r#"<uid Droid="{}"/>"#, xml_escape(&me.callsign));
                self.last_sa = Some(ev.clone());
                ev
            }
            ClientOp::Chat { id, text, to } => {
                check_id(&id)?;
                let text = text.trim();
                if text.is_empty() || text.chars().count() > MAX_TEXT {
                    return Err(format!("chat text must be 1 to {MAX_TEXT} characters"));
                }
                let point = self.last_point.unwrap_or(Point::new(0.0, 0.0));
                let chat = match to {
                    Some(to) => {
                        check_id(&to.uid)?;
                        check_name(&to.callsign)?;
                        GeoChat::direct(&me.uid, &me.callsign, to.uid, to.callsign.trim(), id, text, now)
                    }
                    None => GeoChat::to_all(&me.uid, &me.callsign, id, text, now),
                };
                chat.to_event(point)
            }
            ClientOp::Marker { uid, marker_type, name, lat, lon, remarks } => {
                check_id(&uid)?;
                check_name(&name)?;
                let kind = cot::CotType::new(&marker_type);
                if kind.affiliation().is_none() || kind.dimension().is_none() {
                    return Err(format!(
                        "marker type {marker_type:?} is not a map symbol (a-<affiliation>-<dimension>...)"
                    ));
                }
                if remarks.chars().count() > MAX_TEXT {
                    return Err(format!("remarks are limited to {MAX_TEXT} characters"));
                }
                let point = Point::new(lat, lon);
                check_point(&point)?;
                let mut ev = stamped(Event::new(uid, marker_type, point), now, MARKER_LIFETIME);
                ev.how = "h-g-i-g-o".into();
                ev.detail.contact = Some(Contact { callsign: name.trim().into(), endpoint: None });
                let time = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                ev.detail.xml = format!(
                    r#"<link uid="{}" type="a-f-G-U-C" relation="p-p" parent_callsign="{}" production_time="{time}"/><archive/>"#,
                    xml_escape(&me.uid),
                    xml_escape(&me.callsign),
                );
                if !remarks.trim().is_empty() {
                    ev.detail.xml.push_str(&format!("<remarks>{}</remarks>", xml_escape(remarks.trim())));
                }
                ev
            }
            ClientOp::Delete { uid } => {
                check_id(&uid)?;
                let mut ev = stamped(
                    Event::new(
                        format!("{uid}.delete.{}", now.timestamp_millis()),
                        DELETE_TYPE,
                        Point::new(0.0, 0.0),
                    ),
                    now,
                    Duration::seconds(20),
                );
                ev.how = "h-g-i-g-o".into();
                ev.detail.xml = format!(
                    r#"<link uid="{}" relation="none" type="none"/><__forcedelete/>"#,
                    xml_escape(&uid)
                );
                ev
            }
        };
        Ok(Some(ev))
    }
}

fn stamped(mut ev: Event, now: DateTime<Utc>, lifetime: Duration) -> Event {
    ev.time = now;
    ev.start = now;
    ev.stale = now + lifetime;
    ev
}

fn check_id(id: &str) -> Result<(), String> {
    let ok =
        (1..=128).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c));
    if ok { Ok(()) } else { Err(format!("id {id:?} must be 1-128 of A-Z a-z 0-9 - _ . :")) }
}

fn check_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(format!("name must be 1 to {MAX_NAME} printable characters"));
    }
    Ok(())
}

fn check_point(p: &Point) -> Result<(), String> {
    if p.lat.is_finite() && p.lon.is_finite() && p.lat.abs() <= 90.0 && p.lon.abs() <= 180.0 {
        Ok(())
    } else {
        Err("position out of range".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(json: &str) -> ClientOp {
        serde_json::from_str(json).unwrap()
    }

    fn session() -> WebSession {
        let mut s = WebSession::default();
        s.handle(
            op(r#"{"op":"hello","uid":"WEB-1","callsign":"KESTREL","team":"Red","role":"HQ"}"#),
            Utc::now(),
        )
        .unwrap();
        s
    }

    #[test]
    fn requires_hello() {
        let mut s = WebSession::default();
        let err = s.handle(op(r#"{"op":"chat","id":"m1","text":"hi"}"#), Utc::now()).unwrap_err();
        assert!(err.contains("hello"));
    }

    #[test]
    fn builds_sa_and_chat() {
        let mut s = session();
        let sa = s.handle(op(r#"{"op":"sa","lat":38.9,"lon":-77.0}"#), Utc::now()).unwrap().unwrap();
        assert_eq!(sa.uid, "WEB-1");
        assert_eq!(sa.detail.group.as_ref().unwrap().name, "Red");
        assert_eq!(sa.how, "h-e");

        let chat = s.handle(op(r#"{"op":"chat","id":"m1","text":" hello "}"#), Utc::now()).unwrap().unwrap();
        let parsed = GeoChat::from_event(&chat).unwrap();
        assert_eq!((parsed.sender_callsign.as_str(), parsed.text.as_str()), ("KESTREL", "hello"));
        assert_eq!(chat.point.lat, 38.9);
    }

    #[test]
    fn marker_and_delete_serialize_for_ui() {
        let mut s = session();
        let m = s
            .handle(
                op(r#"{"op":"marker","uid":"mk-1","type":"a-h-G","name":"Hostile 1","lat":1,"lon":2,"remarks":"2 trucks"}"#),
                Utc::now(),
            )
            .unwrap()
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&event_json(&m, Source::Web(1))).unwrap();
        assert_eq!(json["event"]["sidc"], "SHGP-----------");
        assert_eq!(json["event"]["remarks"], "2 trucks");
        assert_eq!(json["event"]["callsign"], "Hostile 1");

        let d = s.handle(op(r#"{"op":"delete","uid":"mk-1"}"#), Utc::now()).unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(&event_json(&d, Source::Web(1))).unwrap();
        assert_eq!(json["event"]["deletes"][0], "mk-1");
    }

    #[test]
    fn rejects_bad_ops() {
        let mut s = session();
        let now = Utc::now();
        for bad in [
            r#"{"op":"sa","lat":95,"lon":0}"#,
            r#"{"op":"chat","id":"has space","text":"x"}"#,
            r#"{"op":"chat","id":"m","text":"   "}"#,
            r#"{"op":"marker","uid":"m","type":"b-t-f","name":"x","lat":0,"lon":0}"#,
            r#"{"op":"hello","uid":"u","callsign":"x","team":"Pink","role":"HQ"}"#,
        ] {
            assert!(s.handle(op(bad), now).is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<ClientOp>(r#"{"op":"sa","lat":0,"lon":0,"evil":1}"#).is_err());
    }
}
