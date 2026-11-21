use cot::{Contact, Error, Event, Group, Point, Status, Track};

// Shape of an ATAK-CIV self position report.
const ATAK_SA: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<event version="2.0" uid="ANDROID-589520ccfcd20f01" type="a-f-G-U-C" how="h-e"
       time="2026-11-20T15:04:05.123Z" start="2026-11-20T15:04:05.123Z" stale="2026-11-20T15:10:05.123Z">
  <point lat="38.8977" lon="-77.0365" hae="17.5" ce="9999999.0" le="9999999.0"/>
  <detail>
    <takv os="34" version="5.3.0.1 (abcd).1700000000-CIV" device="GOOGLE PIXEL 8" platform="ATAK-CIV"/>
    <contact endpoint="*:-1:stcp" callsign="VIPER"/>
    <uid Droid="VIPER"/>
    <precisionlocation altsrc="GPS" geopointsrc="GPS"/>
    <__group role="Team Member" name="Cyan"/>
    <status battery="88"/>
    <track course="271.5" speed="1.2"/>
  </detail>
</event>"#;

#[test]
fn parses_atak_sa() {
    let ev = Event::from_xml(ATAK_SA).unwrap();
    assert_eq!(ev.uid, "ANDROID-589520ccfcd20f01");
    assert_eq!(ev.event_type, "a-f-G-U-C");
    assert_eq!(ev.how, "h-e");
    assert_eq!(ev.time.to_rfc3339(), "2026-11-20T15:04:05.123+00:00");
    assert_eq!(ev.point.lat, 38.8977);
    assert_eq!(ev.point.ce, Point::UNKNOWN);

    let d = &ev.detail;
    assert_eq!(d.contact, Some(Contact { callsign: "VIPER".into(), endpoint: Some("*:-1:stcp".into()) }));
    assert_eq!(d.group, Some(Group { name: "Cyan".into(), role: "Team Member".into() }));
    assert_eq!(d.status, Some(Status { battery: 88 }));
    assert_eq!(d.track, Some(Track { speed: 1.2, course: 271.5 }));
    assert_eq!(d.takv.as_ref().unwrap().platform.as_deref(), Some("ATAK-CIV"));
    assert_eq!(d.xml, r#"<uid Droid="VIPER"/>"#);
}

#[test]
fn xml_round_trip() {
    let ev = Event::from_xml(ATAK_SA).unwrap();
    assert_eq!(Event::from_xml(&ev.to_xml()).unwrap(), ev);
}

#[test]
fn keeps_unpromotable_elements_raw() {
    // `phone` has no protobuf field, `readiness` isn't battery, and __group repeats.
    let xml = r#"<event version="2.0" uid="m1" type="b-m-p-s-p-i" time="2026-11-20T15:00:00Z"
        start="2026-11-20T15:00:00Z" stale="2026-11-21T15:00:00Z">
        <point lat="1" lon="2"/>
        <detail>
          <contact callsign="A" phone="555"/>
          <status readiness="true"/>
          <remarks>left &amp; right</remarks>
          <__group name="Red" role="Team Lead"/>
          <__group name="Blue" role="Medic"/>
        </detail></event>"#;
    let ev = Event::from_xml(xml).unwrap();
    assert_eq!(ev.detail.contact, None);
    assert_eq!(ev.detail.status, None);
    assert_eq!(ev.detail.group, None);
    assert_eq!(
        ev.detail.xml,
        concat!(
            r#"<contact callsign="A" phone="555"/>"#,
            r#"<status readiness="true"/>"#,
            r#"<remarks>left &amp; right</remarks>"#,
            r#"<__group name="Red" role="Team Lead"/>"#,
            r#"<__group name="Blue" role="Medic"/>"#,
        )
    );
    assert_eq!(Event::from_xml(&ev.to_xml()).unwrap(), ev);
}

#[test]
fn escapes_attributes() {
    let mut ev = Event::new("u<1>", "a-f-G", Point::new(0.0, 0.0));
    ev.detail.contact = Some(Contact { callsign: "R&D \"Team\"\n2".into(), endpoint: None });
    let back = Event::from_xml(&ev.to_xml()).unwrap();
    assert_eq!(back.uid, "u<1>");
    assert_eq!(back.detail.contact.unwrap().callsign, "R&D \"Team\"\n2");
}

#[test]
fn accepts_time_without_zone() {
    let xml = r#"<event uid="x" type="a-u-G" time="2026-11-20T15:00:00.5" start="2026-11-20T15:00:00"
        stale="2026-11-20T15:05:00"><point lat="0" lon="0"/></event>"#;
    let ev = Event::from_xml(xml).unwrap();
    assert_eq!(ev.time.timestamp_millis() % 1000, 500);
}

#[test]
fn rejects_bad_input() {
    let base = |point: &str| {
        format!(
            r#"<event uid="x" type="a-u-G" time="2026-11-20T15:00:00Z" start="2026-11-20T15:00:00Z"
            stale="2026-11-20T15:05:00Z">{point}</event>"#
        )
    };
    assert!(matches!(
        Event::from_xml(&base(r#"<point lat="91" lon="0"/>"#)),
        Err(Error::InvalidValue { name: "lat", .. })
    ));
    assert!(matches!(
        Event::from_xml(&base(r#"<point lat="NaN" lon="0"/>"#)),
        Err(Error::InvalidValue { name: "lat", .. })
    ));
    assert!(matches!(Event::from_xml(&base("")), Err(Error::MissingElement("point"))));
    assert!(matches!(Event::from_xml("<foo/>"), Err(Error::NotAnEvent(_))));
    assert!(matches!(
        Event::from_xml(r#"<event uid="x"><point lat="0" lon="0"/></event>"#),
        Err(Error::MissingAttribute("type"))
    ));
}

#[test]
fn rejects_dtd() {
    let xml = r#"<!DOCTYPE event [<!ENTITY a "aaaaaaaaaa">]><event uid="&a;"/>"#;
    assert!(matches!(Event::from_xml(xml), Err(Error::Xml(_))));
}
