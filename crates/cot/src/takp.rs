//! Streaming protocol negotiation: how a TCP/TLS connection that starts as CoT XML switches to
//! TAK protocol (protobuf). The server offers versions with [`SUPPORT_TYPE`], the client asks
//! for one with [`REQUEST_TYPE`], and the server's [`RESPONSE_TYPE`] is the last XML either side
//! sends if it says yes. Specified in `commoncommo/core/impl/protobuf/protocol.txt` in ATAK-CIV.

use chrono::{DateTime, Duration, Utc};
use roxmltree::Document;

use crate::{Event, Point};

pub const SUPPORT_TYPE: &str = "t-x-takp-v";
pub const REQUEST_TYPE: &str = "t-x-takp-q";
pub const RESPONSE_TYPE: &str = "t-x-takp-r";

/// The only TAK protocol version that exists.
pub const VERSION: u32 = 1;

pub fn is_negotiation(ev: &Event) -> bool {
    ev.event_type.starts_with("t-x-takp")
}

/// Server offer. `uid` names the negotiation and is echoed back in the request and response.
pub fn support(uid: &str, versions: &[u32], now: DateTime<Utc>) -> Event {
    let body: String = versions.iter().map(|v| format!(r#"<TakProtocolSupport version="{v}"/>"#)).collect();
    control(uid, SUPPORT_TYPE, &body, now)
}

pub fn request(uid: &str, version: u32, now: DateTime<Utc>) -> Event {
    control(uid, REQUEST_TYPE, &format!(r#"<TakRequest version="{version}"/>"#), now)
}

pub fn response(uid: &str, accepted: bool, now: DateTime<Utc>) -> Event {
    control(uid, RESPONSE_TYPE, &format!(r#"<TakResponse status="{accepted}"/>"#), now)
}

/// Versions offered by a [`SUPPORT_TYPE`] event.
pub fn supported_versions(ev: &Event) -> Vec<u32> {
    if ev.event_type != SUPPORT_TYPE {
        return Vec::new();
    }
    control_attrs(ev, "TakProtocolSupport", "version").iter().filter_map(|v| v.parse().ok()).collect()
}

/// Version asked for by a [`REQUEST_TYPE`] event.
pub fn requested_version(ev: &Event) -> Option<u32> {
    if ev.event_type != REQUEST_TYPE {
        return None;
    }
    control_attrs(ev, "TakRequest", "version").first()?.parse().ok()
}

/// Whether a [`RESPONSE_TYPE`] event accepted the request.
pub fn response_status(ev: &Event) -> Option<bool> {
    if ev.event_type != RESPONSE_TYPE {
        return None;
    }
    control_attrs(ev, "TakResponse", "status").first()?.parse().ok()
}

fn control(uid: &str, event_type: &str, body: &str, now: DateTime<Utc>) -> Event {
    let point = Point { hae: 0.0, ce: 999_999.0, le: 999_999.0, ..Point::new(0.0, 0.0) };
    let mut ev = Event::new(uid, event_type, point);
    ev.time = now;
    ev.start = now;
    ev.stale = now + Duration::minutes(1);
    ev.detail.xml = format!("<TakControl>{body}</TakControl>");
    ev
}

/// `attr` of every `<TakControl><{name}>` child.
fn control_attrs(ev: &Event, name: &str, attr: &str) -> Vec<String> {
    let wrapped = format!("<detail>{}</detail>", ev.detail.xml);
    let Ok(doc) = Document::parse(&wrapped) else { return Vec::new() };
    doc.root_element()
        .children()
        .filter(|n| n.has_tag_name("TakControl"))
        .flat_map(|c| c.children())
        .filter(|n| n.has_tag_name(name))
        .filter_map(|n| n.attribute(attr).map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_round_trips_through_xml() {
        let now = Utc::now();
        let offer = Event::from_xml(&support("protouid", &[1], now).to_xml()).unwrap();
        assert_eq!(supported_versions(&offer), vec![1]);

        let ask = Event::from_xml(&request("protouid", 1, now).to_xml()).unwrap();
        assert_eq!(requested_version(&ask), Some(1));
        assert!(is_negotiation(&ask));

        let yes = Event::from_xml(&response("protouid", true, now).to_xml()).unwrap();
        assert_eq!(response_status(&yes), Some(true));
        assert_eq!(response_status(&response("p", false, now)), Some(false));
    }

    // The offer as printed in protocol.txt, single quotes and whitespace included.
    #[test]
    fn parses_spec_example() {
        let xml = r#"<event version='2.0' uid='protouid' type='t-x-takp-v' time='2026-11-21T10:00:00Z'
            start='2026-11-21T10:00:00Z' stale='2026-11-21T10:01:00Z' how='m-g'>
              <point lat='0.0' lon='0.0' hae='0.0' ce='999999' le='999999'/>
              <detail>
                <TakControl>
                  <TakProtocolSupport version="1"/>
                  <TakProtocolSupport version="2"/>
                </TakControl>
              </detail>
            </event>"#;
        assert_eq!(supported_versions(&Event::from_xml(xml).unwrap()), vec![1, 2]);
    }

    #[test]
    fn ignores_other_types() {
        let ev = Event::new("x", "a-f-G", Point::new(0.0, 0.0));
        assert_eq!(requested_version(&ev), None);
        assert!(supported_versions(&ev).is_empty());
    }
}
