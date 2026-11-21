use chrono::{TimeZone, Utc};
use cot::proto::{self, CotEvent};
use cot::{Contact, Error, Event, Point, Status, Takv};

fn sample() -> Event {
    let t = Utc.with_ymd_and_hms(2026, 11, 20, 15, 0, 0).unwrap();
    let mut ev =
        Event::new("SIM-1", "a-f-G-U-C", Point { hae: 12.0, ce: 5.0, le: 3.0, ..Point::new(38.9, -77.0) });
    ev.time = t;
    ev.start = t;
    ev.stale = t + Event::DEFAULT_STALE;
    ev.detail.contact = Some(Contact { callsign: "ALPHA-1".into(), endpoint: Some("*:-1:stcp".into()) });
    ev.detail.status = Some(Status { battery: 100 });
    ev.detail.takv =
        Some(Takv { device: None, platform: Some("tak-software".into()), os: None, version: None });
    ev.detail.xml = r#"<uid Droid="ALPHA-1"/>"#.into();
    ev
}

#[test]
fn proto_round_trip() {
    let ev = sample();
    let pb = ev.to_proto();
    assert_eq!(pb.send_time, ev.time.timestamp_millis() as u64);
    assert_eq!(pb.detail.as_ref().unwrap().xml_detail, ev.detail.xml);
    assert_eq!(Event::from_proto(pb).unwrap(), ev);
}

#[test]
fn proto_and_xml_agree() {
    let ev = sample();
    let via_xml = Event::from_xml(&ev.to_xml()).unwrap();
    assert_eq!(Event::from_proto(via_xml.to_proto()).unwrap(), ev);
}

#[test]
fn empty_detail_omitted() {
    let mut ev = sample();
    ev.detail = Default::default();
    assert_eq!(ev.to_proto().detail, None);
    assert!(!ev.to_xml().contains("<detail>"));
}

#[test]
fn rejects_xml_detail_that_escapes_detail() {
    let mut pb = sample().to_proto();
    pb.detail = Some(proto::Detail {
        xml_detail: r#"</detail></event><event uid="evil" type="a-h-G"><point lat="0" lon="0"/><detail>"#
            .into(),
        ..Default::default()
    });
    assert!(matches!(Event::from_proto(pb), Err(Error::InvalidXmlDetail(_))));
}

#[test]
fn rejects_missing_identity_and_bad_point() {
    let good = sample().to_proto();
    assert!(matches!(
        Event::from_proto(CotEvent { uid: String::new(), ..good.clone() }),
        Err(Error::MissingAttribute("uid"))
    ));
    assert!(matches!(
        Event::from_proto(CotEvent { lon: 200.0, ..good.clone() }),
        Err(Error::InvalidValue { name: "lon", .. })
    ));
    assert!(matches!(
        Event::from_proto(CotEvent { stale_time: u64::MAX, ..good }),
        Err(Error::InvalidValue { name: "staleTime", .. })
    ));
}
